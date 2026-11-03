# Intentional differences from the reference

The Rust port matches the deployed Python pipeline (`reference/`) except for the differences listed here. Each one says what changed, why, how many reference records it changes, and how it was checked (see "Matching the reference vs simplifying it" in `plan.md`).

| Area | Reference behaviour | Ours | Why | Records changed | Checked by |
|---|---|---|---|---|---|
| SSML (removed feature) | A speech request with `ssml: true` (and `allow_voice_tags`) has its `<speak>` document translated to inline tags (`ssml.py`) | Not supported: the new API has no `ssml` option, and a request that sets it is rejected | Brendan's decision (2026-11-02): Lion Reader doesn't use SSML, and the inline tags we keep (`[pause:Xs]`, `[voice:...]`, `[word](/phonemes/)`) cover the same ground. It also removes an XML parser from the untrusted-input path | 0 in corpus (SSML is a request option, not text) | n/a |
| Text: sentence splitting | A `[word](/phonemes/)` placeholder in the text (`xN`) whose index has more than 4,300 digits makes `int()` raise `ValueError`, failing the request | Treated as text that isn't a placeholder | The request shouldn't fail on hostile input | 0 in corpus | `crates/text` unit tests |
