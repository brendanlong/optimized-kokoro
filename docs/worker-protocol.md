# Worker protocol v1 (front ⇄ GPU worker)

This file is the specification. The worker (`worker/kokoro_worker/protocol.py`, `server.py`) and the front (`crates/server/src/protocol.rs`, `worker.rs`) both implement it; change it here first.

One Unix stream socket. The **worker listens** on a path inside a directory bind-mounted into its sandbox; the front connects. The worker serves one connection at a time; a second connection waits in the listen backlog until the first closes. The front multiplexes all requests over its one connection. All integers little-endian.

Every frame: `u32 body_len` then `body_len` bytes of body; body starts with `u8 type`. A reader that sees a length outside its bound (below), an unknown type, a short or overlong body, or any invalid field closes the connection (no error frames for protocol violations; the peer is buggy or hostile). Each length prefix is checked against its bound before anything is allocated:
- front→worker: 1..=64 KiB;
- worker→front, by type: Hello at most 23 + 65,535 × 65 bytes, Durations at most 1,035, Failed exactly 10, and Audio exactly the size its request's Durations imply, which is at most the request's chunk cap (see the size rules).

## Worker → front
- `0x01 Hello`, the first frame after connect, sent at once: the worker compiles and warms up before it listens. `[4]u8 magic = "KOKW"`, `u16 version = 1`, `u16 vocab_size` (178 for Kokoro-82M; 0 is the boundary/pad id the worker adds itself), `u16 max_phonemes = 510`, `u16 max_batch` (≥ 1), `u32 sample_rate = 24000`, `u32 max_chunk_samples` (≥ 1; the most audio one request may produce at speed 1, default 2,880,000 = 120 s; the front refuses a Hello above its configured ceiling, `worker_max_chunk_samples`, default also 2,880,000), `u16 n_voices` (≥ 1), then per voice `u8 name_len` + ASCII name (`[a-z0-9_]`, 1..=64 bytes, names distinct; the worker refuses to start with any other voice name). Voice index = position in this list. The fixed part is 23 bytes including the type byte.
- `0x03 Durations`: `u64 request_id`, `u16 n`, `n × u16` predicted durations, one per input id plus the two boundary ids (for word timestamps). One duration unit is 600 samples (25 ms). Sent immediately before the request's Audio: the next frame after a Durations is that request's Audio, and anything else is a violation.
- `0x02 Audio`: `u64 request_id`, `u32 n_samples`, `n_samples × i16` PCM (mono, sample_rate). Exactly one per completed Synthesize (a chunk is synthesized whole, in one frame; at the default cap and the slowest speed that's at most 23 MB).
- `0x04 Failed`: `u64 request_id`, `u8 code`. Ends the request. Codes:
  - 1 = cancelled, only in reply to a Cancel for that request;
  - 2 = internal error;
  - 3 = too long: the predicted audio exceeds the request's chunk cap. The worker predicts durations first and doesn't run the decoder for such a request. Retrying the same chunk won't help; split it or raise the speed.

Size rules for a request's frames, which the front checks:
- Durations has `n = n_ids + 2`.
- Each duration is 1..=`ceil(50 / speed)`, never 0: the model sums 50 sigmoids per id, divides by the request's f32 `speed` and rounds. The front allows one more for float rounding.
- Audio has exactly `n_samples = 600 × sum(durations)`, and that is at most the request's **chunk cap**, `ceil(max_chunk_samples / speed)`, computed in f64 from the f32 speed. The worker replies `Failed` code 3 instead of exceeding it; Durations implying more are a violation.
- The front keeps no audio byte budget (Brendan's decision, 2026-11-02). Its memory is bounded by counts times the cap: each admitted speech request holds at most `lookahead + 2` chunks, so the worst case is about `max_concurrent_requests × (lookahead + 2) × 2 × chunk cap` bytes (`Config::audio_bound` in `crates/server/src/config.rs`, checked by a test with clients that never read). With the defaults (16 requests, lookahead 3, 120 s cap) that is about 467 MB at speed 1 and 1.85 GB if every request runs at speed 0.25 with chunks at the cap; real sentences are around 5 s (240 KB).

Choosing the cap: across 63 chunks of 450–510 phonemes (consecutive sentences of *The Time Machine*) in 8 voices at speed 1, the longest predicted audio was 43.9 s (af_nicole, 508 phonemes) and the median 27.4 s (`worker/bench/max_duration.py`). The 120 s default leaves 2.7× headroom for real text while bounding what a pathological chunk can cost: the model's theoretical maximum is 512 × 51 × 600 samples (31 MB) at speed 1.

## Front → worker
- `0x10 Synthesize`: `u64 request_id`, `u8 priority` (0 = a listener is waiting on this chunk now; 1 = prefetch), `f32 speed` (finite, 0.25..=4.0), `u8 n_voices` (1..=8), then `n_voices × (u16 voice_index, f32 weight)` (weights finite, > 0; the worker normalizes them to sum 1 and mixes the voice packs), `u16 n_ids` (1..=max_phonemes), `u16 style_len` (1..=max_phonemes), `n_ids × u8 id` (each 1..vocab_size-1).
- `0x11 Cancel`: `u64 request_id`. If still queued, the worker drops it and replies `Failed(cancelled)`. If already running, or unknown (e.g. already answered), the worker ignores it and sends nothing; a running request finishes normally with Durations and Audio.

## Semantics
- The worker **re-validates every field** and does not trust the front. Once a frame's first byte arrives, the rest must arrive within 10 s, or the worker closes the connection (a front may stay idle between frames as long as it likes); a worker send that can't complete in 10 s closes it too. A front that disconnects at any point, even before Hello, only ends its own connection. The front doesn't trust the worker either: every frame after Hello must name a request it sent that hasn't ended, and the size rules hold.
- `request_id` is unique among the connection's outstanding requests (sent, and not yet ended by Audio or Failed). Reusing an outstanding id is a violation. At most 1024 requests may be outstanding; one more is a violation.
- Style vector: mixed voice pack row `style_len - 1` (KPipeline's `pack[len(ps)-1]`). For a normal chunk the front sends `style_len = n_ids`. For a piece of an overlap-stitched long clause it sends the whole clause's length (capped at 510), as Kokoro-FastAPI does, so every piece gets the clause's style row.
- Voice weights stay positive: voice subtraction isn't supported. There is no Reprioritize frame; to change a queued request's priority, Cancel it and send it again.
- Ids in `1..vocab_size-1` that aren't in the model's `config.json` vocab (Kokoro-82M uses 114 of the 177) are accepted: they are untrained embedding rows, harmless, and the front never sends them.
- Scheduling is the worker's: whenever the GPU is free, take queued requests by (priority, arrival), up to `max_batch`, and never wait to fill a batch. The batch's text side (durations) runs together; then its decoder runs in groups of similar output length, most urgent group first, and each group's responses are sent as soon as it finishes. Responses may come back in any order; the front reorders per stream.
- Padding doesn't change the result: it's masked out (see `worker/README.md`). In fp16, batch size itself changes the numerics slightly, and about one sentence in 16 gets one duration a frame different from batch 1.
- No state carries between requests except loaded weights and voices. Closing the connection drops every queued request; requests already on the GPU finish and their results are discarded.

## Changes from the first draft

1. `Failed` code 3 (too long), and `u32 max_chunk_samples` in Hello: a practical cap on one chunk's audio that both sides enforce, replacing the front's per-chunk reservation of the theoretical worst case (31 MB per chunk at speed 1, 123 MB at 0.25) and its audio byte budget; memory is bounded by counts instead. Brendan's decision (2026-11-02).
2. `u16 style_len` after `n_ids` (from the server side): the style row for stitched pieces.
3. Per-type bounds on worker→front lengths, and the size rules (`n = n_ids + 2`, durations ≤ `ceil(50 / speed)`, `n_samples = 600 × sum(durations)`), as the server checks them.
4. From the PR #11 review: zero durations and anything between a Durations and its Audio are violations; the front has a ceiling on `max_chunk_samples`; the worker has a 10 s deadline for a frame once started.
5. Duplicate outstanding ids are a violation, and at most 1024 may be outstanding (the server caps its own limit there). Defined Cancel of running or unknown ids, the Hello's fixed size, Hello before any warmup, the Durations-then-Audio adjacency, and what a closed connection does to queued work.
