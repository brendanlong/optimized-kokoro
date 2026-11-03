//! Time grows linearly with the number of clauses in a text: 4× the clauses take well under 16×
//! the time, which is what work per clause proportional to the rest of the text would give.

use kokoro_espeak_ng::{default_data_dir, Espeak, EspeakData, PHONEMES_IPA_TIE};
use std::time::{Duration, Instant};

fn best_of(runs: usize, mut f: impl FnMut()) -> Duration {
    (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
fn many_clauses_take_linear_time() {
    let data = EspeakData::load(default_data_dir()).unwrap();
    let n = if cfg!(debug_assertions) { 4_000 } else { 20_000 };
    for (voice, unit) in [
        ("gmw/en-US", "a, "),
        ("gmw/en-US", "Yes. "),
        // read as one abbreviation after another: long clauses rather than many
        ("gmw/en-US", "a. "),
        ("gmw/en-US", "a\n\n"),
        ("roa/es", "a。"),
        ("roa/fr", "1 234, "),
    ] {
        let mut e = Espeak::new(data.clone());
        e.set_voice_by_name(voice).unwrap();
        let mut time = |count: usize| {
            let text = unit.repeat(count);
            best_of(2, || {
                assert!(!e.text_to_phonemes(&text, PHONEMES_IPA_TIE).is_empty());
            })
        };
        let (t1, t4) = (time(n), time(4 * n));
        let ratio = t4.as_secs_f64() / t1.as_secs_f64();
        assert!(ratio < 8.0, "{voice} {unit:?}: {n} clauses {t1:?}, {} clauses {t4:?} ({ratio:.1}×)", 4 * n);
    }
}
