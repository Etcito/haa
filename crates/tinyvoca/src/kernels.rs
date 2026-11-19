//! The numeric kernels of the transformer: linear layers, layer norm, GELU, convolutions and rotary attention.
//! Activations are row-major `[frames x channels]` f32. Weights are stored transposed (`[inputs x outputs]`) so
//! every inner loop is a contiguous multiply-add over outputs, which LLVM vectorizes (simd128 on wasm).

use crate::det::Det;

/// A linear layer, `y = x W^T + b`, with `W` kept transposed: `wt[i * outputs + o] = W[o][i]`.
#[derive(Clone)]
pub struct Linear {
    pub wt: Vec<f32>,
    pub bias: Vec<f32>,
    pub inputs: usize,
    pub outputs: usize,
}

impl Linear {
    /// From a PyTorch weight `[outputs x inputs]` and bias.
    pub fn new(weight: &[f32], bias: &[f32], outputs: usize, inputs: usize) -> Self {
        let mut wt = vec![0.0; inputs * outputs];
        for o in 0..outputs {
            for i in 0..inputs {
                wt[i * outputs + o] = weight[o * inputs + i];
            }
        }
        Linear { wt, bias: bias.to_vec(), inputs, outputs }
    }
}

#[derive(Clone)]
pub struct Norm {
    pub weight: Vec<f32>,
    pub bias: Vec<f32>,
}

#[cfg_attr(target_arch = "wasm32", target_feature(enable = "simd128"))]
fn axpy4(y: &mut [f32], width: usize, a: [f32; 4], w: &[f32]) {
    let (y0, rest) = y.split_at_mut(width);
    let (y1, rest) = rest.split_at_mut(width);
    let (y2, y3) = rest.split_at_mut(width);
    let (y3, w) = (&mut y3[..width], &w[..width]);
    for o in 0..width {
        let v = w[o];
        y0[o] += a[0] * v;
        y1[o] += a[1] * v;
        y2[o] += a[2] * v;
        y3[o] += a[3] * v;
    }
}

#[cfg_attr(target_arch = "wasm32", target_feature(enable = "simd128"))]
fn axpy(y: &mut [f32], a: f32, w: &[f32]) {
    for (y, &v) in y.iter_mut().zip(w) {
        *y += a * v;
    }
}

#[cfg_attr(target_arch = "wasm32", target_feature(enable = "simd128"))]
fn matmul(x: &[f32], count: usize, wt: &[f32], bias: &[f32], inputs: usize, outputs: usize, y: &mut [f32]) {
    let mut t = 0;
    while t < count {
        let rows = (count - t).min(4);
        for r in 0..rows {
            y[(t + r) * outputs..(t + r + 1) * outputs].copy_from_slice(bias);
        }
        if rows == 4 {
            let block = &mut y[t * outputs..(t + 4) * outputs];
            for i in 0..inputs {
                let a = [x[t * inputs + i], x[(t + 1) * inputs + i], x[(t + 2) * inputs + i], x[(t + 3) * inputs + i]];
                axpy4(block, outputs, a, &wt[i * outputs..(i + 1) * outputs]);
            }
        } else {
            for r in 0..rows {
                let row = &mut y[(t + r) * outputs..(t + r + 1) * outputs];
                for i in 0..inputs {
                    axpy(row, x[(t + r) * inputs + i], &wt[i * outputs..(i + 1) * outputs]);
                }
            }
        }
        t += rows;
    }
}

/// `x [count x inputs]` through `layer`.
pub fn linear(x: &[f32], count: usize, layer: &Linear) -> Vec<f32> {
    let mut y = vec![0.0; count * layer.outputs];
    matmul(x, count, &layer.wt, &layer.bias, layer.inputs, layer.outputs, &mut y);
    y
}

/// Layer norm over the last axis (eps 1e-5, PyTorch's default), statistics in f64.
pub fn normalize(x: &[f32], count: usize, norm: &Norm) -> Vec<f32> {
    let width = norm.weight.len();
    let mut y = vec![0.0; count * width];
    for t in 0..count {
        let row = &x[t * width..(t + 1) * width];
        let mean = row.iter().map(|&v| v as f64).sum::<f64>() / width as f64;
        let var = row.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / width as f64;
        let scale = 1.0 / (var + 1e-5).sqrt();
        let out = &mut y[t * width..(t + 1) * width];
        for i in 0..width {
            out[i] = (((row[i] as f64 - mean) * scale) as f32) * norm.weight[i] + norm.bias[i];
        }
    }
    y
}

