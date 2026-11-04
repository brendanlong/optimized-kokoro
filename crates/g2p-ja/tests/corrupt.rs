//! A damaged dictionary file is an error from `Dictionary::load`, never a panic, an abort or a huge
//! allocation: every count is checked against what's left of the file. The mutations are the
//! PR #19 review's (`dicmut.py`): counts set to 0xffffffff, sizes past the file, truncations.

use kokoro_g2p_ja::Dictionary;
use std::path::PathBuf;

fn tmp(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("corrupt-dictionaries");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn rejected(name: &str, bytes: &[u8]) {
    let path = tmp(name, bytes);
    assert!(Dictionary::load(&path).is_err(), "{name} loaded");
}

#[test]
fn tiny_files_are_rejected() {
    rejected("empty", b"");
    rejected("magic_only", b"KOJADIC1");
    let mut v = b"KOJADIC1".to_vec();
    v.extend(u32::MAX.to_le_bytes());
    v.resize(100, 0);
    rejected("ver_len", &v);
    rejected("other", b"not a dictionary at all, but long enough to have a header.....");
}

fn u32_at(b: &[u8], i: usize) -> usize {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as usize
}

#[test]
fn damaged_copies_of_the_real_dictionary_are_rejected() {
    let Some(path) = kokoro_g2p_ja::default_dictionary_path().filter(|p| p.exists()) else {
        return kokoro_g2p_ja::skip_without_dictionary("no dictionary at $KOKORO_UNIDIC");
    };
    use std::io::Read;
    let mut head = vec![0; 64 << 20];
    let n = std::fs::File::open(&path).unwrap().read(&mut head).unwrap();
    head.truncate(n);
    // Offsets of the counts, walking the header as `Dictionary::load` does.
    let mut p = 8;
    p += 4 + u32_at(&head, p);
    let (off_l, off_r) = (p, p + 4);
    p += 8;
    p += 4 + 4 * u32_at(&head, p);
    let off_cat = p;
    let cats = u32_at(&head, p);
    p += 4;
    let mut first_unk_n = None;
    for _ in 0..cats {
        p += 4 + u32_at(&head, p);
        first_unk_n.get_or_insert(p);
        p += 4 + 10 * u32_at(&head, p);
    }
    let off_da = p;
    let patched = |off: usize, val: u32, upto: usize| {
        let mut b = head[..upto.min(head.len())].to_vec();
        b[off..off + 4].copy_from_slice(&val.to_le_bytes());
        b
    };
    let mut lr = patched(off_l, u32::MAX, off_da + 16);
    lr[off_r..off_r + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    rejected("lr_max", &lr);
    let mut big = patched(off_l, 0x10000, off_da + 16);
    big[off_r..off_r + 4].copy_from_slice(&0x10000u32.to_le_bytes());
    rejected("lr_big", &big);
    rejected("cats_max", &patched(off_cat, u32::MAX, off_cat + 100));
    rejected("unk_n_max", &patched(first_unk_n.unwrap(), u32::MAX, first_unk_n.unwrap() + 100));
    rejected("da_n_max", &patched(off_da, u32::MAX, off_da + 100));
    rejected("ver_len_real", &patched(8, u32::MAX, 100));
    for frac in [0.01, 0.1, 0.3, 0.5, 0.999] {
        rejected(&format!("trunc{frac}"), &head[..(head.len() as f64 * frac) as usize]);
    }
}
