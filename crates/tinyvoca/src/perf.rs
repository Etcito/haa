//! `perf.json` v1, the performance between the edit stages and the runtime:
//!
//! ```text
//! {v, rate:24000, hop:256, tokens[], stress[], durations[],
//!  log_f0[T] (ln f0/200), voiced[T], energy[T], spans:[{from,to,emotion,level,emph}]}
//! ```
//! `T` is the sum of `durations`. `voiced` is a logit or 0/1 (> 0 is voiced). `spans` is carried, not read here.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Perf {
    #[serde(default = "one")]
    pub v: u32,
    #[serde(default = "rate")]
    pub rate: u32,
    #[serde(default = "hop")]
    pub hop: u32,
    pub tokens: Vec<u32>,
    pub stress: Vec<u32>,
    pub durations: Vec<u32>,
    pub log_f0: Vec<f32>,
    pub voiced: Vec<f32>,
    pub energy: Vec<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spans: Option<serde_json::Value>,
}

fn one() -> u32 {
    1
}
fn rate() -> u32 {
    24_000
}
fn hop() -> u32 {
    256
}

impl Perf {
    pub fn parse(text: &str) -> Result<Perf, String> {
        serde_json::from_str(text).map_err(|e| format!("perf.json: {e}"))
    }

    /// Checks the version, the rate and hop against the model and that every array agrees.
    pub fn check(&self, model: &crate::Model) -> Result<(), String> {
        if self.v != 1 {
            return Err("perf.json: only v 1 is read".into());
        }
        if self.rate as usize != model.config.sample_rate || self.hop as usize != model.config.hop {
            return Err(format!("perf.json: rate/hop must be {}/{}", model.config.sample_rate, model.config.hop));
        }
        let n = self.tokens.len();
        if self.stress.len() != n || self.durations.len() != n {
            return Err("perf.json: tokens, stress and durations must have the same length".into());
        }
        let frames: usize = self.durations.iter().map(|&d| d as usize).sum();
        if self.log_f0.len() != frames || self.voiced.len() != frames || self.energy.len() != frames {
            return Err(format!("perf.json: log_f0, voiced and energy must have sum(durations) = {frames} values"));
        }
        if self.log_f0.iter().chain(&self.voiced).chain(&self.energy).any(|v| !v.is_finite()) {
            return Err("perf.json: a curve value is not finite".into());
        }
        Ok(())
    }
}
