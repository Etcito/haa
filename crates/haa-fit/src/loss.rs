//! Loss kernel: how far a candidate render is from the target sample.
//!
//! Terms (each is a mean, about 0..2 for unrelated sounds and 0 for identical ones):
//! - `stft`: multi-resolution STFT (Hann, hop n/4, centred frames), per resolution the spectral
//!   convergence ‖T−C‖/‖T‖ plus the mean |dB(T) − dB(C)| / 20 with an −80 dB floor;
//! - `mel`: mean |dB| / 20 between the log-mel energies (64 bands, the middle resolution);
//! - `env`: mean |dB| / 20 between the RMS envelopes (5 ms hop, −80 dB floor);
//! - `f0`: mean |log2(f_c / f_t)| over voiced frames (capped at one octave), 0.5 per frame voiced on
//!   one side only; used when the target is pitched (`F0Mode::Auto`) or forced on.
//!
//! Levels are compared as they are: nothing is normalised, so the fitted patch carries the target's
//! loudness. The target features are computed once; `loss` reuses their scratch buffers.

use crate::fft::{RealFft, hann};

/// Whether the f0 term is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum F0Mode {
    /// On when the target is pitched (most active frames voiced).
    Auto,
    On,
    Off,
}

/// Loss weights and resolutions.
#[derive(Clone, Debug, PartialEq)]
pub struct LossConfig {
    /// STFT sizes (powers of two); hop is n/4.
    pub ffts: Vec<usize>,
    pub mel_bands: usize,
    pub w_stft: f64,
    pub w_mel: f64,
    pub w_env: f64,
    pub w_f0: f64,
    pub f0: F0Mode,
}

impl Default for LossConfig {
    fn default() -> LossConfig {
        LossConfig {
            ffts: vec![256, 1024, 4096],
            mel_bands: 64,
            w_stft: 1.0,
            w_mel: 1.0,
            w_env: 1.0,
            w_f0: 1.0,
            f0: F0Mode::Auto,
        }
    }
}

/// The weighted loss and its terms.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LossTerms {
    pub total: f64,
    pub stft: f64,
    pub mel: f64,
    pub env: f64,
    pub f0: f64,
}

impl LossTerms {
    /// The terms as a JSON object.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "total": round6(self.total), "stft": round6(self.stft), "mel": round6(self.mel),
            "env": round6(self.env), "f0": round6(self.f0),
        })
    }

    /// The loss of a candidate that does not render.
    pub fn failed() -> LossTerms {
        LossTerms { total: FAILED_LOSS, stft: FAILED_LOSS, mel: 0.0, env: 0.0, f0: 0.0 }
    }
}

/// Score of a candidate that cannot render.
pub const FAILED_LOSS: f64 = 1.0e3;
const MAG_FLOOR: f64 = 1.0e-4; // −80 dB
const MEL_FLOOR: f64 = 1.0e-10; // −100 dB (energy)
const ENV_FLOOR_DB: f64 = -80.0;

struct Resolution {
    fft: RealFft,
    window: Vec<f64>,
    hop: usize,
    frames: usize,
    bins: usize,
    /// Magnitude normaliser: a full-scale sine reads 1.0.
    norm: f64,
    /// Target magnitudes (frames × bins) and their dB.
    target: Vec<f64>,
    target_db: Vec<f64>,
    target_norm: f64,
}

struct Mel {
    /// One sparse triangle per band: (first bin, weights).
    bands: Vec<(usize, Vec<f64>)>,
    /// Centre frequency of each band in Hz.
    centres: Vec<f64>,
    res: usize,
}

/// One analysed signal's envelope, mel and pitch features.
#[derive(Clone, Debug, Default)]
struct Shape {
    mel_db: Vec<f64>,
    env_db: Vec<f64>,
    f0: Vec<f64>,
    /// Pitch frames with signal (above -60 dB RMS).
    active: usize,
}

