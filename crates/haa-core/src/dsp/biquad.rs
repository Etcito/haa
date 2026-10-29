//! Mono RBJ biquad filter. Ported from Sona v11 effects.js.
//! Sona implements bandpass and EQ filters; the other kinds use the same RBJ cookbook.

use core::f64::consts::{PI, SQRT_2};

/// The response shape of a biquad filter.
#[derive(Clone, Copy, Debug)]
pub enum BiquadKind {
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
    Peaking,
    Lowshelf,
    Highshelf,
    Allpass,
}

/// A mono transposed direct-form II biquad.
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    s1: f64,
    s2: f64,
}

impl Default for Biquad {
    fn default() -> Self {
        Self::new()
    }
}

impl Biquad {
    /// Creates a unity filter with empty state.
    pub fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// Sets normalized RBJ coefficients while preserving the filter state.
    pub fn set(&mut self, kind: BiquadKind, freq_hz: f64, q: f64, gain_db: f64, sample_rate: f64) {
        // Sona's bandpass uses 30 Hz; its equalizer uses 20 Hz.
        let min_freq = if matches!(kind, BiquadKind::Bandpass) {
            30.0
        } else {
            20.0
        };
        let w = 2.0 * PI * freq_hz.max(min_freq).min(sample_rate * 0.43) / sample_rate;
        let c = w.cos();
        let sin = w.sin();
        let alpha = sin / (2.0 * q);
        let a = 10.0_f64.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            BiquadKind::Lowpass => (
                (1.0 - c) / 2.0,
                1.0 - c,
                (1.0 - c) / 2.0,
                1.0 + alpha,
                -2.0 * c,
                1.0 - alpha,
            ),
            BiquadKind::Highpass => (
                (1.0 + c) / 2.0,
                -(1.0 + c),
                (1.0 + c) / 2.0,
                1.0 + alpha,
                -2.0 * c,
                1.0 - alpha,
            ),
            BiquadKind::Bandpass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha),
            BiquadKind::Notch => (1.0, -2.0 * c, 1.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha),
            BiquadKind::Peaking => (
                1.0 + alpha * a,
                -2.0 * c,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * c,
                1.0 - alpha / a,
            ),
            BiquadKind::Allpass => (
                1.0 - alpha,
                -2.0 * c,
                1.0 + alpha,
                1.0 + alpha,
                -2.0 * c,
                1.0 - alpha,
            ),
            BiquadKind::Lowshelf | BiquadKind::Highshelf => {
                // Sona's shelves use the cookbook's fixed unit slope.
                let t = 2.0 * a.sqrt() * (sin / SQRT_2);
                if matches!(kind, BiquadKind::Lowshelf) {
                    (
                        a * ((a + 1.0) - (a - 1.0) * c + t),
                        2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                        a * ((a + 1.0) - (a - 1.0) * c - t),
                        (a + 1.0) + (a - 1.0) * c + t,
                        -2.0 * ((a - 1.0) + (a + 1.0) * c),
                        (a + 1.0) + (a - 1.0) * c - t,
                    )
                } else {
                    (
                        a * ((a + 1.0) + (a - 1.0) * c + t),
                        -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                        a * ((a + 1.0) + (a - 1.0) * c - t),
                        (a + 1.0) - (a - 1.0) * c + t,
                        2.0 * ((a - 1.0) - (a + 1.0) * c),
                        (a + 1.0) - (a - 1.0) * c - t,
                    )
                }
            }
        };
        let norm = 1.0 / a0;
        self.b0 = b0 * norm;
        self.b1 = b1 * norm;
        self.b2 = b2 * norm;
        self.a1 = a1 * norm;
        self.a2 = a2 * norm;
    }

    /// Processes one sample without allocating.
    pub fn process(&mut self, x: f64) -> f64 {
        let y = x * self.b0 + self.s1;
        self.s1 = x * self.b1 - self.a1 * y + self.s2;
        self.s2 = x * self.b2 - self.a2 * y;
        y
    }

    /// Clears the delay state, retaining coefficients.
    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowpass_passes_dc_and_attenuates_near_nyquist() {
        let mut filter = Biquad::new();
        filter.set(BiquadKind::Lowpass, 1000.0, 0.707, 0.0, 48_000.0);
        let mut y = 0.0;
        for _ in 0..4800 {
            y = filter.process(1.0);
        }
        assert!((y - 1.0).abs() < 0.01);
        filter.reset();
        let mut energy = 0.0;
        for i in 0..4800 {
            let x = (2.0 * PI * 23_000.0 * i as f64 / 48_000.0).sin();
            let y = filter.process(x);
            if i >= 2400 {
                energy += y * y;
            }
        }
        assert!((energy / 1200.0).sqrt() < 0.1);
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let mut filter = Biquad::new();
        for kind in [
            BiquadKind::Lowpass,
            BiquadKind::Highpass,
            BiquadKind::Bandpass,
            BiquadKind::Notch,
            BiquadKind::Peaking,
            BiquadKind::Lowshelf,
            BiquadKind::Highshelf,
            BiquadKind::Allpass,
        ] {
            filter.set(kind, 1000.0, 0.7, 6.0, 48_000.0);
            for _ in 0..100 {
                assert_eq!(filter.process(0.0), 0.0);
            }
        }
    }
}
