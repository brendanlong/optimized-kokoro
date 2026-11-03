//! The `tok2vec` and `tagger` components: thinc's MultiHashEmbed, MaxoutWindowEncoder and a linear
//! output layer, in f32.

use serde::Deserialize;
use std::collections::HashMap;

/// Hyperparameters exported alongside the weights.
#[derive(Debug, Deserialize)]
pub(crate) struct ModelMeta {
    embed_seeds: Vec<u32>,
    embed_columns: Vec<usize>,
    features: Vec<String>,
    pad: usize,
    window: usize,
    pieces: usize,
}

/// The order of feature columns, as `FeatureExtractor` lays them out.
pub(crate) const FEATURES: [&str; 6] = ["NORM", "PREFIX", "SUFFIX", "SHAPE", "SPACY", "IS_SPACE"];

const PIECES: usize = 3;

/// One HashEmbed table, already multiplied through its slice of the mixing Maxout's weights: a
/// token's mixed pre-activation is the sum of one row per hash, so the layer costs no matmul.
struct Embed {
    /// `(rows, width * PIECES)`.
    projected: Vec<f32>,
    rows: u32,
    seed: u32,
}

/// The Maxout pieces' biases and the LayerNorm after them.
struct Activation {
    b: Vec<f32>,
    g: Vec<f32>,
    beta: Vec<f32>,
}

/// `expand_window >> Maxout >> LayerNorm`, used residually.
struct Conv {
    /// `(width * PIECES, 3 * width)`.
    w: Packed,
    act: Activation,
}

pub(crate) struct Tagger {
    embeds: Vec<Embed>,
    mix: Activation,
    encode: Vec<Conv>,
    out_w: Packed,
    out_b: Vec<f32>,
    width: usize,
}

/// One tensor's entry in a safetensors header.
#[derive(Deserialize)]
struct TensorInfo {
    dtype: String,
    shape: Vec<usize>,
    data_offsets: (usize, usize),
}

/// A safetensors file (8-byte little-endian header length, JSON header, then the data), read
/// only as far as the tagger needs: little-endian `F32` tensors by name. The file is the bundled
/// `data/tagger.safetensors`, so any inconsistency is a bug and panics.
struct SafeTensors<'a> {
    tensors: HashMap<String, TensorInfo>,
    data: &'a [u8],
}

impl<'a> SafeTensors<'a> {
    fn parse(bytes: &'a [u8]) -> Self {
        let (len, rest) = bytes.split_first_chunk::<8>().expect("safetensors: no header length");
        let len = usize::try_from(u64::from_le_bytes(*len)).expect("safetensors: header length");
        assert!(len <= rest.len(), "safetensors: header length {len} past the end of the file");
        let (header, data) = rest.split_at(len);
        let mut entries: HashMap<String, serde_json::Value> =
            serde_json::from_slice(header).unwrap_or_else(|e| panic!("safetensors header: {e}"));
        entries.remove("__metadata__");
        let tensors = entries
            .into_iter()
            .map(|(name, entry)| {
                let info: TensorInfo = serde_json::from_value(entry).unwrap_or_else(|e| panic!("{name}: {e}"));
                (name, info)
            })
            .collect();
        SafeTensors { tensors, data }
    }

    fn shape(&self, name: &str) -> Option<&[usize]> {
        self.tensors.get(name).map(|t| t.shape.as_slice())
    }

    /// The `F32` tensor `name`, which must have shape `shape`.
    fn tensor(&self, name: &str, shape: &[usize]) -> Vec<f32> {
        let t = self.tensors.get(name).unwrap_or_else(|| panic!("{name}: missing"));
        assert_eq!(t.dtype, "F32", "{name}");
        assert_eq!(t.shape, shape, "{name}");
        let (start, end) = t.data_offsets;
        let bytes = self.data.get(start..end).unwrap_or_else(|| panic!("{name}: data {start}..{end} out of bounds"));
        let elements = shape.iter().try_fold(1usize, |n, &d| n.checked_mul(d));
        assert_eq!(elements.and_then(|n| n.checked_mul(4)), Some(bytes.len()), "{name}: data size");
        bytes.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect()
    }
}

/// Output columns per block (`b`'s rows): two rows of `a` times 16 columns is 8 SSE registers of
/// accumulators. Other shapes measured slower, some by 8× when LLVM stopped vectorizing.
const NR: usize = 16;
/// Output rows per block (`a`'s rows).
const MR: usize = 2;

/// `bᵀ` for [`matmul`], from row-major `b: (n, k)`: column blocks of `NR`, each a `(k, NR)` panel
/// (zero-padded past `n`).
struct Packed {
    panels: Vec<[f32; NR]>,
    k: usize,
    n: usize,
}

impl Packed {
    /// Packs `b: (n, k)`, whose rows start `b_stride` apart.
    fn new(b: &[f32], b_stride: usize, k: usize, n: usize) -> Self {
        let row = |j: usize| &b[j * b_stride..][..k];
        let mut panels = Vec::with_capacity(n.div_ceil(NR) * k);
        for j in (0..n).step_by(NR) {
            for t in 0..k {
                panels.push(std::array::from_fn(|l| if j + l < n { row(j + l)[t] } else { 0.0 }));
            }
        }
        Packed { panels, k, n }
    }
}