/// Target features plus the scratch used to score candidates against them.
pub struct TargetFeatures {
    sample_rate: f64,
    len: usize,
    cfg: LossConfig,
    res: Vec<Resolution>,
    mel: Mel,
    env_hop: usize,
    pitch: Pitch,
    use_f0: bool,
    target: Shape,
    target_rms: f64,
    scratch_frame: Vec<f64>,
    scratch_mag: Vec<f64>,
    scratch_pcm: Vec<f64>,
    cand: Shape,
}

impl TargetFeatures {
    /// Analyses the mono target once.
    pub fn new(pcm: &[f32], sample_rate: f64, cfg: &LossConfig) -> TargetFeatures {
        let len = pcm.len().max(1);
        let mut sizes: Vec<usize> = cfg
            .ffts
            .iter()
            .copied()
            .filter(|n| n.is_power_of_two() && *n >= 16)
            .collect();
        // A resolution far longer than the sample only measures padding.
        let keep = |n: &usize| *n <= 2 * len.next_power_of_two();
        if sizes.iter().any(keep) {
            sizes.retain(keep);
        } else {
            sizes.truncate(1);
        }
        if sizes.is_empty() {
            sizes.push(256);
        }
        let res: Vec<Resolution> = sizes
            .iter()
            .map(|&n| {
                let hop = (n / 4).max(1);
                let frames = len.div_ceil(hop).max(1);
                let window = hann(n);
                let norm = window.iter().sum::<f64>() / 2.0;
                Resolution {
                    fft: RealFft::new(n),
                    window,
                    hop,
                    frames,
                    bins: n / 2 + 1,
                    norm,
                    target: Vec::new(),
                    target_db: Vec::new(),
                    target_norm: 0.0,
                }
            })
            .collect();
        let mel_res = res.len() / 2;
        let mel = Mel::new(&res[mel_res], sample_rate, cfg.mel_bands.max(4), mel_res);
        let env_hop = ((sample_rate * 0.005).round() as usize).max(1);
        let max_n = res.iter().map(|r| r.fft.len()).max().unwrap_or(256);
        let mut t = TargetFeatures {
            sample_rate,
            len,
            cfg: cfg.clone(),
            res,
            mel,
            env_hop,
            pitch: Pitch::new(sample_rate),
            use_f0: false,
            target: Shape::default(),
            target_rms: rms_of(pcm),
            scratch_frame: vec![0.0; max_n],
            scratch_mag: vec![0.0; max_n / 2 + 1],
            scratch_pcm: vec![0.0; len],
            cand: Shape::default(),
        };
        t.load_pcm(pcm);
        for r in 0..t.res.len() {
            let (mags, norm) = t.stft_target(r);
            let db = mags.iter().map(|m| db20(*m)).collect();
            let res = &mut t.res[r];
            res.target = mags;
            res.target_db = db;
            res.target_norm = norm;
        }
        let mut shape = Shape::default();
        t.shape_into(&mut shape, true);
        t.use_f0 = match cfg.f0 {
            F0Mode::On => true,
            F0Mode::Off => false,
            F0Mode::Auto => pitched(&shape),
        };
        t.target = shape;
        t
    }

    /// Target length in frames.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Target sample rate.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// True when the f0 term is part of the loss.
    pub fn uses_f0(&self) -> bool {
        self.use_f0
    }

    /// Per mel band, the time-averaged energy of the target minus that of the last candidate
    /// scored by `loss`, in dB (positive: the candidate lacks energy there), with band centres in Hz.
    pub fn mel_residual_db(&self) -> Vec<(f64, f64)> {
        let bands = self.mel.centres.len();
        let mean = |db: &[f64], b: usize| -> f64 {
            let frames = db.len() / bands.max(1);
            let e: f64 = (0..frames).map(|f| 10f64.powf(db[f * bands + b] / 10.0)).sum::<f64>() / frames.max(1) as f64;
            10.0 * e.max(MEL_FLOOR).log10()
        };
        (0..bands).map(|b| (self.mel.centres[b], mean(&self.target.mel_db, b) - mean(&self.cand.mel_db, b))).collect()
    }

