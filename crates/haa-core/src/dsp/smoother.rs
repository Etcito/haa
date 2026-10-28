// Ported from Sona v11 dsp.js
//! One-pole smoother of parameters, edge amounts and inputs: `value += (target - value) * (1 - coef)` per sample.
//! Deviation: the coefficient `exp(-1 / (time_s * sr))` is derived from a time in seconds, and time 0 jumps.

/// One-pole smoother that glides toward a target with a time constant in seconds.
pub struct Smoother {
    value: f64,
    target: f64,
    coef: f64,
}

impl Smoother {
    /// A smoother starting at `initial`, with time constant `time_s` at `sample_rate`.
    pub fn new(time_s: f64, sample_rate: f64, initial: f64) -> Smoother {
        Smoother {
            value: initial,
            target: initial,
            coef: coefficient(time_s, sample_rate),
        }
    }

    /// Changes the time constant; the value and the target are kept.
    pub fn set_time(&mut self, time_s: f64, sample_rate: f64) {
        self.coef = coefficient(time_s, sample_rate);
    }

    /// Sets the value the smoother glides toward.
    pub fn set_target(&mut self, v: f64) {
        self.target = v;
    }

    /// Advances one sample toward the target and returns the new value.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        if self.coef == 0.0 {
            self.value = self.target;
        } else {
            self.value += (self.target - self.value) * (1.0 - self.coef);
        }
        self.value
    }

    /// Sets the value and the target at once, without gliding.
    pub fn jump(&mut self, v: f64) {
        self.value = v;
        self.target = v;
    }

    /// The current value.
    pub fn value(&self) -> f64 {
        self.value
    }
}

fn coefficient(time_s: f64, sample_rate: f64) -> f64 {
    if time_s > 0.0 {
        (-1.0 / (time_s * sample_rate)).exp()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaches_63_percent_after_one_time_constant() {
        let mut smoother = Smoother::new(0.005, 48_000.0, 0.0);
        smoother.set_target(1.0);
        let one_time_constant = (0.005 * 48_000.0_f64).round() as usize;
        let mut value = 0.0;
        for _ in 0..one_time_constant {
            value = smoother.next();
        }
        assert!(
            (value - 0.63).abs() <= 0.01,
            "value after one time constant: {value}"
        );
    }

    #[test]
    fn zero_time_jumps_to_target() {
        let mut smoother = Smoother::new(0.0, 48_000.0, 0.25);
        smoother.set_target(0.75);
        assert_eq!(smoother.next(), 0.75);
        smoother.jump(-1.0);
        assert_eq!(smoother.value(), -1.0);
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let mut smoother = Smoother::new(0.005, 48_000.0, 0.0);
        for _ in 0..1_000 {
            assert_eq!(smoother.next(), 0.0);
        }
    }
}