/// Exact GELU, `x/2 (1 + erf(x/sqrt 2))`, as `F.gelu` computes it.
pub fn gelu(x: &mut [f32]) {
    for v in x.iter_mut() {
        *v = 0.5 * *v * (1.0 + libm::erff(*v * std::f32::consts::FRAC_1_SQRT_2));
    }
}

pub fn add(x: &mut [f32], y: &[f32]) {
    for (a, &b) in x.iter_mut().zip(y) {
        *a += b;
    }
}

/// Depthwise convolution with zero padding `taps / 2`; `weight [channels x taps]`.
pub fn depthwise(x: &[f32], count: usize, weight: &[f32], bias: &[f32], taps: usize) -> Vec<f32> {
    let channels = bias.len();
    let reach = taps / 2;
    let mut y = vec![0.0; count * channels];
    // tap-major copy of the weights so the inner loop runs over channels
    let mut wk = vec![0.0; taps * channels];
    for c in 0..channels {
        for k in 0..taps {
            wk[k * channels + c] = weight[c * taps + k];
        }
    }
    for t in 0..count {
        let out = &mut y[t * channels..(t + 1) * channels];
        out.copy_from_slice(bias);
        for k in 0..taps {
            let src = t as isize + k as isize - reach as isize;
            if src < 0 || src >= count as isize {
                continue;
            }
            let row = &x[src as usize * channels..(src as usize + 1) * channels];
            let w = &wk[k * channels..(k + 1) * channels];
            for c in 0..channels {
                out[c] += w[c] * row[c];
            }
        }
    }
    y
}

/// A full 1-D convolution (`weight [outputs x inputs x taps]`, zero padding `taps / 2`), kept as one transposed
/// linear layer per tap.
#[derive(Clone)]
pub struct Conv {
    pub taps: Vec<Linear>,
    pub bias: Vec<f32>,
}

impl Conv {
    pub fn new(weight: &[f32], bias: &[f32], outputs: usize, inputs: usize, taps: usize) -> Self {
        let zero = vec![0.0; outputs];
        let taps = (0..taps)
            .map(|k| {
                let w: Vec<f32> = (0..outputs * inputs).map(|oi| weight[oi * taps + k]).collect();
                Linear::new(&w, &zero, outputs, inputs)
            })
            .collect();
        Conv { taps, bias: bias.to_vec() }
    }
}

pub fn convolve(x: &[f32], count: usize, conv: &Conv) -> Vec<f32> {
    let outputs = conv.bias.len();
    let reach = conv.taps.len() / 2;
    let mut y = vec![0.0; count * outputs];
    for t in 0..count {
        y[t * outputs..(t + 1) * outputs].copy_from_slice(&conv.bias);
    }
    for (k, tap) in conv.taps.iter().enumerate() {
        let part = linear(x, count, tap);
        for t in 0..count {
            let src = t as isize + reach as isize - k as isize;
            if src < 0 || src >= count as isize {
                continue;
            }
            // y[src] gets W_k x[src + k - reach] = W_k x[t]
            add(&mut y[src as usize * outputs..(src as usize + 1) * outputs], &part[t * outputs..(t + 1) * outputs]);
        }
    }
    y
}

/// The rotary table of a stack: `cos, sin [count x width/2]`, computed in f32 like `rotary_table`.
pub fn rotary(count: usize, width: usize) -> (Vec<f32>, Vec<f32>) {
    let half = width / 2;
    let inverse: Vec<f32> =
        (0..half).map(|j| 1.0 / (10000f64.dpowf(((2 * j) as f32 / width as f32) as f64) as f32)).collect();
    let mut cos = vec![0.0; count * half];
    let mut sin = vec![0.0; count * half];
    for t in 0..count {
        for j in 0..half {
            let angle = t as f32 * inverse[j];
            cos[t * half + j] = angle.dcos();
            sin[t * half + j] = angle.dsin();
        }
    }
    (cos, sin)
}

