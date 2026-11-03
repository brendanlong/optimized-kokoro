//! misaki's `subtokenize`: `regex.findall` with
//!
//! ```text
//! ^['‘’]+ | \p{Lu}(?=\p{Lu}\p{Ll}) | (?:^-)?(?:\d?[,.]?\d)+ | [-_]+ | ['‘’]{2,}
//! | \p{L}*?(?:['‘’]\p{L})*?\p{Ll}(?=\p{Lu}) | \p{L}+(?:['‘’]\p{L})* | [^-_\p{L}'‘’\d] | ['‘’]+$
//! ```
//!
//! The Rust `regex` crate has neither lookahead nor backtracking semantics for the lazy alternative,
//! so each alternative is matched by hand, in order, the way a backtracking engine would.

use crate::text::{re_digit, re_letter, re_lower, re_upper};

fn is_quote(c: char) -> bool {
    matches!(c, '\'' | '‘' | '’')
}

struct Chars {
    chars: Vec<char>,
}

impl Chars {
    fn at(&self, i: usize) -> Option<char> {
        self.chars.get(i).copied()
    }

    fn is(&self, i: usize, pred: impl Fn(char) -> bool) -> bool {
        self.at(i).is_some_and(pred)
    }

    fn run(&self, mut i: usize, pred: impl Fn(char) -> bool) -> usize {
        while self.is(i, &pred) {
            i += 1;
        }
        i
    }

    /// `$`: the end, or just before a final newline.
    fn at_end(&self, i: usize) -> bool {
        let n = self.chars.len();
        i == n || (i + 1 == n && self.chars[i] == '\n')
    }

    /// End of the match starting at `i`, if any alternative matches there.
    fn match_at(&self, i: usize) -> Option<usize> {
        // ^['‘’]+
        if i == 0 && self.is(0, is_quote) {
            return Some(self.run(0, is_quote));
        }
        // \p{Lu}(?=\p{Lu}\p{Ll})
        if self.is(i, re_upper) && self.is(i + 1, re_upper) && self.is(i + 2, re_lower) {
            return Some(i + 1);
        }
        // (?:^-)?(?:\d?[,.]?\d)+
        if i == 0
            && self.at(0) == Some('-')
            && let Some(end) = self.numbers(1)
        {
            return Some(end);
        }
        if let Some(end) = self.numbers(i) {
            return Some(end);
        }
        // [-_]+
        let end = self.run(i, |c| c == '-' || c == '_');
        if end > i {
            return Some(end);
        }
        // ['‘’]{2,}
        let end = self.run(i, is_quote);
        if end >= i + 2 {
            return Some(end);
        }
        // \p{L}*?(?:['‘’]\p{L})*?\p{Ll}(?=\p{Lu})
        let mut letters = i;
        loop {
            let mut q = letters;
            loop {
                if self.is(q, re_lower) && self.is(q + 1, re_upper) {
                    return Some(q + 1);
                }
                if self.is(q, is_quote) && self.is(q + 1, re_letter) {
                    q += 2;
                } else {
                    break;
                }
            }
            if !self.is(letters, re_letter) {
                break;
            }
            letters += 1;
        }
        // \p{L}+(?:['‘’]\p{L})*
        if self.is(i, re_letter) {
            let mut end = self.run(i, re_letter);
            while self.is(end, is_quote) && self.is(end + 1, re_letter) {
                end += 2;
            }
            return Some(end);
        }
        // [^-_\p{L}'‘’\d]
        if self.is(i, |c| !(c == '-' || c == '_' || re_letter(c) || is_quote(c) || re_digit(c))) {
            return Some(i + 1);
        }
        // ['‘’]+$
        let end = self.run(i, is_quote);
        (end > i && self.at_end(end)).then_some(end)
    }

    /// `(?:\d?[,.]?\d)+`, each repetition taking the first of its options that matches.
    fn numbers(&self, start: usize) -> Option<usize> {
        let sep = |c: char| c == ',' || c == '.';
        let mut i = start;
        loop {
            let step = if self.is(i, re_digit) && self.is(i + 1, sep) && self.is(i + 2, re_digit) {
                3
            } else if (self.is(i, re_digit) || self.is(i, sep)) && self.is(i + 1, re_digit) {
                2
            } else if self.is(i, re_digit) {
                1
            } else {
                break;
            };
            i += step;
        }
        (i > start).then_some(i)
    }
}

/// The pieces of `word`, as `regex.findall` returns them (characters no alternative matches are dropped).
pub fn subtokenize(word: &str) -> Vec<&str> {
    let offsets: Vec<usize> = word.char_indices().map(|(o, _)| o).chain([word.len()]).collect();
    let chars = Chars { chars: word.chars().collect() };
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.chars.len() {
        match chars.match_at(i) {
            Some(end) => {
                out.push(&word[offsets[i]..offsets[end]]);
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::subtokenize;

    #[test]
    fn splits_like_misaki() {
        assert_eq!(subtokenize("iPhone"), ["i", "Phone"]);
        assert_eq!(subtokenize("HTMLParser"), ["HTMLParser"]);
        assert_eq!(subtokenize("don't"), ["don't"]);
        assert_eq!(subtokenize("'tis"), ["'", "tis"]);
        assert_eq!(subtokenize("-1,234.5"), ["-1,234.5"]);
        assert_eq!(subtokenize("a-b_c"), ["a", "-", "b", "_", "c"]);
        assert_eq!(subtokenize("1'2"), ["1", "2"]);
        assert_eq!(subtokenize("rock''n"), ["rock", "''", "n"]);
        assert_eq!(subtokenize("dogs'"), ["dogs", "'"]);
        assert_eq!(subtokenize("x2y"), ["x", "2", "y"]);
        assert_eq!(subtokenize("McDonald's"), ["Mc", "Donald's"]);
    }
}
