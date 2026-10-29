//! Acoustic space: early reflections plus an 8-line FDN. Shared by `space` and `reverb`.
//! Ported from Sona v11 `effects.js` (`acousticSpace`, lines 107-155, its state in
//! `createEffectState`, and `FDN_TIMES`, `EARLY_TIMES`, `EARLY_GAINS`, `readDelay`).
//!
//! Deviations from Sona:
//! - Sona stores the delay lines in `Float32Array`; here every buffer is f64, like the
//!   rest of `haa-core`.
//! - Sona's `acousticSpace` runs on a state object shared with the other effects; here the
//!   state lives in `AcousticSpace` and every allocation happens in `new`.
//! - Sona writes the early sample at `earlyIndex` and then reads at `earlyIndex - t`.
//!   `DelayLine::read` measures from the next write slot, so after `write` the equivalent
//!   read is `read(t + 1.0)` (marked `// sona:` below).

use crate::dsp::delay_line::DelayLine;

const FDN_TIMES: [f64; 8] = [
    0.0217, 0.0271, 0.0319, 0.0373, 0.0437, 0.0499, 0.0571, 0.0637,
];
const EARLY_TIMES: [f64; 6] = [0.0043, 0.0079, 0.0127, 0.0181, 0.0269, 0.0397];
const EARLY_GAINS: [f64; 6] = [0.38, -0.27, 0.24, 0.19, -0.13, 0.10];
const TAU: f64 = std::f64::consts::TAU;

/// Sona's combined early-reflection and feedback-delay-network reverb.
pub struct AcousticSpace {
    sample_rate: f64,
    lines: [DelayLine; 8],
    early_l: DelayLine,
    early_r: DelayLine,
    damping: [f64; 8],
    reads: [f64; 8],
    delay_samples: [f64; 8],
    feedback: [f64; 8],
    early_samples: [f64; 6],
    distance_l: f64,
    distance_r: f64,
    air_alpha: f64,
    damp_alpha: f64,
    fresh: bool,
}

impl AcousticSpace {
    /// Allocates every buffer from the maximum times. This is the only allocating method.
    pub fn new(sample_rate: f64) -> Self {
        // Sona sizes the FDN lines `ceil(t * 2.25 * sr) + 4` and the early buffers
        // `ceil(.12 * sr) + 8`. DelayLine adds its own interpolation slots, so asking for
        // the Sona maximum plus four samples covers both maxima.
        let lines = std::array::from_fn(|i| {
            DelayLine::new(FDN_TIMES[i] * 2.25 + 4.0 / sample_rate, sample_rate)
        });
        let early_l = DelayLine::new(0.12 + 8.0 / sample_rate, sample_rate);
        let early_r = DelayLine::new(0.12 + 8.0 / sample_rate, sample_rate);
        Self {
            sample_rate,
            lines,
            early_l,
            early_r,
            damping: [0.0; 8],
            reads: [0.0; 8],
            delay_samples: [0.0; 8],
            feedback: [0.0; 8],
            early_samples: [0.0; 6],
            distance_l: 0.0,
            distance_r: 0.0,
            air_alpha: 1.0,
            damp_alpha: 0.3,
            fresh: true,
        }
    }

