//! `spacy.training.Alignment.from_strings(a, b).y2x.data`: for each token of `b` in order, the sorted
//! indices of the `a` tokens it overlaps, concatenated. misaki indexes spaCy's tokens by position in this
//! flat array (not by token), so the port keeps it flat.

use crate::text::{is_space, lower};

/// `"".join(tokens).lower()` (lowered as a whole, so final sigma sees its neighbours) and each character's
/// token, counted from each token's own lowered length as spaCy does. `None` if those lengths disagree.
fn lowered(tokens: &[impl AsRef<str>]) -> Option<(Vec<char>, Vec<usize>)> {
    let mut owner = Vec::new();
    let mut joined = String::new();
    for (i, t) in tokens.iter().enumerate() {
        owner.extend(std::iter::repeat_n(i, lower(t.as_ref()).chars().count()));
        joined.push_str(t.as_ref());
    }
    let chars: Vec<char> = lower(&joined).chars().collect();
    (chars.len() == owner.len()).then_some((chars, owner))
}

fn without_space(chars: &[char]) -> impl Iterator<Item = &char> {
    chars.iter().filter(|c| !is_space(**c))
}

/// `None` where spaCy raises (the texts differ by more than whitespace and case).
pub(crate) fn y2x_data(a: &[impl AsRef<str>], b: &[impl AsRef<str>]) -> Option<Vec<usize>> {
    let (str_a, tok_a) = lowered(a)?;
    let (str_b, tok_b) = lowered(b)?;
    if !without_space(&str_a).eq(without_space(&str_b)) {
        return None;
    }
    let a_len: Vec<usize> = a.iter().map(|t| t.as_ref().chars().count()).collect();
    let b_len: Vec<usize> = b.iter().map(|t| t.as_ref().chars().count()).collect();
    let (mut ca, mut cb) = (0, 0);
    let mut prev_b = usize::MAX;
    let mut b2a: Vec<Vec<usize>> = Vec::new();
    while ca < str_a.len() && cb < str_b.len() {
        let (ta, tb) = (tok_a[ca], tok_b[cb]);
        if prev_b != tb {
            b2a.push(Vec::new());
        }
        let starts_a = ca == 0 || tok_a[ca - 1] < ta;
        let starts_b = cb == 0 || tok_b[cb - 1] < tb;
        if a[ta].as_ref() == b[tb].as_ref() && starts_a && starts_b {
            b2a.last_mut().expect("pushed").push(ta);
            ca += a_len[ta];
            cb += b_len[tb];
        } else if str_a[ca] == str_b[cb] {
            b2a.last_mut().expect("pushed").push(ta);
            ca += 1;
            cb += 1;
        } else if is_space(str_a[ca]) {
            ca += 1;
        } else if is_space(str_b[cb]) {
            cb += 1;
        } else {
            return None;
        }
        prev_b = tb;
    }
    let mut data = Vec::new();
    for mut set in b2a {
        set.sort_unstable();
        set.dedup();
        data.extend(set);
    }
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::y2x_data;

    #[test]
    fn aligns_split_tokens() {
        let a = ["I", "can't", "go."];
        let b = ["I", "ca", "n't", "go", "."];
        assert_eq!(y2x_data(&a, &b), Some(vec![0, 1, 1, 2, 2]));
        assert_eq!(y2x_data(&["ab"], &["a", "c"]), None);
        assert_eq!(y2x_data(&["ΟΣ", "Α"], &["ΟΣΑ"]), Some(vec![0, 1]));
    }
}
