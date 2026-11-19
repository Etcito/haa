//! tinyvoca: tiny-voca (n4ze3m/tiny-voca, Apache-2.0) inference in Rust, for native and wasm.
//!
//! The weights are an official tiny-voca GGUF that the host fetches and passes in as bytes ([`Model::from_gguf`]);
//! nothing is bundled. The model runs in stages with plain arrays between them (see [`model`]), takes a voice from
//! a blend of the GGUF's two speakers plus `post` edits (voice.tvv.json v3, [`Model::voice`]) and renders a `perf.json` v1 performance ([`Model::perform`]).
//! See NOTICE.md for what was ported from where.

mod det;
pub mod edit;
pub mod gguf;
pub mod head;
pub mod kernels;
pub mod model;
pub mod perf;
pub mod voice;

pub use model::{Cepstra, Encoded, Frames, Model, Options, Prosody, Synthesis, pitch_wobble, widen_pitch};
pub use perf::Perf;
pub use voice::{Post, Voice, VoiceFile};

use std::sync::{Arc, Mutex};

use det::Det;

/// The model the host loaded for this process (the wasm module's `haa_tinyvoca_load`), shared by every `speak`
/// node. `None` until a GGUF is loaded.
static SHARED: Mutex<Option<Arc<Model>>> = Mutex::new(None);

/// Installs `model` as the shared model (replacing any earlier one).
pub fn install(model: Model) {
    *SHARED.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(model));
}

/// The shared model, if the host loaded one.
pub fn shared() -> Option<Arc<Model>> {
    SHARED.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Band-limited resampling (Hann-windowed sinc, 32 zero crossings) from `from` Hz to `to` Hz.
pub fn resample(input: &[f32], from: f64, to: f64) -> Vec<f32> {
    if (from - to).abs() < 1e-9 || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to / from;
    let cutoff = ratio.min(1.0) * 0.97;
    let zeros = 32.0;
    let reach = zeros / cutoff;
    let out_len = (input.len() as f64 * ratio).round() as usize;
    let tap = |d: f64| {
        let arg = d * cutoff;
        let sinc = if arg.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * arg).dsin() / (std::f64::consts::PI * arg) };
        let window = 0.5 + 0.5 * (std::f64::consts::PI * d / reach).dcos();
        cutoff * sinc * window
    };
    // integer rates (24 kHz -> 48 or 44.1 kHz): output n sits at input (n * from) / to, so its fractional position
    // takes `to / gcd` values only and the kernel of each is computed once (same taps, same sums)
    let integer = |x: f64| x > 0.0 && x.fract() == 0.0 && x < 1e9;
    if integer(from) && integer(to) {
        let (f, t) = (from as u64, to as u64);
        let g = gcd(f, t);
        let phases = (t / g) as usize;
        if phases <= 4096 {
            let span = (2.0 * reach).ceil() as usize + 2;
            let mut kernels: Vec<Option<(isize, Vec<f64>)>> = vec![None; phases];
            let mut out = Vec::with_capacity(out_len);
            for n in 0..out_len {
                let whole = n as u64 * f;
                let base = (whole / t) as isize;
                let phase = ((whole % t) / g) as usize;
                let centre_frac = (whole % t) as f64 / t as f64;
                let (lo, weights) = kernels[phase].get_or_insert_with(|| {
                    let lo = (centre_frac - reach).ceil() as isize;
                    let hi = (centre_frac + reach).floor() as isize;
                    let w: Vec<f64> = (lo..=hi).map(|j| tap(j as f64 - centre_frac)).collect();
                    debug_assert!(w.len() <= span);
                    (lo, w)
                });
                let mut acc = 0.0f64;
                for (j, w) in weights.iter().enumerate() {
                    let k = base + *lo + j as isize;
                    if k >= 0 && (k as usize) < input.len() {
                        acc += input[k as usize] as f64 * w;
                    }
                }
                out.push(acc as f32);
            }
            return out;
        }
    }
    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        let centre = n as f64 / ratio;
        let first = (centre - reach).ceil().max(0.0) as usize;
        let last = ((centre + reach).floor() as usize).min(input.len() - 1);
        let mut acc = 0.0f64;
        for (k, &x) in input.iter().enumerate().take(last + 1).skip(first) {
            acc += x as f64 * tap(k as f64 - centre);
        }
        out.push(acc as f32);
    }
    out
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}