    /// Mean target-minus-candidate envelope (dB) over the last `fraction` of the sample.
    pub fn tail_residual_db(&self, fraction: f64) -> f64 {
        let n = self.target.env_db.len().min(self.cand.env_db.len());
        let from = ((1.0 - fraction.clamp(0.0, 1.0)) * n as f64) as usize;
        if from >= n {
            return 0.0;
        }
        (from..n).map(|i| self.target.env_db[i] - self.cand.env_db[i]).sum::<f64>() / (n - from) as f64
    }

    /// Median f0 of the target's voiced frames (0 when unpitched).
    pub fn target_f0(&self) -> f64 {
        median_voiced(&self.target.f0)
    }

    /// Median f0 of a mono signal at the target rate (0 when unpitched).
    pub fn median_f0(&mut self, pcm: &[f32]) -> f64 {
        self.load_pcm(pcm);
        let mut f0 = Vec::new();
        let active = self.pitch.track(&self.scratch_pcm, &mut f0);
        let voiced = f0.iter().filter(|f| **f > 0.0).count();
        if voiced >= 3 && voiced * 2 > active { median_voiced(&f0) } else { 0.0 }
    }

    /// RMS of the target.
    pub fn target_rms(&self) -> f64 {
        self.target_rms
    }

    fn load_pcm(&mut self, pcm: &[f32]) {
        for (i, x) in self.scratch_pcm.iter_mut().enumerate() {
            *x = pcm.get(i).map_or(0.0, |v| f64::from(*v));
        }
    }

    /// Fills `scratch_frame` with the windowed frame `f` of resolution `r`.
    fn frame(&mut self, r: usize, f: usize) {
        let res = &self.res[r];
        let n = res.fft.len();
        let start = (f * res.hop) as isize - (n / 2) as isize;
        // Samples i in lo..hi of the frame fall inside the signal; the rest is zero padding.
        let lo = (-start).clamp(0, n as isize) as usize;
        let hi = (self.len as isize - start).clamp(lo as isize, n as isize) as usize;
        let frame = &mut self.scratch_frame[..n];
        frame[..lo].fill(0.0);
        frame[hi..].fill(0.0);
        let src = &self.scratch_pcm[(start + lo as isize) as usize..(start + hi as isize) as usize];
        for ((y, x), w) in frame[lo..hi].iter_mut().zip(src).zip(&res.window[lo..hi]) {
            *y = x * w;
        }
    }

    fn stft_target(&mut self, r: usize) -> (Vec<f64>, f64) {
        let (frames, bins) = (self.res[r].frames, self.res[r].bins);
        let mut mags = vec![0.0; frames * bins];
        let mut energy = 0.0;
        for f in 0..frames {
            self.frame(r, f);
            let n = self.res[r].fft.len();
            let res = &mut self.res[r];
            res.fft.magnitudes(&self.scratch_frame[..n], &mut self.scratch_mag[..bins]);
            for b in 0..bins {
                let m = self.scratch_mag[b] / res.norm;
                mags[f * bins + b] = m;
                energy += m * m;
            }
        }
        (mags, energy.sqrt())
    }

    /// Envelope, mel (from the stored or freshly computed magnitudes) and f0 of the loaded pcm.
    fn shape_into(&mut self, shape: &mut Shape, target: bool) {
        // RMS envelope.
        let hop = self.env_hop;
        let frames = self.len.div_ceil(hop).max(1);
        shape.env_db.clear();
        for f in 0..frames {
            let c = f * hop;
            let a = c.saturating_sub(hop);
            let b = (c + hop).min(self.len);
            let mut e = 0.0;
            for x in &self.scratch_pcm[a..b] {
                e += x * x;
            }
            let rms = (e / (b - a).max(1) as f64).sqrt();
            shape.env_db.push(db20(rms).max(ENV_FLOOR_DB));
        }
        if target {
            let r = self.mel.res;
            let bins = self.res[r].bins;
            shape.mel_db.clear();
            for f in 0..self.res[r].frames {
                let mags = &self.res[r].target[f * bins..(f + 1) * bins];
                self.mel.push_frame(mags, &mut shape.mel_db);
            }
        }
        if target || self.use_f0 {
            shape.active = self.pitch.track(&self.scratch_pcm, &mut shape.f0);
        }
    }

