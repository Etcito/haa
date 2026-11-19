//! The voice's `post` edits (voice.tvv.json v3) on the stage arrays.
//!
//! - Envelope (`warp`, `tilt_db_oct`, `env_offset_db`, `breath_db`, `phonation`): [`Envelope::apply`] on the head's
//!   cepstra. Port of `warp_cepstrum` in the offline `tv/tvd.py` (its fixed version): the log magnitude is read only
//!   at the head's own FFT bins, causal plus anticausal part (between the bins the 80-term series swings far, and a
//!   warp read there blew the level up by 50 dB), clamped like the head does, held flat above the harmonic limit,
//!   then frequency-scaled, tilted, offset, and fitted back by least squares onto a causal cepstrum of the same
//!   length (the anticausal part is kept, so the causal part carries the whole change). Per frame, on the raw head
//!   outputs: the change is computed on the liftered values and divided back by the lifter.
//! - Curves: [`retime`] (`rate_x` on given durations and curves), [`crate::widen_pitch`] (`f0_range_x`),
//!   [`oscillator_pitch`] (`f0_shift_st`, `jitter_x`, oscillator only, like model.py's `pitch_shift`).

use crate::det::Det;
use crate::head::gaussian_noise;
use crate::model::{Cepstra, Config};
use crate::voice::{ENV_BANDS, Post};
use std::f64::consts::{LN_10, LN_2, PI};

fn lifter(n: usize) -> f64 {
    if n == 0 { 2.0 } else { 1.0 / (1.0 + n as f64 / 4.0) }
}

/// One branch's fixed tables: the cosine basis at the FFT bins and its least-squares inverse.
struct Branch {
    bins: usize,
    order: usize,
    /// `basis[k * order + l] = cos(w_k l)`.
    basis: Vec<f64>,
    /// `(B^T B)^-1 B^T`, `[order x bins]`.
    inverse: Vec<f64>,
    /// First bin held flat (above `harmonic_limit` of the rate).
    top: usize,
    /// Octaves from 1 kHz of each bin (50 Hz floor), for the tilt.
    octaves: Vec<f64>,
    hz: Vec<f64>,
}

impl Branch {
    fn new(fft: usize, order: usize, sample_rate: f64, limit: f64) -> Branch {
        let bins = fft / 2 + 1;
        let w: Vec<f64> = (0..bins).map(|k| PI * k as f64 / (bins - 1) as f64).collect();
        let basis: Vec<f64> = w.iter().flat_map(|&wk| (0..order).map(move |l| (wk * l as f64).dcos())).collect();
        // normal equations, solved by Cholesky
        let mut gram = vec![0.0; order * order];
        for k in 0..bins {
            for a in 0..order {
                for b in 0..order {
                    gram[a * order + b] += basis[k * order + a] * basis[k * order + b];
                }
            }
        }
        let mut chol = vec![0.0; order * order];
        for i in 0..order {
            for j in 0..=i {
                let mut s = gram[i * order + j];
                for p in 0..j {
                    s -= chol[i * order + p] * chol[j * order + p];
                }
                chol[i * order + j] = if i == j { s.max(1e-12).sqrt() } else { s / chol[j * order + j] };
            }
        }
        let mut inverse = vec![0.0; order * bins];
        let mut y = vec![0.0; order];
        for k in 0..bins {
            // solve (L L^T) x = B[k]^T
            for i in 0..order {
                let mut s = basis[k * order + i];
                for p in 0..i {
                    s -= chol[i * order + p] * y[p];
                }
                y[i] = s / chol[i * order + i];
            }
            for i in (0..order).rev() {
                let mut s = y[i];
                for p in i + 1..order {
                    s -= chol[p * order + i] * inverse[p * bins + k];
                }
                inverse[i * bins + k] = s / chol[i * order + i];
            }
        }
        let hz: Vec<f64> = w.iter().map(|&wk| wk / PI * sample_rate / 2.0).collect();
        let octaves = hz.iter().map(|&f| (f.max(50.0) / 1000.0).dlog2()).collect();
        let top = (limit * 2.0 * (bins - 1) as f64) as usize;
        Branch { bins, order, basis, inverse, top, octaves, hz }
    }

