//! jieba 0.42.1's `lcut(text)` (accurate mode with the HMM, its default), for the text misaki
//! gives it: runs of U+4E00-U+9FFF.
//!
//! `Tokenizer.cut` splits its input with `re_han_default`, whose Han range ends at U+9FD5; the
//! characters U+9FD6-U+9FFF fall outside it and come out one by one. Each block of U+4E00-U+9FD5
//! goes through `__cut_DAG`: the dictionary DAG, the most probable route through it (`calc`), and
//! the HMM (`finalseg`) for each run of single characters that isn't a word itself.

use rustc_hash::FxHashMap;

/// finalseg's `MIN_FLOAT`.
const MIN_FLOAT: f64 = -3.14e100;
/// The HMM states, in the order of `'BMES'`; comparing them as Python compares the state letters
/// (`B < E < M < S`) breaks ties in `max`.
const B: usize = 0;
const M: usize = 1;
const E: usize = 2;
const S: usize = 3;
const LETTER: [u8; 4] = *b"BMES";
/// `prob_start.py`.
const START: [f64; 4] = [-0.26268660809250016, -3.14e+100, -3.14e+100, -1.4652633398537678];
/// `prob_trans.py`, `TRANS[from][to]`; the pairs it lacks are never looked up (`PrevStatus`).
const TRANS: [[f64; 4]; 4] = [
    [MIN_FLOAT, -0.916290731874155, -0.510825623765990, MIN_FLOAT],
    [MIN_FLOAT, -1.2603623820268226, -0.33344856811948514, MIN_FLOAT],
    [-0.5897149736854513, MIN_FLOAT, MIN_FLOAT, -0.8085250474669937],
    [-0.7211965654669841, MIN_FLOAT, MIN_FLOAT, -0.6658631448798212],
];
/// `PrevStatus`: the states each state may follow.
const PREV: [[usize; 2]; 4] = [[E, S], [M, B], [B, M], [S, E]];

pub struct Jieba {
    /// `FREQ`: every word with its frequency, and every prefix of a word (0 unless a word too).
    freq: FxHashMap<&'static str, u32>,
    log_total: f64,
    /// Emission log-probabilities per state, `MIN_FLOAT` where finalseg has none.
    emit: FxHashMap<char, [f64; 4]>,
}

impl Jieba {
    pub fn new() -> Jieba {
        let mut lines = include_str!("../data/jieba.txt").lines();
        let total: u64 = lines.next().and_then(|l| l.parse().ok()).expect("jieba.txt starts with the total");
        let mut freq = FxHashMap::default();
        let mut words = Vec::new();
        for line in lines {
            let (word, n) = line.split_once(' ').expect("word freq");
            freq.insert(word, n.parse().expect("a frequency"));
            words.push(word);
        }
        for word in words {
            for (i, _) in word.char_indices().skip(1) {
                freq.entry(&word[..i]).or_insert(0);
            }
        }
        let mut emit = FxHashMap::default();
        for line in include_str!("../data/hmm.txt").lines() {
            let mut fields = line.split(' ');
            let c = fields.next().and_then(|f| f.chars().next()).expect("a character");
            let mut p = [MIN_FLOAT; 4];
            for (slot, field) in p.iter_mut().zip(fields) {
                if field != "-" {
                    *slot = field.parse().expect("a log-probability");
                }
            }
            emit.insert(c, p);
        }
        Jieba { freq, log_total: (total as f64).ln(), emit }
    }

    /// `jieba.lcut(text)` for text of U+4E00-U+9FFF only, as byte ranges of `text`.
    pub fn cut(&self, text: &str, out: &mut Vec<(usize, usize)>) {
        let mut start = 0;
        for (i, c) in text.char_indices() {
            if c > '\u{9FD5}' {
                if start < i {
                    self.cut_dag(text, start, i, out);
                }
                out.push((i, i + c.len_utf8()));
                start = i + c.len_utf8();
            }
        }
        if start < text.len() {
            self.cut_dag(text, start, text.len(), out);
        }
    }

    fn get(&self, s: &str) -> u32 {
        self.freq.get(s).copied().unwrap_or(0)
    }

