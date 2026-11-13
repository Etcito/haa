//! Modulated mono delay line with 4-point Hermite interpolation, shared by `chorus` and `flanger`.
//! HAA-only helper (no Sona counterpart): a swept delay read with linear interpolation dulls the
//! highs as the fraction moves, so the modulation effects read through a cubic Hermite kernel.

/// A ring buffer whose read position is measured from the next write position, like `DelayLine`.
pub struct ModDelay {
    buffer: Vec<f64>,
    index: usize,
}

impl ModDelay {
    /// Allocates enough space for `max_seconds` of delay plus the interpolation taps.
    pub fn new(max_seconds: f64, sample_rate: f64) -> Self {
        let len = (max_seconds * sample_rate).ceil().max(0.0) as usize + 5;
        Self {
            buffer: vec![0.0; len],
            index: 0,
        }
    }

    /// Writes a sample and advances the next write position.
    pub fn write(&mut self, x: f64) {
        self.buffer[self.index] = x;
        self.index = (self.index + 1) % self.buffer.len();
    }

    fn tap(&self, delay: usize) -> f64 {
        let len = self.buffer.len();
        self.buffer[(self.index + len - delay % len) % len]
    }

    /// Reads `delay_samples` back (1 = the last written sample), Hermite-interpolated.
    /// The delay is clamped to 2..len-3 so every tap exists.
    pub fn read(&self, delay_samples: f64) -> f64 {
        let max = (self.buffer.len() - 3) as f64;
        let delay = if delay_samples.is_finite() { delay_samples.clamp(2.0, max) } else { 2.0 };
        let whole = delay.floor();
        let frac = delay - whole;
        let i = whole as usize;
        let y0 = self.tap(i - 1);
        let y1 = self.tap(i);
        let y2 = self.tap(i + 1);
        let y3 = self.tap(i + 2);
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * frac + c2) * frac + c1) * frac + y1
    }

    /// Clears the history.
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }
}

/// Sine LFO value for a phase in cycles.
pub fn lfo_sine(phase: f64) -> f64 {
    (core::f64::consts::TAU * phase).sin()
}