    /// The edited liftered causal cepstrum for `causal` (liftered, gain included) and the anticausal magnitude.
    fn edit(&self, causal: &[f64], anti_mag: &[f64], warp: f64, shape: &[f64], out: &mut [f64]) {
        let (n, l) = (self.bins, self.order);
        let mut own = vec![0.0; n];
        for k in 0..n {
            let row = &self.basis[k * l..(k + 1) * l];
            let s: f64 = causal.iter().zip(row).map(|(c, b)| c * b).sum();
            own[k] = (s + anti_mag[k]).clamp(-14.0, 6.0);
        }
        for k in self.top.max(1)..n {
            own[k] = own[self.top.max(1) - 1];
        }
        let mut logmag = vec![0.0; n];
        for k in 0..n {
            let wk = PI * k as f64 / (n - 1) as f64;
            let pos = (wk / warp).clamp(0.0, PI) / PI * (n - 1) as f64;
            let i0 = (pos.floor() as usize).min(n - 2);
            let fr = pos - i0 as f64;
            logmag[k] = own[i0] * (1.0 - fr) + own[i0 + 1] * fr + shape[k] - anti_mag[k];
        }
        for (i, o) in out.iter_mut().enumerate().take(l) {
            *o = self.inverse[i * n..(i + 1) * n].iter().zip(&logmag).map(|(a, b)| a * b).sum();
        }
    }
}

/// The envelope edit tables for one model (build once, reuse for every utterance).
pub struct Envelope {
    harmonic: Branch,
    noise: Branch,
    cep_causal: usize,
    cep_anti: usize,
    noise_cep: usize,
    h_steps: usize,
    g_steps: usize,
}

/// `env_offset_db` at `hz`: linear in log frequency between band centres, held beyond (np.interp).
fn offset_at(offset: &[f32], hz: f64) -> f64 {
    let x = hz.max(1.0).dlog2();
    let centre = |i: usize| (150.0 * 2f64.dpowf((i as f64 + 0.5) / 6.0)).dlog2();
    if x <= centre(0) {
        return offset[0] as f64;
    }
    for i in 1..ENV_BANDS {
        if x <= centre(i) {
            let u = (x - centre(i - 1)) / (centre(i) - centre(i - 1));
            return offset[i - 1] as f64 * (1.0 - u) + offset[i] as f64 * u;
        }
    }
    offset[ENV_BANDS - 1] as f64
}

impl Envelope {
    pub fn new(c: &Config) -> Envelope {
        Envelope {
            harmonic: Branch::new(c.synth_fft, c.cep_causal, c.sample_rate as f64, c.harmonic_limit),
            noise: Branch::new(c.noise_fft, c.noise_cep, c.sample_rate as f64, c.harmonic_limit),
            cep_causal: c.cep_causal,
            cep_anti: c.cep_anti,
            noise_cep: c.noise_cep,
            h_steps: c.hop / c.synth_hop,
            g_steps: c.hop / c.noise_hop,
        }
    }

    /// Applies the envelope edits of `post` to `cepstra` in place. `f0` (Hz per frame) places the phonation edit.
    pub fn apply(&self, cepstra: &mut Cepstra, f0: &[f32], post: &Post) {
        if post.envelope_neutral() {
            return;
        }
        let db = LN_10 / 20.0;
        let warp = post.warp as f64;
        let frames = f0.len();
        // harmonic branch
        let (hb, hw) = (&self.harmonic, self.cep_causal + self.cep_anti + self.h_steps);
        let fixed: Vec<f64> = (0..hb.bins)
            .map(|k| {
                let off = if post.env_offset_db.is_empty() { 0.0 } else { offset_at(&post.env_offset_db, hb.hz[k]) };
                (post.tilt_db_oct as f64 * hb.octaves[k] + off) * db
            })
            .collect();
        let mut shape = vec![0.0; hb.bins];
        let (mut causal, mut anti_mag, mut out) = (vec![0.0; hb.order], vec![0.0; hb.bins], vec![0.0; hb.order]);
        for t in 0..frames {
            let row = &mut cepstra.harmonic[t * hw..(t + 1) * hw];
            for i in 0..self.cep_causal {
                causal[i] = row[i] as f64 * lifter(i);
            }
            let gain = (0..self.h_steps).map(|s| row[self.cep_causal + self.cep_anti + s] as f64).sum::<f64>() / self.h_steps as f64;
            causal[0] += gain;
            for k in 0..hb.bins {
                let wk = PI * k as f64 / (hb.bins - 1) as f64;
                anti_mag[k] = (0..self.cep_anti).map(|j| row[self.cep_causal + j] as f64 * lifter(j + 1) * (wk * (j + 1) as f64).dcos()).sum();
                let phon = if post.phonation != 0.0 {
                    let above = (hb.hz[k].max(1.0) / (f0[t] as f64).max(1.0)).dlog2();
                    post.phonation as f64 * (1.0 - above).clamp(0.0, 1.0) * db
                } else {
                    0.0
                };
                shape[k] = fixed[k] + phon;
            }
            hb.edit(&causal, &anti_mag, warp, &shape, &mut out);
            for i in 0..self.cep_causal {
                row[i] += ((out[i] - causal[i]) / lifter(i)) as f32;
            }
        }
        // noise branch
        let (nb, gw) = (&self.noise, self.noise_cep + self.g_steps);
        let shape: Vec<f64> = (0..nb.bins)
            .map(|k| {
                let off = if post.env_offset_db.is_empty() { 0.0 } else { offset_at(&post.env_offset_db, nb.hz[k]) };
                (post.tilt_db_oct as f64 * nb.octaves[k] + post.breath_db as f64 + off) * db
            })
            .collect();
        let zero = vec![0.0; nb.bins];
        let (mut causal, mut out) = (vec![0.0; nb.order], vec![0.0; nb.order]);
        for t in 0..frames {
            let row = &mut cepstra.noise[t * gw..(t + 1) * gw];
            for i in 0..self.noise_cep {
                causal[i] = row[i] as f64 * lifter(i);
            }
            causal[0] += (0..self.g_steps).map(|s| row[self.noise_cep + s] as f64).sum::<f64>() / self.g_steps as f64;
            nb.edit(&causal, &zero, warp, &shape, &mut out);
            for i in 0..self.noise_cep {
                row[i] += ((out[i] - causal[i]) / lifter(i)) as f32;
            }
        }
    }
}