/// `e^x` for `x <= 0` (the softmax's shifted scores), in plain arithmetic so a loop of it vectorizes: Cody-Waite
/// reduction by ln 2 and a degree-6 polynomial, about 1 ulp. Below -87 it gives about 1.6e-38 rather than 0.
#[inline(always)]
fn exp_nonpositive(x: f32) -> f32 {
    let x = x.max(-87.0);
    let n = (x * std::f32::consts::LOG2_E + 0.5).floor();
    let r = x - n * 0.693_145_75 - n * 1.428_606_8e-6;
    let p = 1.0 + r * (1.0 + r * (0.5 + r * (1.0 / 6.0 + r * (1.0 / 24.0 + r * (1.0 / 120.0 + r * (1.0 / 720.0))))));
    p * f32::from_bits(((n as i32 + 127) << 23) as u32)
}

/// Softmax of one row of scores in place (`scale` applied first).
#[cfg_attr(target_arch = "wasm32", target_feature(enable = "simd128"))]
fn softmax(row: &mut [f32], scale: f32) {
    let mut top = f32::NEG_INFINITY;
    for s in row.iter_mut() {
        *s *= scale;
        top = top.max(*s);
    }
    let mut total = 0.0f32;
    for s in row.iter_mut() {
        *s = exp_nonpositive(*s - top);
        total += *s;
    }
    let inv = 1.0 / total;
    for s in row.iter_mut() {
        *s *= inv;
    }
}

/// Rotary multi-head self-attention over a whole utterance (no mask: one unpadded sequence).
/// `qkv [count x 3C]` as `nn.Linear(C, 3C)` lays it out; returns the heads concatenated `[count x C]`.
/// Scores and mixing run through [`matmul`] on blocks of query rows, so memory stays `rows x count`.
pub fn attention(qkv: &[f32], count: usize, channels: usize, heads: usize, cos: &[f32], sin: &[f32]) -> Vec<f32> {
    const ROWS: usize = 64;
    let width = channels / heads;
    let half = width / 2;
    let scale = 1.0 / (width as f32).sqrt();
    let mut mixed = vec![0.0; count * channels];
    let mut q = vec![0.0; count * width];
    let mut kt = vec![0.0; width * count];
    let mut v = vec![0.0; count * width];
    let zero_t = vec![0.0; count];
    let zero_w = vec![0.0; width];
    let mut share = vec![0.0f32; ROWS.min(count) * count];
    let mut mix = vec![0.0f32; ROWS.min(count) * width];
    for h in 0..heads {
        for t in 0..count {
            let base = t * 3 * channels + h * width;
            let row = t * width;
            for j in 0..half {
                let (c, s) = (cos[t * half + j], sin[t * half + j]);
                let (a, b) = (qkv[base + 2 * j], qkv[base + 2 * j + 1]);
                q[row + 2 * j] = a * c - b * s;
                q[row + 2 * j + 1] = a * s + b * c;
                let (a, b) = (qkv[base + channels + 2 * j], qkv[base + channels + 2 * j + 1]);
                kt[2 * j * count + t] = a * c - b * s;
                kt[(2 * j + 1) * count + t] = a * s + b * c;
            }
            v[row..row + width].copy_from_slice(&qkv[base + 2 * channels..base + 2 * channels + width]);
        }
        let mut r0 = 0;
        while r0 < count {
            let rows = (count - r0).min(ROWS);
            let scores = &mut share[..rows * count];
            matmul(&q[r0 * width..(r0 + rows) * width], rows, &kt, &zero_t, width, count, scores);
            for row in scores.chunks_exact_mut(count) {
                softmax(row, scale);
            }
            let out = &mut mix[..rows * width];
            matmul(scores, rows, &v, &zero_w, count, width, out);
            for r in 0..rows {
                let at = (r0 + r) * channels + h * width;
                mixed[at..at + width].copy_from_slice(&out[r * width..(r + 1) * width]);
            }
            r0 += rows;
        }
    }
    mixed
}