    /// Scores a mono candidate; it is zero-padded or cut to the target length.
    pub fn loss(&mut self, candidate: &[f32]) -> LossTerms {
        self.load_pcm(candidate);
        if self.scratch_pcm.iter().any(|x| !x.is_finite()) {
            return LossTerms::failed();
        }
        let mut stft = 0.0;
        let mut cand_mel = core::mem::take(&mut self.cand.mel_db);
        cand_mel.clear();
        for r in 0..self.res.len() {
            let (frames, bins) = (self.res[r].frames, self.res[r].bins);
            let mut diff2 = 0.0;
            let mut dbsum = 0.0;
            for f in 0..frames {
                self.frame(r, f);
                let n = self.res[r].fft.len();
                let res = &mut self.res[r];
                res.fft.magnitudes(&self.scratch_frame[..n], &mut self.scratch_mag[..bins]);
                for b in 0..bins {
                    let m = self.scratch_mag[b] / res.norm;
                    self.scratch_mag[b] = m;
                    let t = res.target[f * bins + b];
                    diff2 += (t - m) * (t - m);
                    dbsum += (res.target_db[f * bins + b] - db20(m)).abs();
                }
                if r == self.mel.res {
                    self.mel.push_frame(&self.scratch_mag[..bins], &mut cand_mel);
                }
            }
            let res = &self.res[r];
            let sc = diff2.sqrt() / res.target_norm.max(1e-9);
            stft += sc.min(4.0) + dbsum / (frames * bins) as f64 / 20.0;
        }
        stft /= self.res.len() as f64;
        let mel = mean_abs_diff(&self.target.mel_db, &cand_mel) / 20.0;
        self.cand.mel_db = cand_mel;
        let mut shape = core::mem::take(&mut self.cand);
        self.shape_into(&mut shape, false);
        let env = mean_abs_diff(&self.target.env_db, &shape.env_db) / 20.0;
        let f0 = if self.use_f0 { f0_distance(&self.target.f0, &shape.f0) } else { 0.0 };
        self.cand = shape;
        let c = &self.cfg;
        LossTerms {
            total: c.w_stft * stft + c.w_mel * mel + c.w_env * env + if self.use_f0 { c.w_f0 * f0 } else { 0.0 },
            stft,
            mel,
            env,
            f0,
        }
    }
}

/// Scores `candidate` against `target` (both mono, same rate) with the default config.
pub fn loss(target: &mut TargetFeatures, candidate: &[f32]) -> LossTerms {
    target.loss(candidate)
}

impl Mel {
    fn new(res: &Resolution, sr: f64, bands: usize, index: usize) -> Mel {
        let n = res.fft.len();
        let bins = res.bins;
        let fmax = (sr / 2.0).min(16_000.0);
        let (lo, hi) = (hz_to_mel(30.0), hz_to_mel(fmax));
        let edges: Vec<f64> = (0..bands + 2)
            .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (bands + 1) as f64) * n as f64 / sr)
            .collect();
        let mut out = Vec::with_capacity(bands);
        for b in 0..bands {
            let (l, c, r) = (edges[b], edges[b + 1], edges[b + 2]);
            let first = l.floor().max(0.0) as usize;
            let last = (r.ceil() as usize).min(bins - 1);
            let mut w = Vec::new();
            for k in first..=last {
                let x = k as f64;
                let v = if x <= c { (x - l) / (c - l).max(1e-9) } else { (r - x) / (r - c).max(1e-9) };
                w.push(v.clamp(0.0, 1.0));
            }
            // A band narrower than one bin still reads its nearest bin.
            if w.iter().all(|v| *v == 0.0) {
                let k = (c.round() as usize).min(bins - 1);
                out.push((k, vec![1.0]));
            } else {
                out.push((first, w));
            }
        }
        let centres = (0..bands).map(|b| edges[b + 1] * sr / n as f64).collect();
        Mel { bands: out, centres, res: index }
    }

    fn push_frame(&self, mags: &[f64], out: &mut Vec<f64>) {
        for (first, w) in &self.bands {
            let mut e = 0.0;
            for (i, wi) in w.iter().enumerate() {
                let m = mags[first + i];
                e += wi * m * m;
            }
            out.push(DB_PER_LOG2 * 0.5 * fast_log2(e.max(MEL_FLOOR)));
        }
    }
}