/// `rate_x` on a given performance: each token's duration becomes `round(d / rate)` (at least 1 if it had frames)
/// and the per-frame curves are resampled inside each token (nearest frame). Returns the new durations and curves.
pub fn retime(durations: &[u32], curves: &[&[f32]], rate: f32) -> (Vec<u32>, Vec<Vec<f32>>) {
    let new: Vec<u32> = durations
        .iter()
        .map(|&d| if d == 0 { 0 } else { ((d as f32 / rate).round() as u32).max(1) })
        .collect();
    let mut out: Vec<Vec<f32>> = curves.iter().map(|_| Vec::with_capacity(new.iter().sum::<u32>() as usize)).collect();
    let mut start = 0usize;
    for (&old, &now) in durations.iter().zip(&new) {
        for k in 0..now {
            let src = start + (((k as f32 + 0.5) * old as f32 / now as f32) as usize).min(old as usize - 1);
            for (o, c) in out.iter_mut().zip(curves) {
                o.push(c[src]);
            }
        }
        start += old as usize;
    }
    (new, out)
}

/// The oscillator's ln-f0: `log_f0` plus `f0_shift_st` and, for `jitter_x > 1`, a wobble of 0.25 (x - 1)
/// semitones ([`crate::pitch_wobble`] on deterministic draws from `seed`).
pub fn oscillator_pitch(log_f0: &[f32], post: &Post, extra_shift: f32, seed: u32) -> Vec<f32> {
    let shift = extra_shift + (post.f0_shift_st as f64 * LN_2 / 12.0) as f32;
    let mut out: Vec<f32> = log_f0.iter().map(|&p| p + shift).collect();
    let wobble = 0.25 * (post.jitter_x - 1.0).max(0.0);
    if wobble > 0.0 && !out.is_empty() {
        let draws = gaussian_noise(2 * out.len() + 16, seed.wrapping_add(7919));
        let (fast, slow) = draws.split_at(out.len());
        let w = crate::pitch_wobble(fast, slow, wobble);
        for (o, d) in out.iter_mut().zip(w) {
            *o += d;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::VoiceFile;

    /// voice.tvv.json v3: absent fields are neutral, `f2` alone gives `w`, a wrong band count is refused; `rate_x`
    /// retimes durations and curves token by token.
    #[test]
    fn voice_v3_defaults_and_retime() {
        let f = VoiceFile::parse(r#"{"v":3,"base":{"f2":0.25},"post":{"warp":1.1}}"#).unwrap();
        assert_eq!(f.m2_weight(), 0.75);
        assert_eq!(f.post, Post { warp: 1.1, ..Post::default() });
        assert!(!f.post.envelope_neutral() && Post::default().envelope_neutral());
        assert!(VoiceFile::parse(r#"{"v":3,"post":{"env_offset_db":[1,2]}}"#).is_err());
        assert!(VoiceFile::parse(r#"{"v":2,"z":[]}"#).is_err());

        let curve = [0.0, 1.0, 2.0, 3.0, 10.0, 11.0];
        let (d, c) = retime(&[4, 0, 2], &[&curve], 0.5);
        assert_eq!(d, vec![8, 0, 4]);
        assert_eq!(c[0], vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 10.0, 10.0, 11.0, 11.0]);
    }
}
