//! The master limiter of the sfx mix: the last stage before the device, so that no sum of voices,
//! however many and however loud, ever leaves the mix above `CEILING_DB`.
//!
//! - True peak: every input sample and the 3 points between it and the next (4x oversampling, a
//!   32-tap windowed-sinc polyphase interpolator) are measured, so an inter-sample peak that the
//!   device's reconstruction would turn into clipping is caught too. The short interpolator reads
//!   full-band noise up to 0.15 dB low (against a 65-tap, 16x measurement), so the gain aims
//!   `DETECT_MARGIN_DB` under the ceiling.
//! - Look-ahead: the audio is delayed by `LOOKAHEAD_S` (+ the interpolator's 16 samples). The gain a
//!   sample needs is known before it plays; the gain applied is the box average, over the look-ahead,
//!   of the minimum needed over the look-ahead, so it ramps down ahead of the peak and every value in
//!   the average is already under what the peak needs: no attack overshoot, no distortion.
//! - Release: the gain comes back toward 1 with a `RELEASE_S` time constant (it can only rise toward
//!   the minimum the look-ahead allows).
//! - A final clamp at the ceiling only guards against float rounding: the gain alone keeps it there.
//!
//! It counts its overruns: the input samples whose true peak passed the ceiling (where the gain had to
//! act), the loudest input true peak and the lowest gain, since the host last read them.

/// The ceiling (dBFS, true peak).
pub const CEILING_DB: f64 = -1.0;
/// The detector aims this far under the ceiling (dB): the interpolator's worst under-read and some.
pub const DETECT_MARGIN_DB: f64 = 0.25;
/// The look-ahead (s).
pub const LOOKAHEAD_S: f64 = 0.003;
/// The release time constant (s).
pub const RELEASE_S: f64 = 0.08;

const PHASES: usize = 4;
const TAPS: usize = 32;

/// The interpolator's coefficients: phase k (1..3) gives the point k/4 of the way from the history's
/// sample TAPS/2 - 1 (the oldest is 0) to the next one; Hann-windowed sinc, normalised to unit DC gain.
fn polyphase() -> [[f32; TAPS]; PHASES] {
    let mut h = [[0.0f32; TAPS]; PHASES];
    for (k, row) in h.iter_mut().enumerate() {
        let frac = k as f64 / PHASES as f64;
        let mut sum = 0.0;
        let mut v = [0.0f64; TAPS];
        for (j, c) in v.iter_mut().enumerate() {
            // On the history's axis (tap j at position j, the oldest at 0) the point sits at TAPS/2 - 1 + frac.
            let t = j as f64 - (TAPS as f64 / 2.0 - 1.0) - frac;
            let sinc = if t.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t) };
            let w = 0.5 + 0.5 * (std::f64::consts::PI * t / (TAPS as f64 / 2.0 + 1.0)).cos();
            *c = sinc * w;
            sum += *c;
        }
        for (o, c) in row.iter_mut().zip(v) {
            *o = (c / sum) as f32;
        }
    }
    h
}

/// The stereo master limiter (one instance per mix).
pub struct MasterLimiter {
    ceiling: f32,
    /// What the gain aims at (the ceiling less `DETECT_MARGIN_DB`).
    target: f32,
    /// Look-ahead window, samples (W).
    window: usize,
    /// Audio delay line per channel (W + TAPS/2 - 1 samples) and its write position.
    delay: [Vec<f32>; 2],
    pos: usize,
    /// The last TAPS input samples per channel (index 0 oldest).
    hist: [[f32; TAPS]; 2],
    coef: [[f32; TAPS]; PHASES],
    /// Sliding minimum of the needed gain over the last W steps: (step, gain), increasing gains.
    mins: std::collections::VecDeque<(u64, f32)>,
    step: u64,
    release: f32,
    rel_coef: f32,
    /// Box average of the released gain over the last W steps.
    boxed: Vec<f32>,
    box_pos: usize,
    box_sum: f64,
    /// Since the last `take_stats`: input samples over the ceiling, loudest input true peak, lowest gain.
    over: u32,
    peak_in: f32,
    gain_floor: f32,
    /// Output samples the final clamp had to cut (the gain alone should leave none).
    pub clamped: u32,
}

