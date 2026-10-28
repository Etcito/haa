// Ported from Sona v11 dsp.js
//! DC blocker of the output mix: `y = x - x_prev + coef * y_prev`, with Sona's `dcCoefficient` from consts.
//! Deviation: one mono instance per channel; Sona keeps the four `dcIn`/`dcOut` values of a stereo mix.

use crate::consts::dc_blocker_coefficient;

/// First-order DC blocker with the 5 Hz cutoff of Sona's `dcCoefficient`.
pub struct DcBlocker {
    coef: f64,
    x_prev: f64,
    y_prev: f64,
}

impl DcBlocker {
    /// A blocker with no history, for a stream at `sample_rate`.
    pub fn new(sample_rate: f64) -> DcBlocker {
        DcBlocker {
            coef: dc_blocker_coefficient(sample_rate),
            x_prev: 0.0,
            y_prev: 0.0,
        }
    }

    /// Filters one sample.
    pub fn process(&mut self, x: f64) -> f64 {
        let y = x - self.x_prev + self.coef * self.y_prev;
        self.x_prev = x;
        self.y_prev = y;
        y
    }

    /// Clears the filter history.
    pub fn reset(&mut self) {
        self.x_prev = 0.0;
        self.y_prev = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_input_decays_to_zero_within_one_second() {
        let mut blocker = DcBlocker::new(48_000.0);
        let mut y = 0.0;
        for _ in 0..48_000 {
            y = blocker.process(1.0);
        }
        assert!(y.abs() < 1e-6, "output after one second: {y}");
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let mut blocker = DcBlocker::new(48_000.0);
        for _ in 0..1_000 {
            assert_eq!(blocker.process(0.0), 0.0);
        }
    }

    #[test]
    fn reset_clears_history() {
        let mut blocker = DcBlocker::new(48_000.0);
        for _ in 0..100 {
            blocker.process(0.8);
        }
        blocker.reset();
        assert_eq!(blocker.process(0.0), 0.0);
    }
}
