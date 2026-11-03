//! Prints the phonemes for some text: `say VOICE TEXT...` (one clause per line).
//! `ESPEAK_DATA` selects the data directory.

use kokoro_espeak_ng::{Espeak, EspeakData, PHONEMES_IPA_TIE};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::env::var("ESPEAK_DATA").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/espeak-ng-data").to_string());
    let data = EspeakData::load(dir).unwrap();
    let mut e = Espeak::new(data);
    e.set_voice_by_name(&args[1]).unwrap();
    for clause in e.text_to_phonemes(&args[2..].join(" "), PHONEMES_IPA_TIE) {
        println!("{clause}");
    }
}
