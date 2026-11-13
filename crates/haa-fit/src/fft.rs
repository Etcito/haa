//! Radix-2 FFT for the loss kernel: an in-place iterative complex transform with a precomputed plan.
//! A real frame of `n` samples is packed as `n/2` complex values and unpacked into `n/2 + 1` bins.

use core::f64::consts::PI;

/// A precomputed plan for real frames of one power-of-two size.
pub struct RealFft {
    n: usize,
    half: usize,
    /// Bit-reversal permutation of the `half`-point complex FFT.
    rev: Vec<usize>,
    /// Twiddles of every butterfly stage, stage after stage: (cos, sin) of −2πk/size, k < size/2.
    stage_tw: Vec<(f64, f64)>,
    /// Unpack twiddles: (cos, sin) of −2πk/n, k in 0..half.
    unpack: Vec<(f64, f64)>,
    re: Vec<f64>,
    im: Vec<f64>,
}

impl RealFft {
    /// A plan for real frames of `n` samples; `n` must be a power of two and at least 4.
    pub fn new(n: usize) -> RealFft {
        assert!(n.is_power_of_two() && n >= 4, "fft size must be a power of two >= 4");
        let half = n / 2;
        let bits = half.trailing_zeros();
        let rev = (0..half)
            .map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - bits) })
            .collect();
        let mut stage_tw = Vec::with_capacity(half);
        let mut size = 2;
        while size <= half {
            for k in 0..size / 2 {
                let a = -2.0 * PI * k as f64 / size as f64;
                stage_tw.push((a.cos(), a.sin()));
            }
            size *= 2;
        }
        let unpack = (0..half)
            .map(|k| {
                let a = -2.0 * PI * k as f64 / n as f64;
                (a.cos(), a.sin())
            })
            .collect();
        RealFft { n, half, rev, stage_tw, unpack, re: vec![0.0; half], im: vec![0.0; half] }
    }

    /// Frame size in samples.
    pub fn len(&self) -> usize {
        self.n
    }

    /// Writes the `n/2 + 1` bin magnitudes of the real `frame` into `out`.
    pub fn magnitudes(&mut self, frame: &[f64], out: &mut [f64]) {
        self.forward(frame);
        let half = self.half;
        let out = &mut out[..=half];
        out[0] = (self.re[0] + self.im[0]).abs();
        out[half] = (self.re[0] - self.im[0]).abs();
        for k in 1..half {
            let (re, im) = self.bin(k);
            out[k] = (re * re + im * im).sqrt();
        }
    }

    /// Writes the `n/2 + 1` complex bins of the real `frame` into `re`/`im`.
    pub fn spectrum(&mut self, frame: &[f64], re: &mut [f64], im: &mut [f64]) {
        self.forward(frame);
        let half = self.half;
        re[0] = self.re[0] + self.im[0];
        im[0] = 0.0;
        re[half] = self.re[0] - self.im[0];
        im[half] = 0.0;
        for k in 1..half {
            let (r, i) = self.bin(k);
            re[k] = r;
            im[k] = i;
        }
    }

    fn forward(&mut self, frame: &[f64]) {
        let half = self.half;
        let frame = &frame[..2 * half];
        for (i, &j) in self.rev.iter().enumerate() {
            self.re[j] = frame[2 * i];
            self.im[j] = frame[2 * i + 1];
        }
        let mut size = 2;
        let mut off = 0;
        while size <= half {
            let h = size / 2;
            let tw = &self.stage_tw[off..off + h];
            off += h;
            for (rc, ic) in self.re.chunks_exact_mut(size).zip(self.im.chunks_exact_mut(size)) {
                let (ra, rb) = rc.split_at_mut(h);
                let (ia, ib) = ic.split_at_mut(h);
                for k in 0..h {
                    let (c, s) = tw[k];
                    let tr = rb[k] * c - ib[k] * s;
                    let ti = rb[k] * s + ib[k] * c;
                    rb[k] = ra[k] - tr;
                    ib[k] = ia[k] - ti;
                    ra[k] += tr;
                    ia[k] += ti;
                }
            }
            size *= 2;
        }
    }

    /// Bin k (1..n/2) of the real transform from the packed half-size transform.
    fn bin(&self, k: usize) -> (f64, f64) {
        let m = self.half - k;
        let (zr, zi) = (self.re[k], self.im[k]);
        let (cr, ci) = (self.re[m], -self.im[m]);
        let er = 0.5 * (zr + cr);
        let ei = 0.5 * (zi + ci);
        let or = 0.5 * (zi - ci);
        let oi = -0.5 * (zr - cr);
        let (c, s) = self.unpack[k];
        (er + c * or - s * oi, ei + c * oi + s * or)
    }
}

/// A periodic Hann window of `n` samples.
pub fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()).collect()
}
