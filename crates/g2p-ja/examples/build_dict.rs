//! Converts a MeCab dictionary directory (UniDic's `dicdir`) into the file the crate maps:
//! `cargo run --release -p kokoro-g2p-ja --example build_dict -- DICDIR OUT [ORDER]`
//!
//! ORDER is the context id order (default `data/context_order.txt`; `none` keeps UniDic's ids,
//! which `examples/context_order.rs` needs to count them).
use kokoro_g2p_ja::dict::ContextOrder;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: build_dict DICDIR OUT [ORDER|none]");
        std::process::exit(2);
    }
    let order = match args.get(3).map(String::as_str) {
        Some("none") => ContextOrder::default(),
        path => {
            let path = path.map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("data/context_order.txt"), Into::into);
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            ContextOrder::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        }
    };
    if let Err(e) = kokoro_g2p_ja::dict::build(Path::new(&args[1]), kokoro_g2p_ja::UNIDIC_VERSION, &order, Path::new(&args[2])) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