fn hz_to_mel(f: f64) -> f64 {
    2595.0 * (1.0 + f / 700.0).log10()
}

fn mel_to_hz(m: f64) -> f64 {
    700.0 * (10f64.powf(m / 2595.0) - 1.0)
}

fn db20(m: f64) -> f64 {
    DB_PER_LOG2 * fast_log2(m.max(MAG_FLOOR))
}

/// 20·log10(2): dB per octave of amplitude.
const DB_PER_LOG2: f64 = 6.020_599_913_279_624;

/// log2 of a positive normal number: exponent bits plus an atanh series on the mantissa
/// (relative error under 1e-6, far below the dB resolution the loss needs, and ~3× cheaper than log10).
fn fast_log2(x: f64) -> f64 {
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    let t = (m - 1.0) / (m + 1.0);
    let t2 = t * t;
    let ln = 2.0 * t * (1.0 + t2 * (1.0 / 3.0 + t2 * (0.2 + t2 * (1.0 / 7.0 + t2 * (1.0 / 9.0 + t2 / 11.0)))));
    exp as f64 + ln * core::f64::consts::LOG2_E
}

fn median_voiced(f0: &[f64]) -> f64 {
    let mut v: Vec<f64> = f0.iter().copied().filter(|f| *f > 0.0).collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    v[v.len() / 2]
}

/// RMS of a mono buffer.
pub fn rms_of(pcm: &[f32]) -> f64 {
    if pcm.is_empty() {
        return 0.0;
    }
    (pcm.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / pcm.len() as f64).sqrt()
}

fn mean_abs_diff(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    a[..n].iter().zip(&b[..n]).map(|(x, y)| (x - y).abs()).sum::<f64>() / n as f64
}

fn f0_distance(t: &[f64], c: &[f64]) -> f64 {
    let n = t.len().min(c.len());
    let (mut sum, mut count) = (0.0, 0usize);
    for i in 0..n {
        match (t[i] > 0.0, c[i] > 0.0) {
            (true, true) => {
                sum += (c[i] / t[i]).log2().abs().min(1.0);
                count += 1;
            }
            (true, false) | (false, true) => {
                sum += 0.5;
                count += 1;
            }
            (false, false) => {}
        }
    }
    if count == 0 { 0.0 } else { sum / count as f64 }
}

/// Pitched when most frames with signal carry an f0.
fn pitched(shape: &Shape) -> bool {
    let voiced = shape.f0.iter().filter(|f| **f > 0.0).count();
    voiced >= 3 && voiced * 2 > shape.active
}

pub(crate) fn round6(x: f64) -> f64 {
    (x * 1.0e6).round() / 1.0e6
}

/// Monophonic f0 tracker: McLeod's normalised square difference over FFT autocorrelation.
pub(crate) struct Pitch {
    sr: f64,
    /// Boxcar decimation factor: the tracker runs near 16 kHz (f0 stays under 2 kHz).
    decim: usize,
    dec: Vec<f64>,
    w: usize,
    hop: usize,
    fft: RealFft,
    frame: Vec<f64>,
    re: Vec<f64>,
    im: Vec<f64>,
    power: Vec<f64>,
    nsdf: Vec<f64>,
}

impl Pitch {
    pub(crate) fn new(sample_rate: f64) -> Pitch {
        let decim = ((sample_rate / 16_000.0).floor() as usize).max(1);
        let sr = sample_rate / decim as f64;
        let w = ((sr * 0.04) as usize).next_power_of_two().max(256);
        Pitch {
            sr,
            decim,
            dec: Vec::new(),
            w,
            hop: w / 2,
            fft: RealFft::new(2 * w),
            frame: vec![0.0; 2 * w],
            re: vec![0.0; w + 1],
            im: vec![0.0; w + 1],
            power: vec![0.0; 2 * w],
            nsdf: vec![0.0; w],
        }
    }

