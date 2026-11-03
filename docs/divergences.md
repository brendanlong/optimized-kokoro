# Intentional differences from the reference

The Rust port matches the deployed Python pipeline (`reference/`) except for the differences listed here. Each one says what changed, why, how many reference records it changes, and how it was checked (see "Matching the reference vs simplifying it" in `plan.md`).

| Area | Reference behaviour | Ours | Why | Records changed | Checked by |
|---|---|---|---|---|---|
| misaki English | `TypeError` when a punctuation-tagged token starts with `İ` | Returns phonemes | The request shouldn't fail | 0 in corpus (30 of 4,000 random strings) | PR #5 fuzzing |
| misaki English | British `[3ing](-1)` raises `AttributeError` and `[3ing](0.5)` raises `TypeError`: the number reading gets a stress feature, but `_ing` returned `None` | Returns phonemes (the `-ing` form is read in pieces) | The request shouldn't fail | 0 in corpus | PR #5 review (Sol), link fuzzing |
| misaki English | In a run of subtokens without whitespace, tries every span as one word (cubic in the run's length: 1.9 KB of `qx-qx-…` took 8.4 s) | Tries spans of at most 16 subtokens. No lexicon key has more than 13 subtokens, so only dotted abbreviations longer than 8 letters, and numbers whose repeated separators split them into more than 16 subtokens, read differently. `a.b.c.d.e.f.g.h.i` keeps its letter names but gets different stress. | Linear time on adversarial input | 0 of 50,328 recorded calls (corpus, direct misaki cases, fuzz) | `crates/g2p-en/tests/parity.rs`, `tests/scaling.rs` |