/// `c = a · bᵀ` for row-major `a: (m, k)` and `c: (m, n)`.
///
/// Every output is summed in order over `k` from zero, without FMA, so the result is the same
/// whatever the blocking or the target's vector width.
fn matmul(a: &[f32], m: usize, b: &Packed, c: &mut [f32]) {
    let (k, n) = (b.k, b.n);
    let a = &a[..m * k];
    let c = &mut c[..m * n];
    if k == 0 {
        c.fill(0.0);
        return;
    }
    let mut a_panel = vec![[0f32; MR]; k];
    for (i, a_rows) in (0..m).step_by(MR).zip(a.chunks(MR * k)) {
        let rows = a_rows.len() / k;
        for (t, v) in a_panel.iter_mut().enumerate() {
            *v = std::array::from_fn(|r| if r < rows { a_rows[r * k + t] } else { 0.0 });
        }
        for (j, b_panel) in (0..n).step_by(NR).zip(b.panels.chunks_exact(k)) {
            let mut acc = [[0f32; NR]; MR];
            for (av, bv) in a_panel.iter().zip(b_panel) {
                for (acc, &a) in acc.iter_mut().zip(av) {
                    for (acc, &b) in acc.iter_mut().zip(bv) {
                        *acc += a * b;
                    }
                }
            }
            let cols = NR.min(n - j);
            for (r, acc) in acc.iter().take(rows).enumerate() {
                c[(i + r) * n + j..][..cols].copy_from_slice(&acc[..cols]);
            }
        }
    }
}

/// `c = a · bᵀ` for row-major `a: (m, k)`, `b: (n, k)` with row stride `b_stride`, `c: (m, n)`.
fn matmul_bt(a: &[f32], b: &[f32], b_stride: usize, c: &mut [f32], m: usize, k: usize, n: usize) {
    matmul(a, m, &Packed::new(b, b_stride, k, n), c);
}

impl Activation {
    fn load(st: &SafeTensors, name: &str, width: usize) -> Self {
        Activation {
            b: st.tensor(&format!("{name}.b"), &[width * PIECES]),
            g: st.tensor(&format!("{name}.G"), &[width]),
            beta: st.tensor(&format!("{name}.bias"), &[width]),
        }
    }

    /// Maxout over each row's pieces of `pre` (`(rows, width * PIECES)`, without bias), then
    /// LayerNorm, into `out` (`(rows, width)`).
    fn apply(&self, pre: &[f32], out: &mut [f32]) {
        let width = self.g.len();
        for (y, z) in out.chunks_exact_mut(width).zip(pre.chunks_exact(width * PIECES)) {
            for ((y, z), b) in y.iter_mut().zip(z.chunks_exact(PIECES)).zip(self.b.chunks_exact(PIECES)) {
                *y = (z[0] + b[0]).max(z[1] + b[1]).max(z[2] + b[2]);
            }
            layer_norm(y, &self.g, &self.beta);
        }
    }
}

fn layer_norm(x: &mut [f32], g: &[f32], b: &[f32]) {
    let n = x.len() as f32;
    let mean = x.iter().sum::<f32>() / n;
    let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n + 1e-8;
    let scale = var.powf(-0.5);
    for ((x, g), b) in x.iter_mut().zip(g).zip(b) {
        *x = (*x - mean) * scale * g + b;
    }
}

impl Tagger {
    pub fn load(weights: &[u8], meta: &ModelMeta, n_tags: usize) -> Self {
        assert_eq!(meta.features, FEATURES, "feature columns");
        assert_eq!(meta.embed_columns, (0..FEATURES.len()).collect::<Vec<_>>(), "embedding columns");
        assert_eq!((meta.window, meta.pieces), (1, PIECES), "encoder shape");
        let st = SafeTensors::parse(weights);
        let width = st.shape("mix.G").expect("mix.G")[0];
        let n_mix_in = width * meta.embed_seeds.len();
        let mix_w = st.tensor("mix.W", &[width * PIECES, n_mix_in]);
        let embeds = meta
            .embed_seeds
            .iter()
            .enumerate()
            .map(|(i, &seed)| {
                let name = format!("embed.{i}.E");
                let rows = st.shape(&name).expect("embedding")[0];
                let table = st.tensor(&name, &[rows, width]);
                let mut projected = vec![0f32; rows * width * PIECES];
                matmul_bt(&table, &mix_w[i * width..], n_mix_in, &mut projected, rows, width, width * PIECES);
                Embed { projected, rows: rows as u32, seed }
            })
            .collect();
        let encode: Vec<_> = (0..)
            .map_while(|i| st.shape(&format!("encode.{i}.W")).is_some().then_some(i))
            .map(|i| Conv {
                w: Packed::new(
                    &st.tensor(&format!("encode.{i}.W"), &[width * PIECES, width * 3]),
                    width * 3,
                    width * 3,
                    width * PIECES,
                ),
                act: Activation::load(&st, &format!("encode.{i}"), width),
            })
            .collect();
        // thinc pads each doc with as many zero rows as the encoder's receptive field.
        assert_eq!(meta.pad, encode.len(), "padding");
        Tagger {
            embeds,
            mix: Activation::load(&st, "mix", width),
            encode,
            out_w: Packed::new(&st.tensor("tagger.W", &[n_tags, width]), width, width, n_tags),
            out_b: st.tensor("tagger.b", &[n_tags]),
            width,
        }
    }