    /// One f0 per hop (Hz, 0 = unvoiced) over `pcm`; frames start at multiples of the hop.
    /// Returns how many frames carry signal.
    pub(crate) fn track(&mut self, pcm: &[f64], out: &mut Vec<f64>) -> usize {
        out.clear();
        let mut active = 0;
        let mut dec = core::mem::take(&mut self.dec);
        dec.clear();
        for chunk in pcm.chunks(self.decim) {
            dec.push(chunk.iter().sum::<f64>() / chunk.len() as f64);
        }
        let pcm = &dec[..];
        let frames = pcm.len().div_ceil(self.hop).max(1);
        let max_tau = ((self.sr / 50.0) as usize).min(self.w - 2);
        let min_tau = ((self.sr / 2000.0) as usize).max(2);
        for f in 0..frames {
            let start = f * self.hop;
            let mut energy = 0.0;
            for i in 0..2 * self.w {
                self.frame[i] = if i < self.w { pcm.get(start + i).copied().unwrap_or(0.0) } else { 0.0 };
                energy += self.frame[i] * self.frame[i];
            }
            if energy / (self.w as f64) < 1.0e-6 {
                out.push(0.0);
                continue;
            }
            active += 1;
            // Autocorrelation = DFT of the (real, even) power spectrum, divided by its size.
            self.fft.spectrum(&self.frame, &mut self.re, &mut self.im);
            let n = 2 * self.w;
            for k in 0..=self.w {
                let p = self.re[k] * self.re[k] + self.im[k] * self.im[k];
                self.power[k] = p;
                if k > 0 && k < self.w {
                    self.power[n - k] = p;
                }
            }
            self.fft.spectrum(&self.power, &mut self.re, &mut self.im);
            let mut m = 2.0 * self.re[0] / n as f64;
            for tau in 0..self.w {
                let r = self.re[tau] / n as f64;
                if tau > 0 {
                    let a = self.frame[tau - 1];
                    let b = self.frame[self.w - tau];
                    m -= a * a + b * b;
                }
                self.nsdf[tau] = if m > 1e-12 { 2.0 * r / m } else { 0.0 };
            }
            out.push(self.pick(min_tau, max_tau));
        }
        self.dec = dec;
        active
    }

    fn pick(&self, min_tau: usize, max_tau: usize) -> f64 {
        // Key maxima: the highest value between each positive-going and negative-going zero crossing.
        let mut peaks: Vec<(usize, f64)> = Vec::new();
        let mut tau = 1;
        while tau < max_tau && self.nsdf[tau] > 0.0 {
            tau += 1;
        }
        while tau < max_tau {
            while tau < max_tau && self.nsdf[tau] <= 0.0 {
                tau += 1;
            }
            let mut best = (tau, f64::MIN);
            while tau < max_tau && self.nsdf[tau] > 0.0 {
                if self.nsdf[tau] > best.1 {
                    best = (tau, self.nsdf[tau]);
                }
                tau += 1;
            }
            if best.1 > 0.0 && best.0 >= min_tau {
                peaks.push(best);
            }
        }
        let Some(top) = peaks.iter().map(|p| p.1).reduce(f64::max) else { return 0.0 };
        if top < 0.6 {
            return 0.0;
        }
        let Some(&(t, _)) = peaks.iter().find(|p| p.1 >= 0.9 * top) else { return 0.0 };
        let (a, b, c) = (self.nsdf[t - 1], self.nsdf[t], self.nsdf[t + 1]);
        let d = a - 2.0 * b + c;
        let shift = if d.abs() > 1e-12 { 0.5 * (a - c) / d } else { 0.0 };
        self.sr / (t as f64 + shift.clamp(-0.5, 0.5))
    }
}

