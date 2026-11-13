//! haa-fit: the brute-force graph fitter. Given a mono target sample, it searches HAA patches
//! (template graphs, their numeric params and curve breakpoints, one-node structure mutations)
//! for the one whose offline render best matches the target.
//!
//! Pure like haa-core: no fs, time, threads or env; randomness only from `haa_core::rng` with a
//! caller-given seed, so a fit is deterministic. Compute stays in Rust; the wasm exports live in
//! `crates/haa-wasm/src/fit.rs`.
//!
//! - `loss`: multi-resolution STFT + log-mel + RMS envelope (+ f0) loss kernel;
//! - `params`: the fit-able values of a patch (`describe_params`);
//! - `cmaes`: CMA-ES with box constraints, stepped one evaluation at a time;
//! - `mutate`: add/remove one catalog node;
//! - `fit`: the staged search (`Fitter`), stepped by the host;
//! - `spectral`: the spectral judge of pitched instrument stems (spectra, harmonics and envelopes per note).

pub mod cmaes;
pub mod eval;
pub mod fft;
pub mod fit;
pub mod loss;
pub mod mutate;
pub mod params;
pub mod spectral;

pub use fit::{FitOptions, Fitter, fit};
pub use loss::{LossConfig, LossTerms, TargetFeatures, compare, loss};
pub use params::describe_params;
pub use spectral::{NoteSpan, SpectralTarget, SpectralTerms, SpectralWeights, parse_notes};
