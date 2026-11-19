//! A voice is a handful of graph parameters, not an asset: the blend of the GGUF's two speakers and the `post`
//! edits. `voice.tvv.json` v3 is only their import/export form:
//!
//! ```text
//! {v:3, base:{m2:w, f2:1-w},
//!  post:{warp, tilt_db_oct, env_offset_db[32], breath_db, phonation, f0_shift_st, f0_range_x, rate_x, jitter_x},
//!  origin:{kind:"sample"|"prior", seed, attrs}}
//! ```
//! `w` blends the M2 and F2 rows of both `voices` and `frame_voices` (`w M2 + (1 - w) F2`); it may go a little
//! outside [0, 1]. The `post` edits are applied by [`crate::edit`] and [`crate::Model::render`].

use serde::{Deserialize, Serialize};

/// Number of `env_offset_db` bands: 1/6 octave from 150 Hz (centres `150 * 2^((i + 0.5) / 6)`).
pub const ENV_BANDS: usize = 32;

/// The `post` edits of a voice. Every field absent from JSON is neutral.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Post {
    /// Envelope frequency scale (> 1 = shorter tract).
    pub warp: f32,
    /// Envelope tilt in dB per octave about 1 kHz.
    pub tilt_db_oct: f32,
    /// Envelope offset per 1/6-octave band (dB), held beyond the first and last band; empty = none.
    pub env_offset_db: Vec<f32>,
    /// Level of the noise branch (dB).
    pub breath_db: f32,
    /// H1-H2 change (dB): the harmonic envelope raised at f0 and below, fading to 0 at 2 f0.
    pub phonation: f32,
    /// Oscillator pitch shift (semitones).
    pub f0_shift_st: f32,
    /// Pitch movement around the voiced mean, before the decoder.
    pub f0_range_x: f32,
    /// Speaking rate (durations divided by it).
    pub rate_x: f32,
    /// Oscillator wobble: 0.25 (x - 1) semitones above 1, none at or below.
    pub jitter_x: f32,
}

impl Default for Post {
    fn default() -> Self {
        Post {
            warp: 1.0,
            tilt_db_oct: 0.0,
            env_offset_db: Vec::new(),
            breath_db: 0.0,
            phonation: 0.0,
            f0_shift_st: 0.0,
            f0_range_x: 1.0,
            rate_x: 1.0,
            jitter_x: 1.0,
        }
    }
}

impl Post {
    /// True when the envelope edits change nothing (the head then gets the cepstra untouched).
    pub fn envelope_neutral(&self) -> bool {
        self.warp == 1.0
            && self.tilt_db_oct == 0.0
            && self.breath_db == 0.0
            && self.phonation == 0.0
            && self.env_offset_db.iter().all(|&v| v == 0.0)
    }

    pub fn check(&self) -> Result<(), String> {
        let all = [self.warp, self.tilt_db_oct, self.breath_db, self.phonation, self.f0_shift_st, self.f0_range_x, self.rate_x, self.jitter_x];
        if all.iter().chain(&self.env_offset_db).any(|v| !v.is_finite()) {
            return Err("voice post: a value is not finite".into());
        }
        if !self.env_offset_db.is_empty() && self.env_offset_db.len() != ENV_BANDS {
            return Err(format!("voice post: env_offset_db must have {ENV_BANDS} values"));
        }
        if !(0.25..=4.0).contains(&self.warp) || !(0.25..=4.0).contains(&self.rate_x) || !(0.0..=3.0).contains(&self.f0_range_x) {
            return Err("voice post: warp and rate_x must be 0.25 to 4, f0_range_x 0 to 3".into());
        }
        Ok(())
    }
}

/// `base` of voice.tvv.json v3.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Base {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m2: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f2: Option<f32>,
}

/// `voice.tvv.json` v3 as written.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VoiceFile {
    pub v: u32,
    #[serde(default)]
    pub base: Base,
    #[serde(default)]
    pub post: Post,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<serde_json::Value>,
}

impl VoiceFile {
    /// The M2 weight `w`: `base.m2`, else `1 - base.f2`, else 1 (M2).
    pub fn m2_weight(&self) -> f32 {
        self.base.m2.or(self.base.f2.map(|f| 1.0 - f)).unwrap_or(1.0)
    }

    pub fn parse(text: &str) -> Result<VoiceFile, String> {
        let f: VoiceFile = serde_json::from_str(text).map_err(|e| format!("voice.tvv.json: {e}"))?;
        if f.v != 3 {
            return Err("voice.tvv.json: only v 3 is read".into());
        }
        f.post.check()?;
        Ok(f)
    }

    /// The v3 JSON for a blend weight and post edits.
    pub fn to_json(m2: f32, post: &Post) -> String {
        let f = VoiceFile { v: 3, base: Base { m2: Some(m2), f2: Some(1.0 - m2) }, post: post.clone(), origin: None };
        serde_json::to_string(&f).unwrap_or_default()
    }
}

/// A voice resolved against a model (see [`crate::Model::blend_voice`]).
#[derive(Clone, Debug)]
pub struct Voice {
    /// The M2 weight of the blend (1 = M2, 0 = F2).
    pub m2: f32,
    /// Encoder speaker vector `[channels]`.
    pub voice: Vec<f32>,
    /// Frame transformer speaker vector `[channels]`.
    pub frame_voice: Vec<f32>,
    pub post: Post,
}
