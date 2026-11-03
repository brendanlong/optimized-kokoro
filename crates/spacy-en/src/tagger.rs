//! The `tok2vec` and `tagger` components: thinc's MultiHashEmbed, MaxoutWindowEncoder and a linear
//! output layer, in f32.

use safetensors::{Dtype, SafeTensors};
use serde::Deserialize;

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
    /// `(width * PIECES, 3 * width)`, row-major.
    w: Vec<f32>,
    act: Activation,
}

pub(crate) struct Tagger {
    embeds: Vec<Embed>,
    mix: Activation,
    encode: Vec<Conv>,
    out_w: Vec<f32>,
    out_b: Vec<f32>,
    width: usize,
}

fn tensor(st: &SafeTensors, name: &str, shape: &[usize]) -> Vec<f32> {
    let t = st.tensor(name).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(t.dtype(), Dtype::F32, "{name}");
    assert_eq!(t.shape(), shape, "{name}");
    t.data().chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect()
}

/// `c = a · bᵀ` for row-major `a: (m, k)`, `b: (n, k)` with row stride `b_stride`, `c: (m, n)`.
fn matmul_bt(a: &[f32], b: &[f32], b_stride: usize, c: &mut [f32], m: usize, k: usize, n: usize) {
    assert!(a.len() >= m * k && c.len() >= m * n && b.len() >= (n - 1) * b_stride + k);
    // SAFETY: the asserts above bound every element the strides reach.
    unsafe {
        matrixmultiply::sgemm(
            m,
            k,
            n,
            1.0,
            a.as_ptr(),
            k as isize,
            1,
            b.as_ptr(),
            1,
            b_stride as isize,
            0.0,
            c.as_mut_ptr(),
            n as isize,
            1,
        );
    }
}

impl Activation {
    fn load(st: &SafeTensors, name: &str, width: usize) -> Self {
        Activation {
            b: tensor(st, &format!("{name}.b"), &[width * PIECES]),
            g: tensor(st, &format!("{name}.G"), &[width]),
            beta: tensor(st, &format!("{name}.bias"), &[width]),
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
        let st = SafeTensors::deserialize(weights).expect("tagger weights");
        let width = st.tensor("mix.G").expect("mix.G").shape()[0];
        let n_mix_in = width * meta.embed_seeds.len();
        let mix_w = tensor(&st, "mix.W", &[width * PIECES, n_mix_in]);
        let embeds = meta
            .embed_seeds
            .iter()
            .enumerate()
            .map(|(i, &seed)| {
                let name = format!("embed.{i}.E");
                let rows = st.tensor(&name).expect("embedding").shape()[0];
                let table = tensor(&st, &name, &[rows, width]);
                let mut projected = vec![0f32; rows * width * PIECES];
                matmul_bt(&table, &mix_w[i * width..], n_mix_in, &mut projected, rows, width, width * PIECES);
                Embed { projected, rows: rows as u32, seed }
            })
            .collect();
        let encode: Vec<_> = (0..)
            .map_while(|i| st.tensor(&format!("encode.{i}.W")).is_ok().then_some(i))
            .map(|i| Conv {
                w: tensor(&st, &format!("encode.{i}.W"), &[width * PIECES, width * 3]),
                act: Activation::load(&st, &format!("encode.{i}"), width),
            })
            .collect();
        // thinc pads each doc with as many zero rows as the encoder's receptive field.
        assert_eq!(meta.pad, encode.len(), "padding");
        Tagger {
            embeds,
            mix: Activation::load(&st, "mix", width),
            encode,
            out_w: tensor(&st, "tagger.W", &[n_tags, width]),
            out_b: tensor(&st, "tagger.b", &[n_tags]),
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
            matmul_bt(&cols, &conv.w, 3 * w, &mut pre, rows, 3 * w, wp);
            conv.act.apply(&pre[..rows * wp], &mut y[..rows * w]);
            for (x, y) in x[lo * w..hi * w].iter_mut().zip(&y) {
                *x += y;
            }
        }
        let tags = self.n_tags();
        let mut scores = vec![0f32; n * tags];
        matmul_bt(&x[pad * w..], &self.out_w, w, &mut scores, n, w, tags);
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
