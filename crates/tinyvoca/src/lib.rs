//! tinyvoca: tiny-voca (n4ze3m/tiny-voca, Apache-2.0) inference in Rust, for native and wasm.
//!
//! The weights are an official tiny-voca GGUF that the host fetches and passes in as bytes ([`Model::from_gguf`]);
//! nothing is bundled. The model runs in stages with plain arrays between them (see [`model`]), takes a voice from
//! a blend of the GGUF's two speakers plus `post` edits (voice.tvv.json v3, [`Model::voice`]) and renders a `perf.json` v1 performance ([`Model::perform`]).
//! See NOTICE.md for what was ported from where.

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
    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        let centre = n as f64 / ratio;
        let first = (centre - reach).ceil().max(0.0) as usize;
        let last = ((centre + reach).floor() as usize).min(input.len() - 1);
        let mut acc = 0.0f64;
        for (k, &x) in input.iter().enumerate().take(last + 1).skip(first) {
            let d = k as f64 - centre;
            let arg = d * cutoff;
            let sinc = if arg.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * arg).sin() / (std::f64::consts::PI * arg) };
            let window = 0.5 + 0.5 * (std::f64::consts::PI * d / reach).cos();
            acc += x as f64 * cutoff * sinc * window;
        }
        out.push(acc as f32);
    }
    out
}