impl MasterLimiter {
    pub fn new(sample_rate: f64) -> MasterLimiter {
        let window = ((LOOKAHEAD_S * sample_rate).round() as usize).max(1);
        // A need measured at step n is the interval of the sample TAPS/2 steps back; the minimum holds it
        // for W steps and the box average spans W steps, so that sample plays W - 1 steps after its need
        // came in: TAPS/2 + W - 1 steps after it entered.
        let len = window + TAPS / 2 - 1;
        MasterLimiter {
            ceiling: 10f64.powf(CEILING_DB / 20.0) as f32,
            target: 10f64.powf((CEILING_DB - DETECT_MARGIN_DB) / 20.0) as f32,
            window,
            delay: [vec![0.0; len], vec![0.0; len]],
            pos: 0,
            hist: [[0.0; TAPS]; 2],
            coef: polyphase(),
            mins: std::collections::VecDeque::with_capacity(window + 2),
            step: 0,
            release: 1.0,
            rel_coef: (1.0 - (-1.0 / (RELEASE_S * sample_rate)).exp()) as f32,
            boxed: vec![1.0; window],
            box_pos: 0,
            box_sum: window as f64,
            over: 0,
            peak_in: 0.0,
            gain_floor: 1.0,
            clamped: 0,
        }
    }

    /// The latency the limiter adds (samples).
    pub fn latency(&self) -> usize {
        self.delay[0].len()
    }

    /// The true peak of the newest measurable interval of channel `c`: the sample TAPS/2 steps back
    /// (history index TAPS/2 - 1) and the 3 points between it and the next.
    fn true_peak(&self, c: usize) -> f32 {
        let h = &self.hist[c];
        let mut p = h[TAPS / 2 - 1].abs();
        for k in 1..PHASES {
            let mut y = 0.0;
            for (x, w) in h.iter().zip(&self.coef[k]) {
                y += x * w;
            }
            p = p.max(y.abs());
        }
        p
    }