/// A coarse description of a mono signal for reports: level and brightness over time.
pub fn summary(pcm: &[f32], sample_rate: f64, points: usize) -> serde_json::Value {
    let n = 1024usize;
    let hop = n / 4;
    let len = pcm.len();
    let window = hann(n);
    let mut fft = RealFft::new(n);
    let mut frame = vec![0.0; n];
    let mut mags = vec![0.0; n / 2 + 1];
    let frames = len.div_ceil(hop).max(1);
    let mut rms = Vec::with_capacity(frames);
    let mut centroid = Vec::with_capacity(frames);
    let (mut wsum, mut csum) = (0.0, 0.0);
    let mut peak = 0.0f64;
    for x in pcm {
        peak = peak.max(f64::from(x.abs()));
    }
    for f in 0..frames {
        let start = (f * hop) as isize - (n / 2) as isize;
        let mut e = 0.0;
        for i in 0..n {
            let j = start + i as isize;
            let x = if j >= 0 && (j as usize) < len { f64::from(pcm[j as usize]) } else { 0.0 };
            e += x * x;
            frame[i] = x * window[i];
        }
        fft.magnitudes(&frame, &mut mags);
        let (mut num, mut den) = (0.0, 0.0);
        for (k, m) in mags.iter().enumerate() {
            num += k as f64 * sample_rate / n as f64 * m;
            den += m;
        }
        let c = if den > 1e-9 { num / den } else { 0.0 };
        let r = (e / n as f64).sqrt();
        rms.push(db20(r).max(ENV_FLOOR_DB));
        centroid.push((c, r * r));
        wsum += r * r;
        csum += r * r * c;
    }
    let pick = |v: &[f64]| -> Vec<f64> {
        let p = points.max(1).min(v.len());
        (0..p)
            .map(|i| {
                let a = i * v.len() / p;
                let b = ((i + 1) * v.len() / p).max(a + 1);
                (v[a..b].iter().sum::<f64>() / (b - a) as f64 * 10.0).round() / 10.0
            })
            .collect()
    };
    // Energy-weighted centroid per point; 0 where the point is quieter than −60 dB.
    let p = points.max(1).min(centroid.len());
    let centroid_points: Vec<f64> = (0..p)
        .map(|i| {
            let a = i * centroid.len() / p;
            let b = ((i + 1) * centroid.len() / p).max(a + 1);
            let (num, den) = centroid[a..b].iter().fold((0.0, 0.0), |(n, d), (c, e)| (n + c * e, d + e));
            if den / ((b - a) as f64) < 1e-6 { 0.0 } else { (num / den).round() }
        })
        .collect();
    let total_rms = (pcm.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / len.max(1) as f64).sqrt();
    serde_json::json!({
        "seconds": round6(len as f64 / sample_rate),
        "peak": round6(peak),
        "rmsDb": (db20(total_rms) * 10.0).round() / 10.0,
        "centroidHz": if wsum > 0.0 { (csum / wsum).round() } else { 0.0 },
        "rmsEnvelopeDb": pick(&rms),
        "centroidEnvelopeHz": centroid_points,
    })
}

/// Compares two mono signals: loss terms (default config) plus both summaries and the envelope gap.
pub fn compare(target: &[f32], candidate: &[f32], sample_rate: f64, points: usize) -> serde_json::Value {
    let mut t = TargetFeatures::new(target, sample_rate, &LossConfig::default());
    let terms = t.loss(candidate);
    let a = summary(target, sample_rate, points);
    let b = summary(candidate, sample_rate, points);
    // Mean |difference| over the points both sides measure (a 0 centroid means a quiet point).
    let gap = |key: &str, skip_zero: bool| -> f64 {
        let (Some(x), Some(y)) = (a[key].as_array(), b[key].as_array()) else { return 0.0 };
        let pairs: Vec<(f64, f64)> = x
            .iter()
            .zip(y)
            .map(|(p, q)| (p.as_f64().unwrap_or(0.0), q.as_f64().unwrap_or(0.0)))
            .filter(|(p, q)| !skip_zero || (*p > 0.0 && *q > 0.0))
            .collect();
        if pairs.is_empty() {
            return 0.0;
        }
        (pairs.iter().map(|(p, q)| (p - q).abs()).sum::<f64>() / pairs.len() as f64 * 10.0).round() / 10.0
    };
    serde_json::json!({
        "loss": terms.to_json(),
        "usesF0": t.uses_f0(),
        "target": a,
        "candidate": b,
        "rmsEnvelopeGapDb": gap("rmsEnvelopeDb", false),
        "centroidEnvelopeGapHz": gap("centroidEnvelopeHz", true),
    })
}