    /// Processes one stereo sample. `legacy` forces reverb's damping, distance and width.
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        in_l: f64,
        in_r: f64,
        size: f64,
        damping: f64,
        mix: f64,
        width: f64,
        distance: f64,
        frame: u64,
        legacy: bool,
    ) -> (f64, f64) {
        let damping = if legacy { 0.5 } else { damping };
        let distance = if legacy { 0.0 } else { distance };
        let width = if legacy { 0.7 } else { width };
        let sr = self.sample_rate;

        if (frame & 31) == 0 || self.fresh {
            let scale = 0.4 + size * 1.65;
            let rt60 = 0.16 + size * 2.6;
            for i in 0..8 {
                self.delay_samples[i] = FDN_TIMES[i] * scale * sr;
                self.feedback[i] = (-6.907755 * FDN_TIMES[i] * scale / rt60).exp();
            }
            for i in 0..6 {
                self.early_samples[i] = EARLY_TIMES[i] * (0.55 + size * 1.35) * sr;
            }
            self.damp_alpha = 1.0 - (-TAU * (14000.0 * 0.085_f64.powf(damping)) / sr).exp();
            self.air_alpha = 1.0 - (-TAU * (17000.0 / (1.0 + distance * 8.0)) / sr).exp();
            self.fresh = false;
        }

        self.distance_l += self.air_alpha * (in_l - self.distance_l);
        self.distance_r += self.air_alpha * (in_r - self.distance_r);
        let air_blend = distance * 0.85;
        let source_l = in_l * (1.0 - air_blend) + self.distance_l * air_blend;
        let source_r = in_r * (1.0 - air_blend) + self.distance_r * air_blend;

        // sona: Sona writes at `earlyIndex` and reads `readDelay(buf, earlyIndex, t)`,
        // i.e. at `earlyIndex - t`. DelayLine::read measures from the next write slot, so
        // after `write` the same tap is `read(t + 1.0)`.
        self.early_l.write(source_l);
        self.early_r.write(source_r);
        let mut early_l = 0.0;
        let mut early_r = 0.0;
        for i in 0..6 {
            let t = self.early_samples[i];
            let left_line = if i & 1 == 1 {
                &self.early_r
            } else {
                &self.early_l
            };
            let right_line = if i & 1 == 1 {
                &self.early_l
            } else {
                &self.early_r
            };
            early_l += left_line.read(t + 1.0) * EARLY_GAINS[i];
            early_r += right_line.read(t * 1.103 + sr * 0.0013 + 1.0) * EARLY_GAINS[i];
        }

        let mut sum = 0.0;
        let mut late_l = 0.0;
        let mut late_r = 0.0;
        for i in 0..8 {
            let delayed = self.lines[i].read(self.delay_samples[i]);
            self.damping[i] += (delayed - self.damping[i]) * self.damp_alpha;
            self.reads[i] = self.damping[i];
            sum += self.damping[i];
            late_l += delayed * if i & 1 == 1 { -0.25 } else { 0.25 };
            late_r += delayed * if i & 2 == 2 { -0.25 } else { 0.25 };
        }
        // Householder feedback matrix is energy-preserving; each line's loss sets the decay.
        for i in 0..8 {
            let source = if i & 1 == 1 { source_r } else { source_l };
            let sign = if i & 2 == 2 { -0.27 } else { 0.27 };
            let feed = source * sign;
            let value = feed + (self.reads[i] - 0.25 * sum) * self.feedback[i];
            self.lines[i].write(value.clamp(-8.0, 8.0));
        }

        let wet_l = early_l * 0.6 + late_l * 1.45;
        let wet_r = early_r * 0.6 + late_r * 1.45;
        let mid = (wet_l + wet_r) * 0.5;
        let side = (wet_l - wet_r) * 0.5 * width;
        let distance_gain = 1.0 / (1.0 + distance * 1.6);
        let wet_mix = (mix + distance * 0.2).clamp(0.0, 1.0);
        let out_l = (source_l * (1.0 - wet_mix) + (mid + side) * wet_mix) * distance_gain;
        let out_r = (source_r * (1.0 - wet_mix) + (mid - side) * wet_mix) * distance_gain;
        (out_l, out_r)
    }

    /// Clears every buffer and state and forces a coefficient refresh on the next sample.
    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.reset();
        }
        self.early_l.reset();
        self.early_r.reset();
        self.damping = [0.0; 8];
        self.reads = [0.0; 8];
        self.delay_samples = [0.0; 8];
        self.feedback = [0.0; 8];
        self.early_samples = [0.0; 6];
        self.distance_l = 0.0;
        self.distance_r = 0.0;
        self.air_alpha = 1.0;
        self.damp_alpha = 0.3;
        self.fresh = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(state: &mut u32) -> f64 {
        *state ^= *state << 13;
        *state ^= *state >> 17;
        *state ^= *state << 5;
        (*state as f64 / u32::MAX as f64) * 2.0 - 1.0
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let mut space = AcousticSpace::new(48000.0);
        for legacy in [false, true] {
            for frame in 0..4800 {
                let (l, r) = space.process(0.0, 0.0, 0.5, 0.5, 0.5, 0.5, 0.5, frame, legacy);
                assert_eq!((l, r), (0.0, 0.0));
            }
            space.reset();
        }
    }

    #[test]
    fn reverb_tail_decays() {
        // `reverb`: legacy forces damping .5, distance 0 and width .7.
        let mut space = AcousticSpace::new(48000.0);
        let mut early_peak = 0.0_f64;
        let mut late_peak = 0.0_f64;
        let mut overall_peak = 0.0_f64;
        for frame in 0..96000u64 {
            let input = if frame == 0 { 1.0 } else { 0.0 };
            let (l, r) = space.process(input, input, 1.0, 0.5, 1.0, 0.7, 0.0, frame, true);
            let v = l.abs().max(r.abs());
            overall_peak = overall_peak.max(v);
            if (9600..19200).contains(&frame) {
                early_peak = early_peak.max(v);
            }
            if (86400..96000).contains(&frame) {
                late_peak = late_peak.max(v);
            }
        }
        assert!(early_peak > 0.0, "reverb tail is silent");
        // At 48 kHz and size 1 the settled tail is ~3e-3 of the 0.2-0.4 s window and
        // ~3e-4 of the impulse peak: it keeps ringing but 60 dB down.
        assert!(
            late_peak < overall_peak * 1e-3,
            "tail {late_peak} vs peak {overall_peak}"
        );
        assert!(
            late_peak < early_peak * 1e-2,
            "tail {late_peak} vs early {early_peak}"
        );
    }

    #[test]
    fn outputs_stay_finite_at_every_param_extreme() {
        let mut space = AcousticSpace::new(48000.0);
        for size in [0.0, 1.0] {
            for damping in [0.0, 1.0] {
                for width in [0.0, 1.0] {
                    for distance in [0.0, 1.0] {
                        space.reset();
                        let mut seed = 0x9e37_79b9u32;
                        for frame in 0..2400u64 {
                            let l = noise(&mut seed);
                            let r = noise(&mut seed);
                            let (out_l, out_r) = space
                                .process(l, r, size, damping, 1.0, width, distance, frame, false);
                            assert!(out_l.is_finite() && out_r.is_finite());
                        }
                    }
                }
            }
        }
    }
}
