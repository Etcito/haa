//! The unknown-word model (dizione `g2p.py`): letters -> CMUdict phonemes. Letter embeddings with `slots` positions
//! each, residual convolutions, a bidirectional GRU and a CTC output read greedily. Weights come from `g2p.npz` (or
//! any tensor source, e.g. the `g2p/` tensors of tiny-voca's JS GGUF); the arithmetic is in f64, as the JS port.

use std::collections::HashMap;

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

/// A dense float tensor, row-major.
#[derive(Clone, Debug)]
pub struct Tensor {
    pub shape: Vec<usize>,
    pub data: Vec<f64>,
}

pub struct Model {
    letters: HashMap<char, usize>,
    symbols: Vec<String>,
    slots: usize,
    convs: usize,
    layers: usize,
    w: HashMap<String, Tensor>,
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// y[t] = W x[t] + b for `rows` rows.
fn affine(x: &[f64], rows: usize, w: &Tensor, bias: Option<&Tensor>) -> Vec<f64> {
    let (outputs, inputs) = (w.shape[0], w.shape[1]);
    let mut y = vec![0.0; rows * outputs];
    for t in 0..rows {
        let xr = &x[t * inputs..(t + 1) * inputs];
        for o in 0..outputs {
            let mut sum = bias.map_or(0.0, |b| b.data[o]);
            let wr = &w.data[o * inputs..(o + 1) * inputs];
            for i in 0..inputs {
                sum += wr[i] * xr[i];
            }
            y[t * outputs + o] = sum;
        }
    }
    y
}

fn layer_norm(x: &[f64], rows: usize, weight: &Tensor, bias: &Tensor) -> Vec<f64> {
    let width = weight.data.len();
    let mut y = vec![0.0; x.len()];
    for t in 0..rows {
        let r = &x[t * width..(t + 1) * width];
        let mean: f64 = r.iter().map(|v| v / width as f64).sum();
        let variance: f64 = r.iter().map(|v| (v - mean).powi(2) / width as f64).sum();
        for i in 0..width {
            y[t * width + i] = (r[i] - mean) / (variance + 1e-5).sqrt() * weight.data[i] + bias.data[i];
        }
    }
    y
}

fn conv(x: &[f64], rows: usize, w: &Tensor, bias: &Tensor) -> Vec<f64> {
    let (outputs, inputs, kernel) = (w.shape[0], w.shape[1], w.shape[2]);
    let pad = kernel / 2;
    let mut y = vec![0.0; rows * outputs];
    for t in 0..rows {
        for o in 0..outputs {
            let mut sum = bias.data[o];
            for k in 0..kernel {
                let from = t as isize + k as isize - pad as isize;
                if from < 0 || from >= rows as isize {
                    continue;
                }
                for i in 0..inputs {
                    sum += w.data[(o * inputs + i) * kernel + k] * x[from as usize * inputs + i];
                }
            }
            y[t * outputs + o] = sum;
        }
    }
    y
}

fn gru(x: &[f64], rows: usize, wi: &Tensor, wh: &Tensor, bi: &Tensor, bh: &Tensor, reverse: bool) -> Vec<f64> {
    let size = wh.shape[1];
    let inputs = affine(x, rows, wi, Some(bi));
    let mut out = vec![0.0; rows * size];
    let mut state = vec![0.0; size];
    for n in 0..rows {
        let t = if reverse { rows - 1 - n } else { n };
        let at = t * 3 * size;
        let recurrent = affine(&state, 1, wh, Some(bh));
        let mut next = vec![0.0; size];
        for i in 0..size {
            let reset = sigmoid(inputs[at + i] + recurrent[i]);
            let update = sigmoid(inputs[at + size + i] + recurrent[size + i]);
            let candidate = (inputs[at + 2 * size + i] + reset * recurrent[2 * size + i]).tanh();
            next[i] = (1.0 - update) * candidate + update * state[i];
        }
        state = next;
        out[t * size..(t + 1) * size].copy_from_slice(&state);
    }
    out
}

impl Model {
    /// The model from `g2p.json` and its tensors (names as in `g2p.npz`: `embed__weight`, `gru__weight_ih_l0`...).
    pub fn new(settings: &str, tensors: HashMap<String, Tensor>) -> Result<Model, String> {
        let s: Value = serde_json::from_str(settings).map_err(|e| format!("g2p.json: {e}"))?;
        let letters = s["letters"].as_str().ok_or("g2p.json: no letters")?.chars().enumerate().map(|(i, c)| (c, i + 1)).collect();
        let symbols =
            s["symbols"].as_array().ok_or("g2p.json: no symbols")?.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        let num = |k: &str| s[k].as_u64().map(|v| v as usize).ok_or(format!("g2p.json: no {k}"));
        let model = Model { letters, symbols, slots: num("slots")?, convs: num("convs")?, layers: num("layers")?, w: tensors };
        for name in ["embed__weight", "slot", "out__weight", "out__bias"] {
            model.w.get(name).ok_or(format!("g2p: missing tensor {name}"))?;
        }
        Ok(model)
    }

