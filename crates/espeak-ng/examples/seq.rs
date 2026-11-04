//! Runs a sequence of calls on one engine: `seq VOICE < texts.jsonl`, where each input line is a
//! JSON string; prints each call's clauses as a JSON array. `ESPEAK_DATA` selects the data.

use kokoro_espeak_ng::{Espeak, EspeakData, PHONEMES_IPA_TIE};
use std::io::BufRead;

fn main() {
    let voice = std::env::args().nth(1).expect("voice");
    let dir = std::env::var("ESPEAK_DATA")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/espeak-ng-data").to_string());
    let mut e = Espeak::new(EspeakData::load(dir).unwrap());
    e.set_voice_by_name(&voice).unwrap();
    for line in std::io::stdin().lock().lines() {
        let text: String = serde_json::from_str(&line.unwrap()).unwrap();
        let clauses = e.text_to_phonemes(&text, PHONEMES_IPA_TIE);
        println!("{}", serde_json::to_string(&clauses).unwrap());
    }
}