    pub fn n_tags(&self) -> usize {
        self.out_b.len()
    }

    /// Unnormalized tag scores, `(tokens, n_tags)`, from each token's feature keys.
    pub fn scores(&self, features: &[[u64; FEATURES.len()]]) -> Vec<f32> {
        let n = features.len();
        if n == 0 {
            return Vec::new();
        }
        let w = self.width;
        let wp = w * PIECES;
        let pad = self.encode.len();
        let total = n + 2 * pad;
        let mut pre = vec![0f32; total * wp];
        for (row, keys) in pre.chunks_exact_mut(wp).zip(features) {
            for (embed, &key) in self.embeds.iter().zip(keys) {
                for h in crate::hash::murmur3_uint64(key, embed.seed) {
                    let at = (h % embed.rows) as usize * wp;
                    for (o, e) in row.iter_mut().zip(&embed.projected[at..at + wp]) {
                        *o += e;
                    }
                }
            }
        }
        // The encoder runs over the doc with `pad` zero rows either side, as thinc's
        // `with_array(pad=...)` does. Those rows change from layer to layer and feed the edges,
        // so each layer computes the rows the remaining layers can still see.
        let mut x = vec![0f32; total * w];
        self.mix.apply(&pre[..n * wp], &mut x[pad * w..(pad + n) * w]);
        let mut cols = vec![0f32; total * 3 * w];
        let mut y = vec![0f32; total * w];
        for (l, conv) in self.encode.iter().enumerate() {
            let (lo, hi) = (l + 1, total - l - 1);
            let rows = hi - lo;
            for (i, col) in (lo..hi).zip(cols.chunks_exact_mut(3 * w)) {
                col.copy_from_slice(&x[(i - 1) * w..(i + 2) * w]);
            }
            matmul(&cols, rows, &conv.w, &mut pre);
            conv.act.apply(&pre[..rows * wp], &mut y[..rows * w]);
            for (x, y) in x[lo * w..hi * w].iter_mut().zip(&y) {
                *x += y;
            }
        }
        let tags = self.n_tags();
        let mut scores = vec![0f32; n * tags];
        matmul(&x[pad * w..], n, &self.out_w, &mut scores);
        for s in scores.chunks_exact_mut(tags) {
            for (s, b) in s.iter_mut().zip(&self.out_b) {
                *s += b;
            }
        }
        scores
    }
}

/// First index of the maximum, like `numpy.argmax`.
pub(crate) fn argmax(xs: &[f32]) -> usize {
    let mut best = 0;
    for (i, &x) in xs.iter().enumerate() {
        if x > xs[best] {
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::matmul_bt;

    /// A deterministic spread of values in [-1, 1).
    fn values(n: usize, seed: u32) -> Vec<f32> {
        let mut x = seed.wrapping_mul(2_654_435_761) | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 8) as f32 / (1 << 23) as f32 - 1.0
            })
            .collect()
    }

    /// The blocked loop against a plain f64 loop at the tagger's shapes, with row counts that aren't
    /// multiples of the kernels' blocks, a padded stride, and output past `m * n` left alone.
    #[test]
    fn matmul_matches_a_plain_loop() {
        for (k, n, b_stride) in [(96, 50, 96), (288, 288, 288), (96, 288, 576), (5, 3, 7)] {
            for m in [0, 1, 2, 7, 16, 33, 100] {
                let a = values(m * k, 1);
                let b = values((n - 1) * b_stride + k, 2);
                let mut c = vec![f32::NAN; m * n + 3];
                matmul_bt(&a, &b, b_stride, &mut c, m, k, n);
                for i in 0..m {
                    for j in 0..n {
                        let want: f64 = (0..k).map(|t| f64::from(a[i * k + t]) * f64::from(b[j * b_stride + t])).sum();
                        let got = f64::from(c[i * n + j]);
                        assert!((got - want).abs() < 1e-4, "m={m} k={k} n={n} ({i}, {j}): {got} vs {want}");
                    }
                }
                assert!(c[m * n..].iter().all(|x| x.is_nan()), "wrote past the output");
            }
        }
    }

    #[test]
    fn empty_dimensions() {
        let mut c = vec![f32::NAN; 6];
        matmul_bt(&[], &[], 0, &mut c, 2, 0, 3);
        assert!(c.iter().all(|&x| x == 0.0), "k = 0 gives zeros");
        let mut c = vec![f32::NAN; 1];
        matmul_bt(&[], &[1.0], 1, &mut c, 0, 1, 1);
        matmul_bt(&[1.0], &[], 1, &mut c, 1, 1, 0);
        assert!(c[0].is_nan(), "nothing to write");
    }
}