    fn t(&self, name: &str) -> &Tensor {
        self.w.get(name).unwrap_or_else(|| panic!("g2p: missing tensor {name}"))
    }

    /// The word reduced to the letters the model knows (accents removed).
    pub fn spellable(&self, word: &str) -> String {
        let plain: String = word.to_lowercase().nfkd().filter(|c| self.letters.contains_key(c)).collect();
        plain.trim_matches('\'').to_string()
    }

    fn logits(&self, word: &str) -> (usize, Vec<f64>) {
        let ids: Vec<usize> = word.chars().map(|c| self.letters[&c]).collect();
        let (embed, slot) = (self.t("embed__weight"), self.t("slot"));
        let width = embed.shape[1];
        let rows = ids.len() * self.slots;
        let mut x = vec![0.0; rows * width];
        for (n, &id) in ids.iter().enumerate() {
            for s in 0..self.slots {
                for i in 0..width {
                    x[(n * self.slots + s) * width + i] = embed.data[id * width + i] + slot.data[s * width + i];
                }
            }
        }
        for block in 0..self.convs {
            let normed = layer_norm(&x, rows, self.t(&format!("norms__{block}__weight")), self.t(&format!("norms__{block}__bias")));
            let y = conv(&normed, rows, self.t(&format!("convs__{block}__weight")), self.t(&format!("convs__{block}__bias")));
            for (v, yv) in x.iter_mut().zip(&y) {
                *v += yv.max(0.0);
            }
        }
        for layer in 0..self.layers {
            let run = |suffix: &str, reverse: bool| {
                gru(
                    &x,
                    rows,
                    self.t(&format!("gru__weight_ih_l{layer}{suffix}")),
                    self.t(&format!("gru__weight_hh_l{layer}{suffix}")),
                    self.t(&format!("gru__bias_ih_l{layer}{suffix}")),
                    self.t(&format!("gru__bias_hh_l{layer}{suffix}")),
                    reverse,
                )
            };
            let (fwd, back) = (run("", false), run("_reverse", true));
            let size = fwd.len() / rows;
            let mut both = vec![0.0; rows * 2 * size];
            for t in 0..rows {
                both[t * 2 * size..t * 2 * size + size].copy_from_slice(&fwd[t * size..(t + 1) * size]);
                both[t * 2 * size + size..(t + 1) * 2 * size].copy_from_slice(&back[t * size..(t + 1) * size]);
            }
            x = both;
        }
        (rows, affine(&x, rows, self.t("out__weight"), Some(self.t("out__bias"))))
    }

    /// CMUdict symbols with stress digits for `word` (letters and apostrophes).
    pub fn predict(&self, word: &str) -> Vec<String> {
        let word = self.spellable(word);
        if word.is_empty() {
            return Vec::new();
        }
        let (rows, values) = self.logits(&word);
        let labels = values.len() / rows;
        let mut out = Vec::new();
        let mut previous = 0;
        for t in 0..rows {
            let r = &values[t * labels..(t + 1) * labels];
            let mut label = 0;
            for i in 1..labels {
                if r[i] > r[label] {
                    label = i;
                }
            }
            if label != previous && label != 0 {
                out.push(self.symbols[label - 1].clone());
            }
            previous = label;
        }
        out
    }
}

// ---- npz -------------------------------------------------------------------------------------------------------------

fn u16le(b: &[u8], at: usize) -> Result<usize, String> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize).ok_or_else(|| "npz: truncated".to_string())
}
fn u32le(b: &[u8], at: usize) -> Result<usize, String> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize).ok_or_else(|| "npz: truncated".to_string())
}
fn u64le(b: &[u8], at: usize) -> Result<usize, String> {
    b.get(at..at + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()) as usize).ok_or_else(|| "npz: truncated".to_string())
}

