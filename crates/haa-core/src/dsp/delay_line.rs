//! Fractional mono delay line. Ported from Sona v11 effects.js.
//! Sona stores effect delays in f32; this shared primitive stores f64.

/// A ring buffer whose read position is measured from the next write position.
pub struct DelayLine {
    buffer: Vec<f64>,
    index: usize,
}

impl DelayLine {
    /// Allocates enough space for the requested maximum delay and interpolation.
    pub fn new(max_seconds: f64, sample_rate: f64) -> Self {
        let len = (max_seconds * sample_rate).ceil().max(0.0) as usize + 2;
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

    /// Reads a linearly interpolated delay before writing the current sample.
    /// Delays are limited to the ring's available history.
    pub fn read(&self, delay_samples: f64) -> f64 {
        let delay = delay_samples.max(0.0).min((self.buffer.len() - 1) as f64);
        let mut pos = self.index as f64 - delay;
        if pos < 0.0 {
            pos += self.buffer.len() as f64;
        }
        let base = pos.floor() as usize;
        let fraction = pos - base as f64;
        self.buffer[base]
            + fraction * (self.buffer[(base + 1) % self.buffer.len()] - self.buffer[base])
    }

    /// Clears the history and returns the write position to the start.
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_returns_after_requested_delay() {
        let mut delay = DelayLine::new(1.0, 1000.0);
        for i in 0..=100 {
            assert_eq!(delay.read(100.0), if i == 100 { 1.0 } else { 0.0 });
            delay.write(if i == 0 { 1.0 } else { 0.0 });
        }
        delay.reset();
        assert_eq!(delay.read(100.0), 0.0);
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let mut delay = DelayLine::new(0.1, 1000.0);
        for _ in 0..200 {
            assert_eq!(delay.read(12.5), 0.0);
            delay.write(0.0);
        }
    }
}
