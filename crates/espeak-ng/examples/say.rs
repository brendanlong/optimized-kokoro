//! Prints the phonemes for some text: `say VOICE TEXT...` (one clause per line).
//! `ESPEAK_DATA` selects the data directory; `ESPEAK_DUMP=1` prints each clause's phoneme list.

use kokoro_espeak_ng::{Espeak, EspeakData, PHONEMES_IPA_TIE};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::env::var("ESPEAK_DATA").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/espeak-ng-data").to_string());
    let data = EspeakData::load(dir).unwrap();
    let mut e = Espeak::new(data);
    e.set_voice_by_name(&args[1]).unwrap();
    let dump = std::env::var_os("ESPEAK_DUMP").is_some();
    let text = args[2..].join(" ");
    let clauses = e.translate_text(&text, PHONEMES_IPA_TIE, |e| {
        if dump {
            eprint!("{}", e.debug_phoneme_list());
        }
    });
    for clause in clauses {
        println!("{clause}");
    }
}
