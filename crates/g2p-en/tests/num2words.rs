//! Compares with num2words 0.5.14's output, recorded by tools/gen_num2words.py.
//! `NUM2WORDS_TSV` points at another recording in the same format.

use kokoro_g2p_en::num2words;

#[test]
fn matches_num2words() {
    let path = std::env::var("NUM2WORDS_TSV")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/num2words.tsv").to_string());
    let data = std::fs::read_to_string(&path).unwrap();
    let (mut total, mut failures) = (0, Vec::new());
    for line in data.lines() {
        let mut parts = line.splitn(3, '\t');
        let (kind, input, expected) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
        let got = match kind {
            "cardinal" => num2words::cardinal(input),
            "ordinal" => num2words::ordinal(input),
            "year" => Ok(num2words::year(input.parse().unwrap())),
            "float" => num2words::float(input),
            _ => panic!("unknown kind {kind}"),
        };
        let got = got.unwrap_or_else(|_| "ERR".to_string());
        total += 1;
        if got != expected {
            failures.push(format!("{kind} {input}\n  expected {expected}\n  got      {got}"));
        }
    }
    assert!(total > 0);
    assert!(
        failures.is_empty(),
        "{} of {total} differ:\n{}",
        failures.len(),
        failures[..failures.len().min(20)].join("\n")
    );
}
