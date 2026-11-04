# Worker protocol v3 (front ⇄ GPU worker)

This file is the specification. The worker (`worker/kokoro_worker/protocol.py`, `scheduler.py`, `server.py`) and the front (`crates/server/src/protocol.rs`, `worker.rs`) both implement it; change it here first.

One Unix stream socket. The **worker listens** on a path inside a directory bind-mounted into its sandbox; the front connects. The worker serves one connection at a time; a second connection waits in the listen backlog until the first closes. The front multiplexes all requests over its one connection. All integers little-endian, and there are no floats on the wire.

Every frame: `u32 body_len` then `body_len` bytes of body; body starts with `u8 type`. A reader that sees a length outside its bound (below), an unknown type, a short or overlong body, or any invalid field closes the connection (no error frames for protocol violations; the peer is buggy or hostile). Each length prefix is checked against its bound before anything is allocated:
- front→worker: 1..=`13 + max_phonemes` bytes (523);
- worker→front, by type: Hello at most 25 + 65,535 × 65 bytes, Failed exactly 4, and Result exactly the size its durations imply, which is at most `5 + 2 × (n_ids + 2) + 2 × chunk cap` for its slot's request (see the size rules).

## Slots

Hello announces `slots`, the most requests the worker accepts at once. A request lives in a **slot**, an index `0..slots-1`, from the Synthesize that names it until its terminal reply (Result or Failed). The front owns a fixed table of slots: it sends a Synthesize only for a slot that is free in its table, and frees the slot only when that slot's terminal reply arrives. The worker frees a slot as it sends the terminal reply. So:
- a Synthesize for a slot that is busy (or ≥ `slots`) is a violation;
- a Result or Failed for a slot the front considers free is a violation;
- there are no request ids, and no count of outstanding requests beyond the slots themselves.

The front may use fewer slots than the worker announces (only indices below its own limit).

