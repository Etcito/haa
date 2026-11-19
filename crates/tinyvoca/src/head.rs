//! The source-filter head: a band-limited pulse train from f0 and a noise source, each shaped by its time-varying
//! complex-cepstrum filter in the STFT domain. Port of `pulseTrain`, `filtered` and `gaussianNoise` in tiny-voca
//! `js/tiny-voca.js` (itself checked against `SourceFilter.forward`, model.py line 207). Float64 inside, like both.

use std::f64::consts::PI;

struct Fft {
    size: usize,
    cos: Vec<f64>,
    sin: Vec<f64>,
}

impl Fft {
    fn new(size: usize) -> Self {
        let cos = (0..size / 2).map(|k| (-2.0 * PI * k as f64 / size as f64).cos()).collect();
        let sin = (0..size / 2).map(|k| (-2.0 * PI * k as f64 / size as f64).sin()).collect();
        Fft { size, cos, sin }
    }

    fn run(&self, re: &mut [f64], im: &mut [f64]) {
        let size = self.size;
        let mut j = 0;
        for i in 1..size {
            let mut bit = size >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j ^= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut span = 2;
        while span <= size {
            let step = size / span;
            let halfspan = span / 2;
            for k in 0..halfspan {
                let (c, s) = (self.cos[k * step], self.sin[k * step]);
                let mut a = k;
                while a < size {
                    let b = a + halfspan;
                    let x = re[b] * c - im[b] * s;
                    let y = re[b] * s + im[b] * c;
                    re[b] = re[a] - x;
                    im[b] = im[a] - y;
                    re[a] += x;
                    im[a] += y;
                    a += span;
                }
            }
            span *= 2;
        }
    }
}

/// Filters `source` frame by frame (STFT `size`/`hop`, Hann window, centred, half a window of zeros after the end),
/// with `cepstrum(frame, buffer)` writing frame `frame`'s real cepstrum into a zeroed `size` buffer.
fn filtered(source: &[f32], size: usize, hop: usize, frames: usize, mut cepstrum: impl FnMut(usize, &mut [f64])) -> Vec<f32> {
    let half = size / 2;
    let fft = Fft::new(size);
    let window: Vec<f64> = (0..size).map(|n| 0.5 - 0.5 * (2.0 * PI * n as f64 / size as f64).cos()).collect();
    let (mut re, mut im) = (vec![0.0; size], vec![0.0; size]);
    let (mut cr, mut ci) = (vec![0.0; size], vec![0.0; size]);
    let mut sum = vec![0.0; frames * hop + size];
    let mut weight = vec![0.0; frames * hop + size];
    for f in 0..frames {
        let first = (f * hop) as isize - half as isize;
        for n in 0..size {
            let at = first + n as isize;
            re[n] = if at >= 0 && (at as usize) < source.len() { source[at as usize] as f64 * window[n] } else { 0.0 };
            im[n] = 0.0;
        }
        fft.run(&mut re, &mut im);
        cr.fill(0.0);
        ci.fill(0.0);
        cepstrum(f, &mut cr);
        fft.run(&mut cr, &mut ci);
        for k in 0..=half {
            let gain = cr[k].clamp(-14.0, 6.0).exp();
            let (c, s) = (gain * ci[k].cos(), gain * ci[k].sin());
            let (a, b) = (re[k], im[k]);
            re[k] = a * c - b * s;
            im[k] = -(a * s + b * c);
            if k > 0 && k < half {
                re[size - k] = re[k];
                im[size - k] = -im[k];
            }
        }
        fft.run(&mut re, &mut im);
        for n in 0..size {
            sum[f * hop + n] += re[n] / size as f64 * window[n];
            weight[f * hop + n] += window[n] * window[n];
        }
    }
    (0..source.len()).map(|n| (sum[n + half] / weight[n + half]) as f32).collect()
}

/// Band-limited pulse train with unit power, `hop` samples per f0 value, log-domain interpolation (model.py
/// `pulse_train`).
pub fn pulse_train(f0: &[f32], hop: usize, sample_rate: f64, limit: f64) -> Vec<f32> {
    let frames = f0.len();
    let mut out = vec![0.0; frames * hop];
    let knots: Vec<f64> = (0..=frames).map(|i| (f0[i.min(frames - 1)] as f64).max(1.0).ln()).collect();
    let mut turns = 0.0;
    for (n, sample) in out.iter_mut().enumerate() {
        let low = n / hop;
        let w = n as f64 / hop as f64 - low as f64;
        let frequency = (knots[low] * (1.0 - w) + knots[low + 1] * w).exp();
        turns += frequency / sample_rate;
        let theta = (2.0 * PI * turns) % (2.0 * PI);
        let harmonics = (limit * sample_rate / frequency).max(1.0);
        let whole = harmonics.floor();
        let below = 2.0 * (theta / 2.0).sin();
        let core = if below.abs() < 1e-7 { whole } else { ((whole + 0.5) * theta).sin() / below - 0.5 };
        *sample = ((core + (harmonics - whole) * ((whole + 1.0) * theta).cos()) * (2.0 / harmonics).sqrt()) as f32;
    }
    out
}

/// Deterministic unit Gaussian noise, the same numbers as `gaussianNoise(count, seed)` in tiny-voca's JS.
pub fn gaussian_noise(count: usize, seed: u32) -> Vec<f32> {
    let mut state = seed.wrapping_add(0x9E37_79B9);
    let mut uniform = || {
        state = state.wrapping_add(0x6D2B_79F5);
        let mut z = (state ^ (state >> 15)).wrapping_mul(state | 1);
        z ^= z.wrapping_add((z ^ (z >> 7)).wrapping_mul(z | 61));
        (z ^ (z >> 14)) as f64 / 4_294_967_296.0
    };
    let mut out = vec![0.0; count];
    let mut i = 0;
    while i < count {
        let radius = (-2.0 * (1.0 - uniform()).ln()).sqrt();
        let angle = 2.0 * PI * uniform();
        out[i] = (radius * angle.cos()) as f32;
        if i + 1 < count {
            out[i + 1] = (radius * angle.sin()) as f32;
        }
        i += 2;
    }
    out
}

/// Head settings (config.json `model`).
pub struct HeadConfig {
    pub sample_rate: usize,
    pub hop: usize,
    pub synth_fft: usize,
    pub synth_hop: usize,
    pub noise_fft: usize,
    pub noise_hop: usize,
    pub cep_causal: usize,
    pub cep_anti: usize,
    pub noise_cep: usize,
    pub harmonic_limit: f64,
}

fn lifter(n: usize) -> f64 {
    if n == 0 { 2.0 } else { 1.0 / (1.0 + n as f64 / 4.0) }
}

/// Renders the waveform from per-frame head outputs (raw, before the lifter):
/// `harmonic [frames x (cep_causal + cep_anti + hop/synth_hop)]`, `noise_cep [frames x (noise_cep + hop/noise_hop)]`,
/// the oscillator's `f0` in Hz per frame and `noise` (`frames * hop` unit Gaussian samples).
pub fn render(c: &HeadConfig, harmonic: &[f32], noise_cep: &[f32], f0: &[f32], noise: &[f32]) -> Vec<f32> {
    let frames = f0.len();
    let h_steps = c.hop / c.synth_hop;
    let hw = c.cep_causal + c.cep_anti + h_steps;
    let pulses = pulse_train(f0, c.hop, c.sample_rate as f64, c.harmonic_limit);
    let voiced = filtered(&pulses, c.synth_fft, c.synth_hop, h_steps * frames + 3, |f, cep| {
        let sub = f.min(h_steps * frames - 1);
        let here = sub / h_steps;
        let next = ((sub + 1) / h_steps).min(frames - 1);
        let (a, b) = (&harmonic[here * hw..(here + 1) * hw], &harmonic[next * hw..(next + 1) * hw]);
        for i in 0..c.cep_causal {
            cep[i] = 0.5 * (a[i] as f64 + b[i] as f64) * lifter(i);
        }
        for i in 0..c.cep_anti {
            let at = c.cep_causal + i;
            cep[c.synth_fft - 1 - i] = 0.5 * (a[at] as f64 + b[at] as f64) * lifter(i + 1);
        }
        cep[0] += a[c.cep_causal + c.cep_anti + sub % h_steps] as f64;
    });
    let g_steps = c.hop / c.noise_hop;
    let gw = c.noise_cep + g_steps;
    let breath = filtered(noise, c.noise_fft, c.noise_hop, g_steps * frames + 3, |f, cep| {
        let sub = f.min(g_steps * frames - 1);
        let g = &noise_cep[(sub / g_steps) * gw..(sub / g_steps + 1) * gw];
        for i in 0..c.noise_cep {
            cep[i] = g[i] as f64 * lifter(i);
        }
        cep[0] += g[c.noise_cep + sub % g_steps] as f64;
    });
    voiced.iter().zip(&breath).map(|(a, b)| a + b).collect()
}
