//! Without UniDic: pipelines still load, English (and Japanese runs in it, as "Japanese text")
//! still reads, and only the Japanese pipeline's sentences fail. The dictionary is loaded once per
//! process, so the check runs in a child process with `KOKORO_UNIDIC` pointing nowhere.

use kokoro_pipeline::{Error, Lang, NormalizationOptions, Pipeline};

const CHILD: &str = "KOKORO_NO_DICTIONARY_CHILD";

#[test]
fn without_the_dictionary() {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("KOKORO_UNIDIC", "/nonexistent/unidic.dic")
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn child() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let options = NormalizationOptions::default();
    let english = Pipeline::new(Lang::AmericanEnglish).unwrap();
    let chunks: Vec<_> = english.chunks("She said こんにちは.", &options).map(Result::unwrap).collect();
    assert_eq!((chunks[0].phonemes.as_str(), chunks[0].unread), ("ʃˌi sˈɛd ʤˌæpənˈiz tˈɛkst.", 1));
    let japanese = Pipeline::new(Lang::Japanese).unwrap();
    let chunks: Vec<_> = japanese.chunks("東京へ行きます。大阪へ。", &options).collect();
    assert_eq!(chunks.len(), 2);
    for chunk in chunks {
        match chunk {
            Err(Error::Dictionary(message)) => assert!(message.contains("fetch_unidic.sh"), "{message}"),
            other => panic!("{other:?}"),
        }
    }
}