    /// Limits `left` and `right` in place (equal lengths).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let len = self.delay[0].len();
        for i in 0..left.len().min(right.len()) {
            let x = [left[i], right[i]];
            for c in 0..2 {
                self.hist[c].copy_within(1.., 0);
                self.hist[c][TAPS - 1] = if x[c].is_finite() { x[c] } else { 0.0 };
            }
            let tp = self.true_peak(0).max(self.true_peak(1));
            if tp > self.target {
                self.over += 1;
            }
            self.peak_in = self.peak_in.max(tp);
            let need = if tp > self.target { self.target / tp } else { 1.0 };
            // Sliding minimum over the last W needs.
            while self.mins.back().is_some_and(|&(_, g)| g >= need) {
                self.mins.pop_back();
            }
            self.mins.push_back((self.step, need));
            while self.mins.front().is_some_and(|&(s, _)| s + self.window as u64 <= self.step) {
                self.mins.pop_front();
            }
            let m = self.mins.front().map_or(1.0, |&(_, g)| g);
            // Release toward the allowed minimum, never above it.
            self.release = if m < self.release { m } else { self.release + (m - self.release) * self.rel_coef };
            // Box average over the last W released gains.
            self.box_sum += self.release as f64 - self.boxed[self.box_pos] as f64;
            self.boxed[self.box_pos] = self.release;
            self.box_pos = (self.box_pos + 1) % self.window;
            let g = ((self.box_sum / self.window as f64) as f32).min(1.0);
            self.gain_floor = self.gain_floor.min(g);
            // The delayed sample: len steps ago (the interval measured TAPS/2 - 1 steps after it came in,
            // the minimum covering it for W steps from then).
            for (c, out) in [&mut left[i], &mut right[i]].into_iter().enumerate() {
                let d = self.delay[c][self.pos];
                self.delay[c][self.pos] = x[c];
                let y = d * g;
                if y.abs() > self.ceiling {
                    self.clamped += 1;
                }
                *out = y.clamp(-self.ceiling, self.ceiling);
            }
            self.pos = (self.pos + 1) % len;
            self.step += 1;
        }
    }

    /// (input samples over the ceiling, loudest input true peak dBFS, lowest gain dB) since the last
    /// call, then reset.
    pub fn take_stats(&mut self) -> (u32, f32, f32) {
        let s = (self.over, 20.0 * self.peak_in.max(1e-9).log10(), 20.0 * self.gain_floor.max(1e-9).log10());
        self.over = 0;
        self.peak_in = 0.0;
        self.gain_floor = 1.0;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// True peak of a signal (16x oversampled with a long windowed sinc), for checking only.
    fn measured_true_peak(x: &[f32]) -> f32 {
        let (over, half) = (16usize, 32isize);
        let mut p = 0.0f32;
        for n in 0..x.len() as isize {
            for k in 0..over {
                let t = n as f64 + k as f64 / over as f64;
                let mut y = 0.0;
                for j in (n - half)..=(n + half) {
                    if j < 0 || j >= x.len() as isize {
                        continue;
                    }
                    let d = t - j as f64;
                    let s = if d.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * d).sin() / (std::f64::consts::PI * d) };
                    let w = 0.5 + 0.5 * (std::f64::consts::PI * d / (half as f64 + 1.0)).cos();
                    y += x[j as usize] as f64 * s * w;
                }
                p = p.max(y.abs() as f32);
            }
        }
        p
    }

    #[test]
    fn twenty_voices_at_their_own_limit_never_pass_minus_one_dbfs() {
        let sr = 48_000.0;
        let n = 9_600;
        let mut seed = 3u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f64 / 16_777_216.0
        };
        // 20 voices, each peaking at the voice limiter's 0.92: decaying tones, clicks and noise bursts at
        // random onsets, all in phase at some moments.
        let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
        for v in 0..20 {
            let (f, onset, pan) = (60.0 + 4000.0 * rnd(), (rnd() * 0.1 * sr) as usize, rnd());
            let mut voice = vec![0.0f64; n];
            for (i, s) in voice.iter_mut().enumerate().skip(onset) {
                let t = (i - onset) as f64 / sr;
                *s = if v % 3 == 0 { (rnd() * 2.0 - 1.0) * (-t / 0.03).exp() } else { (std::f64::consts::TAU * f * t).sin() * (-t / 0.2).exp() };
            }
            let peak = voice.iter().fold(0.0f64, |a, x| a.max(x.abs()));
            for i in 0..n {
                let s = voice[i] / peak * 0.92;
                l[i] += (s * (1.0 - pan).sqrt()) as f32;
                r[i] += (s * pan.sqrt()) as f32;
            }
        }
        assert!(l.iter().chain(&r).fold(0.0f32, |a, x| a.max(x.abs())) > 4.0, "the sum is far over full scale");
        let mut lim = MasterLimiter::new(sr);
        for (bl, br) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
            lim.process(bl, br);
        }
        let ceiling = 10f32.powf(CEILING_DB as f32 / 20.0);
        let sample_peak = l.iter().chain(&r).fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(sample_peak <= ceiling + 1e-6, "sample peak {sample_peak}");
        assert_eq!(lim.clamped, 0, "the gain alone keeps the samples under the ceiling");
        let tp = measured_true_peak(&l).max(measured_true_peak(&r));
        let (over, peak_in, _) = lim.take_stats();
        assert!(20.0 * tp.log10() <= CEILING_DB as f32 + 0.1, "true peak {} dBFS", 20.0 * tp.log10());
        assert!(over > 0 && peak_in > 12.0, "the overruns are counted: {over}, {peak_in} dBFS");
    }

    #[test]
    fn a_mix_under_the_ceiling_passes_untouched_after_the_look_ahead() {
        let sr = 48_000.0;
        let x: Vec<f32> = (0..4_800).map(|i| 0.8 * (std::f32::consts::TAU * 440.0 * i as f32 / sr as f32).sin()).collect();
        let (mut l, mut r) = (x.clone(), x.clone());
        let mut lim = MasterLimiter::new(sr);
        lim.process(&mut l, &mut r);
        let d = lim.latency();
        for i in d..x.len() {
            assert!((l[i] - x[i - d]).abs() < 1e-6, "sample {i}: {} vs {}", l[i], x[i - d]);
        }
        assert_eq!(lim.take_stats().0, 0);
    }
}
