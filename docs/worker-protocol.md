# Worker protocol v1 (front ⇄ GPU worker)

One Unix stream socket. The **worker listens** on a path inside a directory bind-mounted into its sandbox; the front connects. The worker serves one connection at a time; a second connection waits in the listen backlog until the first closes. The front multiplexes all requests over its one connection. All integers little-endian.

Every frame: `u32 body_len` then `body_len` bytes of body; body starts with `u8 type`. Max body length: 64 KiB from front→worker, 8 MiB from worker→front. A reader that sees a length of 0 or over its cap, an unknown type, a short or overlong body, or any invalid field closes the connection (no error frames for protocol violations; the peer is buggy or hostile). Check the length against the cap before allocating.

## Worker → front
- `0x01 Hello` (first frame after connect): `[4]u8 magic = "KOKW"`, `u16 version = 1`, `u16 vocab_size` (178 for Kokoro-82M; 0 is the boundary/pad id the worker adds itself), `u16 max_phonemes = 510`, `u16 max_batch`, `u32 sample_rate = 24000`, `u16 n_voices`, then per voice `u8 name_len` + ASCII name (`[a-z0-9_]+`, 1..=64 bytes). Voice index = position in this list. The fixed part is 19 bytes including the type byte.
- `0x02 Audio`: `u64 request_id`, `u32 n_samples`, `n_samples × i16` PCM (mono, sample_rate). Exactly one per completed Synthesize (a chunk is synthesized whole).
- `0x03 Durations`: `u64 request_id`, `u16 n`, `n × u16` predicted frame durations, one per input id plus the two boundary ids (for word timestamps). One frame is 600 samples (25 ms). Sent immediately before the request's Audio, with nothing in between.

Size rules for a request's two frames, which the front checks:
- Durations has `n = n_ids + 2`.
- Each duration is 1..=`ceil(50 / speed)` (the model sums 50 sigmoids per id, divides by the request's `speed` as f32, and rounds); at most 200.
- Audio has exactly `n_samples = 600 × sum(durations)`. `sum(durations)` is at most the worker's frame cap (4096 by default, about 102 s; see `Failed` code 3), so Audio bodies stay under the 8 MiB cap.
- `0x04 Failed`: `u64 request_id`, `u8 code` (1 = cancelled, 2 = internal error, 3 = too long: the predicted output exceeds the worker's frame cap, 4096 frames or about 102 s by default; retrying won't help, split the chunk or raise the speed). Ends the request.

## Front → worker
- `0x10 Synthesize`: `u64 request_id`, `u8 priority` (0 = a listener is waiting on this chunk now; 1 = prefetch), `f32 speed` (finite, 0.25..=4.0), `u8 n_voices` (1..=8), then `n_voices × (u16 voice_index, f32 weight)` (weights finite, > 0; the worker normalizes them to sum 1 and mixes the voice packs), `u16 n_ids` (1..=max_phonemes), `u16 style_len` (1..=max_phonemes; the voice-pack row is `style_len - 1`), `n_ids × u8 id`. Each id must be one of the values in the model's `config.json` `vocab` map; that is a subset of `1..vocab_size-1` with gaps (114 symbols for Kokoro-82M), and the front builds ids from the same map.
- `0x11 Cancel`: `u64 request_id`. If still queued, the worker drops it and replies `Failed(cancelled)`. If already running, or unknown (e.g. already answered), the worker ignores it and sends nothing; a running request finishes normally.

## Semantics
- The worker **re-validates every field** and does not trust the front. The front doesn't trust the worker either: it checks frame lengths, types, request ids it actually has outstanding, and the size rules above.
- `request_id` is unique among the connection's outstanding requests (sent, and not yet answered by Audio or Failed). Reusing an outstanding id is a violation. At most 1024 requests may be outstanding; one more is a violation.
- Style vector: mixed voice pack row `style_len - 1` (KPipeline's `pack[len(ps)-1]`). For a normal chunk the front sends `style_len = n_ids`. For a piece of an overlap-stitched long clause it sends the whole clause's length, as Kokoro-FastAPI does, so every piece gets the clause's style row.
- Voice weights stay positive: voice subtraction isn't supported. There is no Reprioritize frame; to change a queued request's priority, Cancel it and send it again.
- Scheduling is the worker's: whenever the GPU is free, take queued requests by (priority, arrival), up to `max_batch`, and never wait to fill a batch. The batch's text side (durations) runs together; then its decoder runs in groups of similar output length, most urgent group first, and each group's responses are sent as soon as it finishes. Responses may come back in any order; the front reorders per stream.
- Batching doesn't change the result beyond run-to-run noise: padding is masked out (see `worker/README.md`).
- No state carries between requests except loaded weights and voices. Closing the connection drops every queued request; requests already on the GPU finish and their results are discarded.

## Changes from the first draft

Made while implementing the worker (`worker/`):
1. Ids must be in the vocab map, not just in `1..vocab_size-1`: the range has gaps, and ids in the gaps are rows of the embedding the model never trained.
2. Added `Failed` code 3 (too long) and a cap on output length, because 510 ids at speed 0.25 can ask for minutes of audio and several GB of activations.
3. Added a worker→front body cap (8 MiB), so the front can check Audio lengths before allocating.
4. Added `u16 style_len` after `n_ids` (amendment from the server side): the style row for stitched pieces.
5. Defined duplicate ids, the outstanding limit, Cancel of running or unknown ids, the Hello's fixed size, the Durations-then-Audio adjacency, `n_samples = 600 × sum(durations)`, and what a closed connection does to queued work.