/// The files of a zip archive (stored or deflated entries, zip64 sizes), from its central directory.
pub fn unzip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let eocd = (0..bytes.len().saturating_sub(21)).rev().find(|&i| bytes[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or("zip: no end record")?;
    let mut entries = u16le(bytes, eocd + 10)?;
    let mut offset = u32le(bytes, eocd + 16)?;
    if offset == 0xFFFF_FFFF || entries == 0xFFFF {
        let locator = eocd.checked_sub(20).ok_or("zip64: no locator")?;
        let record = u64le(bytes, locator + 8)?;
        entries = u64le(bytes, record + 32)?;
        offset = u64le(bytes, record + 48)?;
    }
    let mut out = Vec::new();
    let mut at = offset;
    for _ in 0..entries {
        if bytes.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err("zip: bad central entry".into());
        }
        let method = u16le(bytes, at + 10)?;
        let mut csize = u32le(bytes, at + 20)?;
        let mut usize_ = u32le(bytes, at + 24)?;
        let (nlen, xlen, clen) = (u16le(bytes, at + 28)?, u16le(bytes, at + 30)?, u16le(bytes, at + 32)?);
        let mut local = u32le(bytes, at + 42)?;
        let name = String::from_utf8_lossy(&bytes[at + 46..at + 46 + nlen]).to_string();
        let mut x = at + 46 + nlen;
        let xend = x + xlen;
        while x + 4 <= xend {
            let (id, len) = (u16le(bytes, x)?, u16le(bytes, x + 2)?);
            if id == 1 {
                let mut f = x + 4;
                for value in [&mut usize_, &mut csize, &mut local] {
                    if *value == 0xFFFF_FFFF {
                        *value = u64le(bytes, f)?;
                        f += 8;
                    }
                }
            }
            x += 4 + len;
        }
        at = xend + clen;
        let start = local + 30 + u16le(bytes, local + 26)? + u16le(bytes, local + 28)?;
        let raw = bytes.get(start..start + csize).ok_or("zip: truncated entry")?;
        let data = match method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec(raw).map_err(|e| format!("zip: {name}: {e:?}"))?,
            m => return Err(format!("zip: {name}: compression method {m}")),
        };
        if data.len() != usize_ {
            return Err(format!("zip: {name}: size"));
        }
        out.push((name, data));
    }
    Ok(out)
}

/// One `.npy` array (little-endian f4 or f8, C order).
pub fn npy(bytes: &[u8]) -> Result<Tensor, String> {
    if bytes.get(..6) != Some(b"\x93NUMPY") {
        return Err("npy: no magic".into());
    }
    let (len, at) = if bytes[6] == 1 { (u16le(bytes, 8)?, 10) } else { (u32le(bytes, 8)?, 12) };
    let header = std::str::from_utf8(&bytes[at..at + len]).map_err(|_| "npy: header")?;
    let field = |key: &str| -> Option<&str> {
        let i = header.find(key)? + key.len();
        Some(header[i..].trim_start_matches([':', ' ', '\'']))
    };
    let descr = field("'descr'").ok_or("npy: no descr")?;
    let width = if descr.starts_with("<f4") {
        4
    } else if descr.starts_with("<f8") {
        8
    } else {
        return Err(format!("npy: dtype {}", &descr[..descr.len().min(4)]));
    };
    if field("'fortran_order'").is_some_and(|f| f.starts_with("True")) {
        return Err("npy: Fortran order".into());
    }
    let shape_text = field("'shape'").ok_or("npy: no shape")?;
    let close = shape_text.find(')').ok_or("npy: shape")?;
    let shape: Vec<usize> =
        shape_text[1..close].split(',').map(str::trim).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap_or(0)).collect();
    let count: usize = shape.iter().product();
    let body = &bytes[at + len..];
    if body.len() < count * width {
        return Err("npy: truncated data".into());
    }
    let data = (0..count)
        .map(|i| {
            let s = &body[i * width..(i + 1) * width];
            if width == 4 { f32::from_le_bytes(s.try_into().unwrap()) as f64 } else { f64::from_le_bytes(s.try_into().unwrap()) }
        })
        .collect();
    Ok(Tensor { shape, data })
}

/// The arrays of an `.npz` archive by name (without `.npy`).
pub fn npz(bytes: &[u8]) -> Result<HashMap<String, Tensor>, String> {
    unzip(bytes)?
        .into_iter()
        .map(|(name, data)| Ok((name.trim_end_matches(".npy").to_string(), npy(&data)?)))
        .collect()
}