## Worker → front
- `0x01 Hello`, the first frame after connect, sent at once: the worker compiles and warms up before it listens. `[4]u8 magic = "KOKW"`, `u16 version = 3`, `u16 vocab_size` (178 for Kokoro-82M; 0 is the boundary/pad id the worker adds itself), `u16 max_phonemes = 510`, `u16 max_batch` (≥ 1), `u16 slots` (≥ 1), `u32 sample_rate = 24000`, `u32 max_chunk_samples` (≥ 1; the most audio one request may produce at speed 1, the worker's default is 1,440,000 = 60 s; the front refuses a Hello above its configured ceiling, `worker_max_chunk_samples`, default 2,880,000 = 120 s), `u16 n_voices` (≥ 1), then per voice `u8 name_len` + ASCII name (`[a-z0-9_]`, 1..=64 bytes, names distinct; the worker refuses to start with any other voice name). Voice index = position in this list. The fixed part is 25 bytes including the type byte.
- `0x02 Result`: `u16 slot`, `u16 n`, `n × u16` predicted durations (one per input id plus the two boundary ids, for word timestamps), then exactly `600 × Σ durations` `i16` PCM samples (mono, sample_rate), to the end of the frame. One duration unit is 600 samples (25 ms). Ends the request. A chunk is synthesized whole, in one frame; at the default cap and the slowest speed that's at most 23 MB.
- `0x03 Failed`: `u16 slot`, `u8 code`. Ends the request. Codes:
  - 1 = cancelled, only in reply to a Cancel for that slot;
  - 2 = internal error;
  - 3 = too long: the predicted audio exceeds the request's chunk cap. The worker predicts durations first and doesn't run the decoder for such a request. Retrying the same chunk won't help; split it or raise the speed.

Size rules for a Result, which the front checks:
- `n = n_ids + 2`.
- Each duration is 1..=`ceil(5000 / speed)` (speed in hundredths, so this is `ceil(50 / speed)` in exact integer arithmetic), never 0: the model sums 50 sigmoids per id, divides by the speed and rounds. The worker also clamps to this range.
- The sample count is exactly `600 × Σ durations`, and that is at most the request's **chunk cap**, `ceil(max_chunk_samples × 100 / speed)` in integers. The worker replies `Failed` code 3 instead of exceeding it; durations implying more are a violation, detected before any sample is read.
- The front keeps no audio byte budget (Brendan's decision, 2026-11-02). Its memory is bounded by counts times the cap: each admitted speech request holds at most `lookahead + 2` chunks, so the worst case is about `max_concurrent_requests × (lookahead + 2) × 2 × chunk cap` bytes, plus one Result being read (`Config::audio_bound` in `crates/server/src/config.rs`, checked by a test with clients that never read). With the defaults (16 requests, lookahead 3, the front's 120 s ceiling) that is about 467 MB at speed 1 and 1.85 GB if every request runs at speed 0.25 with chunks at the cap; real sentences are around 5 s (240 KB). Slots don't enter this bound: a slot is freed when its audio arrives, and the audio then counts against its request. Slots bound the rest: the front's table and outgoing queue (at most three frames per slot: a stale Cancel for its previous request, a Synthesize and a Cancel, 541 bytes with their length prefixes) and the worker's queue (at most `slots` Synthesize frames of at most 523 bytes).

Choosing the cap: across 63 chunks of 450–510 phonemes (consecutive sentences of *The Time Machine*) in 8 voices at speed 1, the longest predicted audio was 43.9 s (af_nicole, 508 phonemes) and the median 27.4 s (`worker/bench/max_duration.py`). The worker's 60 s default leaves 1.4× headroom for real text while bounding what a pathological chunk can cost: the model's theoretical maximum is 512 × 50 × 600 samples (31 MB) at speed 1. It was 120 s until 2026-11-04. That allowed 480 s (19,200 frames) at speed 0.25, which ran out of memory on an 8 GB card, whereas 240 s peaks at 3.9 GB (`worker/README.md`).

## Front → worker
- `0x10 Synthesize`: `u16 slot` (free, < slots), `u8 priority` (the request's priority, 0..=255, higher more urgent; every value is valid), `u8 waiting` (1 = a listener is waiting on this chunk now, 0 = prefetch; anything else is a violation), `u16 speed` (hundredths: 25..=400, i.e. 0.25×..4×), `u16 voice_index` (< n_voices), `u16 n_ids` (1..=max_phonemes), `u16 style_len` (1..=max_phonemes), `n_ids × u8 id` (each 1..vocab_size-1). The fixed part is 13 bytes.
- `0x11 Cancel`: `u16 slot` (< slots). If that slot's request is still queued, the worker drops it and replies `Failed(cancelled)`. If it is running, the worker ignores the Cancel and the request finishes normally with its Result. If the slot is free, the worker ignores it: its reply crossed the Cancel. Either way the slot stays busy until its terminal reply.

## Semantics
- The worker **re-validates every field** and does not trust the front. Once a frame's first byte arrives, the rest must arrive within 10 s, or the worker closes the connection (a front may stay idle between frames as long as it likes); a worker send that can't complete in 10 s closes it too. A front that disconnects at any point, even before Hello, only ends its own connection. The front doesn't trust the worker either: every frame after Hello must name a busy slot, and the size rules hold.
- Style vector: voice pack row `style_len - 1` (KPipeline's `pack[len(ps)-1]`). For a normal chunk the front sends `style_len = n_ids`. For a piece of an overlap-stitched long clause it sends the whole clause's length (capped at 510), as Kokoro-FastAPI does, so every piece gets the clause's style row.
- There is no Reprioritize frame; to change a queued request's priority, Cancel it, wait for its reply, and send it again.
- Ids in `1..vocab_size-1` that aren't in the model's `config.json` vocab (Kokoro-82M uses 114 of the 177) are accepted: they are untrained embedding rows, harmless, and the front never sends them.
- Scheduling is the worker's: whenever the GPU is free, take queued requests most urgent first, up to `max_batch`, and never wait to fill a batch. Most urgent means the highest `priority`, then, at equal priority, `waiting` before prefetch, then the earliest arrival: lexicographic on `(priority, waiting)` descending, then arrival ascending. So a more urgent request is always taken before a less urgent one, whatever its waiting flag, and within a request (whose chunks share one priority) the chunk a listener is waiting on goes first. The batch's text side (durations) runs together; then its decoder runs in groups of similar output length, most urgent group first, and each group's responses are sent as soon as it finishes. Responses may come back in any order; the front reorders per stream.
- Padding doesn't change the result: it's masked out (see `worker/README.md`). In fp16, batch size itself changes the numerics slightly, and about one sentence in 16 gets one duration a frame different from batch 1.
- No state carries between requests except loaded weights and voices. Closing the connection drops every queued request and frees every slot; requests already on the GPU finish and their results are discarded.

## Changes from v2

From issue #26 (request priorities and load shedding), 2026-11-04. Brendan's decisions there: request priority is a `u8`, 0–255, carried as a `u8` on the wire; higher priority is always served before lower. Ours:
1. **`u8 priority` is the request's priority**, higher more urgent (v2: 0 = waiting, 1 = prefetch, lower more urgent, anything above 1 a violation). Higher-is-more-urgent matches the HTTP field and reads naturally as a level.
2. **`u8 waiting`**, a new byte after `priority`, carries v2's waiting-versus-prefetch distinction, which now ranks only within a priority. One byte can't carry both 256 levels and the flag, and a combined formula would cost levels; a separate byte keeps both exact. The Synthesize fixed part is 13 bytes and the front→worker bound 523.
3. **Version 3**, so a front and worker of different versions refuse each other's Hello instead of misreading frames.

The front's latency estimate and load shedding (`crates/server/src/load.rs`) need nothing more from the worker: it learns the render cost from when Results arrive.

## Changes in v2 from v1

Brendan's decisions (2026-11-03):
1. **Slots instead of request ids.** Hello announces `u16 slots`; requests are named by slot, and a slot is busy until its terminal reply. Replaces v1's `u64 request_id`, its uniqueness rule, the 1,024 outstanding limit and the id maps on both sides.
2. **Cancel is kept, keyed by slot**, with the same meaning as before.
3. **Speed is `u16` hundredths** (25..=400) instead of an `f32`, so there is no float to validate, and the duration and chunk-cap bounds are exact integer arithmetic (v1's front allowed one more frame per id for float rounding).
4. **One `Result` frame** replaces Durations + Audio (and with it v1's rule that Audio must directly follow its Durations).
5. **A single `u16 voice_index`**: no voice blends, so no weights to validate.

Also: the front→worker length bound is now the exact maximum, 522 bytes (v1: 64 KiB), and Failed is type `0x03` (v1 had Audio `0x02`, Durations `0x03`, Failed `0x04`).

## Changes in v1 from the first draft

1. `Failed` code 3 (too long), and `u32 max_chunk_samples` in Hello: a practical cap on one chunk's audio that both sides enforce, replacing the front's per-chunk reservation of the theoretical worst case and its audio byte budget; memory is bounded by counts instead. Brendan's decision (2026-11-02).
2. `u16 style_len` after `n_ids` (from the server side): the style row for stitched pieces.
3. Per-type bounds on worker→front lengths, and the size rules, as the server checks them.
4. From the PR #11 review: zero durations are violations; the front has a ceiling on `max_chunk_samples`; the worker has a 10 s deadline for a frame once started.
5. Defined Cancel of running requests, the Hello's fixed size, Hello before any warmup, and what a closed connection does to queued work.