    /// `__cut_DAG` on `text[from..to]`.
    fn cut_dag(&self, text: &str, from: usize, to: usize, out: &mut Vec<(usize, usize)>) {
        // Character boundaries: `b[i]` is the byte offset of the block's i-th character.
        let mut b: Vec<usize> = text[from..to].char_indices().map(|(i, _)| from + i).collect();
        let n = b.len();
        b.push(to);
        // get_DAG: for each k, the ends i of the words text[k..=i] (or k alone).
        let mut dag: Vec<Vec<usize>> = Vec::with_capacity(n);
        for k in 0..n {
            let mut ends = Vec::new();
            let mut i = k;
            while i < n {
                match self.freq.get(&text[b[k]..b[i + 1]]) {
                    Some(&f) => {
                        if f != 0 {
                            ends.push(i);
                        }
                    }
                    None => break,
                }
                i += 1;
            }
            if ends.is_empty() {
                ends.push(k);
            }
            dag.push(ends);
        }
        // calc: route[idx] = max over x of (log(FREQ or 1) - logtotal + route[x + 1].0, x).
        let mut route = vec![(0.0f64, 0usize); n + 1];
        for idx in (0..n).rev() {
            let mut best: Option<(f64, usize)> = None;
            for &x in &dag[idx] {
                let f = self.get(&text[b[idx]..b[x + 1]]).max(1);
                let score = (f as f64).ln() - self.log_total + route[x + 1].0;
                if best.is_none_or(|(s, bx)| score > s || (score == s && x > bx)) {
                    best = Some((score, x));
                }
            }
            route[idx] = best.expect("every DAG entry has an end");
        }
        let mut x = 0;
        let mut buf: Option<usize> = None; // start (char index) of the run of single characters
        while x < n {
            let y = route[x].1 + 1;
            if y - x == 1 {
                buf.get_or_insert(x);
            } else {
                if let Some(s) = buf.take() {
                    self.flush(text, &b, s, x, out);
                }
                out.push((b[x], b[y]));
            }
            x = y;
        }
        if let Some(s) = buf {
            self.flush(text, &b, s, n, out);
        }
    }

    /// A run of single characters `[s, e)` in `__cut_DAG`: one character as is, a word's characters
    /// one by one, anything else through the HMM.
    fn flush(&self, text: &str, b: &[usize], s: usize, e: usize, out: &mut Vec<(usize, usize)>) {
        if e - s == 1 {
            out.push((b[s], b[e]));
        } else if self.get(&text[b[s]..b[e]]) == 0 {
            self.hmm(text, &b[s..=e], out);
        } else {
            out.extend((s..e).map(|i| (b[i], b[i + 1])));
        }
    }

    /// finalseg's `__cut`: Viterbi over the BMES states, then words at each E and S. `b` holds the
    /// characters' boundaries.
    fn hmm(&self, text: &str, b: &[usize], out: &mut Vec<(usize, usize)>) {
        let n = b.len() - 1;
        let emit = |i: usize| -> [f64; 4] {
            let c = text[b[i]..].chars().next().expect("a character");
            self.emit.get(&c).copied().unwrap_or([MIN_FLOAT; 4])
        };
        let mut v = [0.0f64; 4];
        let e0 = emit(0);
        for y in 0..4 {
            v[y] = START[y] + e0[y];
        }
        // back[t][y]: the state before y at t.
        let mut back: Vec<[u8; 4]> = Vec::with_capacity(n);
        back.push([0; 4]);
        for t in 1..n {
            let em = emit(t);
            let mut next = [0.0f64; 4];
            let mut from = [0u8; 4];
            for y in 0..4 {
                let mut best: Option<(f64, usize)> = None;
                for &y0 in &PREV[y] {
                    let p = v[y0] + TRANS[y0][y] + em[y];
                    if best.is_none_or(|(bp, by)| p > bp || (p == bp && LETTER[y0] > LETTER[by])) {
                        best = Some((p, y0));
                    }
                }
                let (p, y0) = best.expect("two predecessors");
                next[y] = p;
                from[y] = y0 as u8;
            }
            v = next;
            back.push(from);
        }
        let mut state = if v[S] >= v[E] { S } else { E };
        let mut states = vec![0usize; n];
        for t in (0..n).rev() {
            states[t] = state;
            state = back[t][state] as usize;
        }
        let (mut begin, mut next) = (0, 0);
        for (i, &pos) in states.iter().enumerate() {
            match pos {
                B => begin = i,
                E => {
                    out.push((b[begin], b[i + 1]));
                    next = i + 1;
                }
                S => {
                    out.push((b[i], b[i + 1]));
                    next = i + 1;
                }
                _ => {}
            }
        }
        if next < n {
            out.push((b[next], b[n]));
        }
    }
}
