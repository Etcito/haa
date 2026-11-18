//! The singer's own vowels and timbre: the chimera voice's habit inputs fitted on a stem's sung vowel nuclei.
//!
//! The lyrics voice says each vowel from the body's vowel tables; what a singer adds (a lower larynx, a more nasal
//! port, a firmer closure, a tongue further forward in one vowel) stays out of the tables. Each nucleus (a stretch of
//! one vowel the stem holds, its IPA and times) is rendered by the chimera node itself, `lyrics` mode with the vowel
//! held (rate input at its floor) on the stem's own f0 track, with the voice's settings and the timbre the phrase
//! curves play there; its spectrum is compared with the stem's on the same frames: 1/3-octave band levels 100 Hz to
//! 10 kHz, each against its own total (bands above 5 kHz at half weight), plus the harmonic-to-valley ratio in four
//! bands. CMA-ES (`cmaes`) moves the node's habit inputs in their own ranges (read from the node's definition):
//!
//! - the joint stage: every input, one value for every nucleus (the singer's static habit);
//! - the vowel stage: per vowel quality, the articulator inputs of [`VOWEL_KEYS`] alone, the static ones held
//!   (a value per vowel: the graph plays them as a few-point curve); a vowel whose own values gain less than
//!   [`VOWEL_GAIN_DB`] keeps the static ones.
//!
//! A small pull toward each input's default keeps an input the spectra do not need where it was. Steppable, like
//! [`crate::Fitter`]: [`SingerFitter::step`] renders at most `n` nuclei per call; pure and deterministic (seeded).

use haa_core::catalog::ParamKind;
use haa_core::patch::{Patch, load_patch};
use serde_json::{Map, Value, json};

use crate::cmaes::{Cma, Gauss};
use crate::eval::Renderer;
use crate::fft::{RealFft, hann};

const NFFT: usize = 2048;
const HOP: usize = 256;
/// Seconds rendered before a nucleus's first frame: the vowel planned and its articulators settled on it.
const WARM: f64 = 0.15;
/// Where the held vowel's line starts (patch s).
const LINE_START: f64 = 0.02;
/// The longest stretch of a nucleus analysed (s): a longer one is cut around its middle.
const MAX_NUCLEUS: f64 = 0.1;
/// The shortest nucleus kept (s).
const MIN_NUCLEUS: f64 = 0.05;
/// Band edges of the loss (Hz): 1/3 octaves from 100 Hz, 20 bands to 10.2 kHz.
const BAND_TOP: usize = 20;
/// Bands at or above this edge count half.
const HALF_WEIGHT_HZ: f64 = 5000.0;
const HNR_EDGES: [f64; 5] = [100.0, 700.0, 1500.0, 3000.0, 5000.0];
const HNR_WEIGHT: f64 = 0.3;
/// Loss per squared unit-range distance of an input from its default.
const PULL: f64 = 0.1;
/// Report bands (Hz): residual per band of each nucleus before and after.
pub const REPORT_EDGES: [f64; 15] = [100., 200., 300., 400., 500., 630., 800., 1000., 1250., 1600., 2000., 2500., 3150., 4000., 5000.];
/// A vowel keeps its own articulator values only when they bring its loss down by at least this (dB).
pub const VOWEL_GAIN_DB: f64 = 0.15;

/// The node's habit inputs the fit moves; the first [`VOWEL_KEYS`]`.len()` take a value per vowel in the vowel stage.
///
/// Left out: the velum and aspiration inputs, which act on every sound (a port held open under the stops' closures,
/// the glottal jet's noise in every voiceless consonant) where a nucleus sees only the vowel: on The Partisan their
/// nucleus fit (velum 0.53, aspiration 0) took the whole render's log-mel from 10.6 to 12.9 and 13.1 dB.
pub const SINGER_KEYS: [&str; 15] = [
    "larynx", "tongueHeight", "tongueFront", "lipRound",
    "closureSharpness", "skew", "openQuotient", "glottalGap", "sinus", "fossa", "lipSpread", "walls", "wallMass",
    "epilarynx", "tongueRoot",
];
/// The articulator inputs fitted per vowel quality.
pub const VOWEL_KEYS: [&str; 4] = ["larynx", "tongueHeight", "tongueFront", "lipRound"];

/// One input: its key and the node's range and default.
#[derive(Clone, Copy, Debug)]
pub struct SingerInput {
    pub key: &'static str,
    pub lo: f64,
    pub hi: f64,
    pub default: f64,
}

/// The habit inputs with the chimera node's own ranges.
pub fn singer_inputs() -> Vec<SingerInput> {
    SINGER_KEYS
        .iter()
        .map(|k| {
            let def = haa_core::nodes::chimera::DEF.params.iter().find(|p| p.key == *k).expect("chimera input");
            match def.kind {
                ParamKind::Number { min, max, default, .. } => SingerInput { key: k, lo: min, hi: max, default },
                _ => unreachable!("chimera habit inputs are numbers"),
            }
        })
        .collect()
}

/// A stretch of one vowel the stem holds.
#[derive(Clone, Debug)]
pub struct Nucleus {
    /// the vowel as the lyrics voice's IPA (a diphthong: its first part is held)
    pub ipa: String,
    /// stem seconds
    pub start: f64,
    pub end: f64,
    /// node params the phrase plays there (the timbre curves' values), over the voice's own
    pub params: Map<String, Value>,
}

/// Parses `{"ipa", "start", "end", "params"?}` objects.
pub fn parse_nuclei(v: &Value) -> Vec<Nucleus> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|n| {
                    Some(Nucleus {
                        ipa: n.get("ipa")?.as_str()?.to_string(),
                        start: n.get("start")?.as_f64()?,
                        end: n.get("end")?.as_f64()?,
                        params: n.get("params").and_then(Value::as_object).cloned().unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The fit's settings.
#[derive(Clone, Debug)]
pub struct SingerOptions {
    /// evaluations of the joint stage (each renders every nucleus kept)
    pub joint_evals: usize,
    /// evaluations per vowel quality in the vowel stage
    pub vowel_evals: usize,
    /// nuclei kept per vowel quality (the longest)
    pub per_vowel: usize,
    pub seed: u32,
    /// f0 search range (Hz)
    pub f_lo: f64,
    pub f_hi: f64,
}

impl Default for SingerOptions {
    fn default() -> Self {
        SingerOptions { joint_evals: 200, vowel_evals: 50, per_vowel: 1, seed: 1, f_lo: 70.0, f_hi: 700.0 }
    }
}

struct Target {
    ipa: String,
    start: f64,
    end: f64,
    bands: Vec<f64>,
    hnr: Vec<f64>,
    report: Vec<f64>,
    f0: Vec<f64>,
    patch: Patch,
    frames: usize,
    centres: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Joint,
    Vowel(usize),
    Done,
}

/// The fit in progress.
pub struct SingerFitter {
    inputs: Vec<SingerInput>,
    targets: Vec<Target>,
    /// vowel qualities, and per quality the targets that hold it
    vowels: Vec<(String, Vec<usize>)>,
    opts: SingerOptions,
    sample_rate: f64,
    fft: RealFft,
    win: Vec<f64>,
    g: Gauss,
    cma: Option<Cma>,
    stage: Stage,
    evals: usize,
    renders: usize,
    total_renders: usize,
    /// unit-cube values: the static set, and per vowel its own values of VOWEL_KEYS (None = the static ones)
    x_static: Vec<f64>,
    x_vowel: Vec<Option<Vec<f64>>>,
    loss_start: f64,
    loss_joint: f64,
    vowel_loss: Vec<(f64, f64)>,
    stage_best: f64,
    stage_best_x: Vec<f64>,
}

fn third_octave() -> Vec<f64> {
    (0..=BAND_TOP).map(|k| 100.0 * 2f64.powf(k as f64 / 3.0)).collect()
}

/// f0 of a frame by normalised autocorrelation in `lo..hi` Hz, parabolic peak (0 when nothing periodic).
fn ac_f0(x: &[f64], sr: f64, lo: f64, hi: f64) -> f64 {
    let n = x.len();
    let m = x.iter().sum::<f64>() / n as f64;
    let y: Vec<f64> = x.iter().map(|v| v - m).collect();
    let (kmin, kmax) = (((sr / hi) as usize).max(2), ((sr / lo) as usize).min(n / 2));
    if kmax <= kmin + 2 {
        return 0.0;
    }
    let r0: f64 = y.iter().map(|v| v * v).sum::<f64>() + 1e-20;
    let r: Vec<f64> = (0..=kmax + 1).map(|k| if k < kmin - 1 { 0.0 } else { (0..n - k).map(|i| y[i] * y[i + k]).sum::<f64>() / r0 }).collect();
    let (mut best, mut bk) = (f64::MIN, kmin);
    for k in kmin..=kmax {
        if r[k] > best {
            best = r[k];
            bk = k;
        }
    }
    if best < 0.3 {
        return 0.0;
    }
    let (a, b, c) = (r[bk - 1], r[bk], r[bk + 1]);
    let den = a - 2.0 * b + c;
    let off = if den.abs() > 1e-12 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 };
    sr / (bk as f64 + off)
}

/// Mean power spectrum of frames centred on `centres`, and the harmonic / valley ratio (dB) in the HNR bands.
fn analyze(x: &[f64], centres: &[usize], f0: &[f64], sr: f64, fft: &mut RealFft, win: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let nb = NFFT / 2 + 1;
    let mut mean = vec![0.0; nb];
    let (mut hp, mut vp) = ([0.0; 4], [0.0; 4]);
    let mut frame = vec![0.0; NFFT];
    let mut mag = vec![0.0; nb];
    let bin = sr / NFFT as f64;
    for (fi, &c) in centres.iter().enumerate() {
        for (i, s) in frame.iter_mut().enumerate() {
            let p = c as isize + i as isize - (NFFT / 2) as isize;
            *s = if p >= 0 && (p as usize) < x.len() { x[p as usize] * win[i] } else { 0.0 };
        }
        fft.magnitudes(&frame, &mut mag);
        for k in 0..nb {
            mean[k] += mag[k] * mag[k];
        }
        let f = f0[fi];
        if f <= 0.0 {
            continue;
        }
        let mut h = 1;
        while (h as f64 + 0.5) * f < HNR_EDGES[4] {
            let fk = h as f64 * f;
            if fk >= HNR_EDGES[0] {
                let band = (0..4).find(|b| fk >= HNR_EDGES[*b] && fk < HNR_EDGES[b + 1]).unwrap_or(3);
                let (lo, hi) = (((fk * 0.96) / bin).floor() as usize, ((fk * 1.04) / bin).ceil() as usize);
                let peak = (lo..=hi.min(nb - 1)).map(|k| mag[k] * mag[k]).fold(0.0, f64::max);
                let fv = fk + 0.5 * f;
                let (vlo, vhi) = (((fv - 0.1 * f) / bin).round() as usize, ((fv + 0.1 * f) / bin).round() as usize);
                let cnt = (vhi.min(nb - 1) + 1).saturating_sub(vlo).max(1);
                let valley = (vlo..=vhi.min(nb - 1)).map(|k| mag[k] * mag[k]).sum::<f64>() / cnt as f64;
                hp[band] += peak;
                vp[band] += valley;
            }
            h += 1;
        }
    }
    let hnr = (0..4).map(|b| 10.0 * ((hp[b] + 1e-30) / (vp[b] + 1e-30)).log10()).collect();
    (mean, hnr)
}

/// Band levels (dB) of a power spectrum between `edges`, each against the power from 100 Hz to the top edge.
fn band_levels(p: &[f64], edges: &[f64], sr: f64, norm_hi: f64) -> Vec<f64> {
    let bin = sr / NFFT as f64;
    let sum = |a: f64, b: f64| (0..p.len()).filter(|k| (*k as f64 * bin) >= a && (*k as f64 * bin) < b).map(|k| p[k]).sum::<f64>();
    let tot = sum(100.0, norm_hi) + 1e-30;
    edges.windows(2).map(|w| 10.0 * ((sum(w[0], w[1]) + 1e-30) / tot).log10()).collect()
}

fn nucleus_patch(base: &Map<String, Value>, n: &Nucleus, f0: &[f64], seconds: f64, sr: f64) -> Option<Patch> {
    let voiced: Vec<f64> = f0.iter().copied().filter(|v| *v > 0.0).collect();
    let mean = voiced.iter().sum::<f64>() / voiced.len().max(1) as f64;
    let lo = voiced.iter().copied().fold(f64::MAX, f64::min).min(mean) * 0.98;
    let hi = voiced.iter().copied().fold(0.0, f64::max).max(lo + 1.0);
    let mut params = base.clone();
    for (k, v) in &n.params {
        params.insert(k.clone(), v.clone());
    }
    params.insert("mode".into(), json!("lyrics"));
    params.insert("language".into(), json!("ipa"));
    params.insert("text".into(), json!(format!("{LINE_START:.2} {}", n.ipa)));
    params.insert("rate".into(), json!(0.1));
    params.insert("gain".into(), json!(0.5));
    params.insert("vibratoDepth".into(), json!(0.0));
    params.insert("pitch".into(), json!(lo));
    // the stem's f0 track: held through the warm-up, then frame by frame on the analysis frames' centres
    let x = |t: f64| (t / seconds).clamp(0.0, 1.0);
    let y = |f: f64| ((if f > 0.0 { f } else { mean } - lo) / (hi - lo)).clamp(0.0, 1.0);
    let mut pts = vec![json!([0.0, y(f0[0])]), json!([x(WARM), y(f0[0])])];
    for (i, f) in f0.iter().enumerate() {
        let t = WARM + (NFFT / 2 + i * HOP) as f64 / sr;
        pts.push(json!([x(t), y(*f)]));
    }
    let patch = json!({
        "haa": 1, "id": "singer-nucleus", "title": "singer nucleus", "inputs": [],
        "graph": {
            "duration": seconds, "loop": false, "mode": "timeline",
            "nodes": [
                { "id": "voice", "type": "chimera", "label": "voice", "params": params },
                { "id": "f0", "type": "curve", "label": "f0", "params": { "amount": 1 }, "curve": pts },
                { "id": "out", "type": "output", "label": "out", "params": { "gain": 1 } },
            ],
            "connections": [
                { "id": "c0", "from": "voice", "to": "out", "target": "audio", "amount": 1 },
                { "id": "c1", "from": "f0", "to": "voice", "target": "pitch", "amount": hi - lo },
            ],
        },
        "master": 1,
    });
    load_patch(&patch.to_string()).ok().map(|(p, _)| p)
}

impl SingerFitter {
    /// A fit of `nuclei` of the mono stem `pcm`: `base` holds the voice's chimera params (female, pressure, tension,
    /// breath, brightness, presence, tract, glide...). None when no nucleus is usable (too short, unvoiced, or its
    /// patch does not play).
    pub fn new(pcm: &[f32], sample_rate: f64, base: &Map<String, Value>, nuclei: &[Nucleus], opts: SingerOptions) -> Option<SingerFitter> {
        let sr = sample_rate;
        let x: Vec<f64> = pcm.iter().map(|v| *v as f64).collect();
        let mut fft = RealFft::new(NFFT);
        let win = hann(NFFT);
        // the longest nuclei of each vowel quality
        let mut order: Vec<&Nucleus> = nuclei.iter().filter(|n| n.end - n.start >= MIN_NUCLEUS && n.start >= 0.0 && n.end * sr < x.len() as f64).collect();
        order.sort_by(|a, b| (b.end - b.start).total_cmp(&(a.end - a.start)));
        let mut kept: Vec<&Nucleus> = Vec::new();
        for n in order {
            if kept.iter().filter(|k| k.ipa == n.ipa).count() < opts.per_vowel {
                kept.push(n);
            }
        }
        kept.sort_by(|a, b| a.start.total_cmp(&b.start));
        let edges = third_octave();
        let mut targets = Vec::new();
        for n in kept {
            let mid = 0.5 * (n.start + n.end);
            let half = (0.5 * (n.end - n.start)).min(0.5 * MAX_NUCLEUS);
            let (t0, t1) = (mid - half, mid + half);
            let c0 = (t0 * sr) as usize;
            let len = ((t1 - t0) * sr) as usize;
            let centres: Vec<usize> = (0..).map(|i| c0 + i * HOP).take_while(|c| *c < c0 + len).collect();
            let raw: Vec<f64> = centres
                .iter()
                .map(|&c| {
                    let (a, b) = (c.saturating_sub(NFFT / 2), (c + NFFT / 2).min(x.len()));
                    ac_f0(&x[a..b], sr, opts.f_lo, opts.f_hi)
                })
                .collect();
            if raw.iter().filter(|f| **f > 0.0).count() * 2 < raw.len().max(1) {
                continue;
            }
            let mut f0 = raw.clone();
            for i in 1..f0.len().saturating_sub(1) {
                let mut w = [raw[i - 1], raw[i], raw[i + 1]];
                w.sort_by(|p, q| p.total_cmp(q));
                f0[i] = w[1];
            }
            let (p, hnr) = analyze(&x, &centres, &f0, sr, &mut fft, &win);
            let bands = band_levels(&p, &edges, sr, edges[BAND_TOP]);
            let report = band_levels(&p, &REPORT_EDGES, sr, edges[BAND_TOP]);
            let frames = ((WARM * sr) as usize) + len + NFFT;
            let seconds = frames as f64 / sr;
            let Some(patch) = nucleus_patch(base, n, &f0, seconds, sr) else { continue };
            let rc: Vec<usize> = (0..centres.len()).map(|i| (WARM * sr) as usize + NFFT / 2 + i * HOP).collect();
            targets.push(Target { ipa: n.ipa.clone(), start: t0, end: t1, bands, hnr, report, f0, patch, frames, centres: rc });
        }
        if targets.is_empty() {
            return None;
        }
        let mut vowels: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, t) in targets.iter().enumerate() {
            match vowels.iter_mut().find(|v| v.0 == t.ipa) {
                Some(v) => v.1.push(i),
                None => vowels.push((t.ipa.clone(), vec![i])),
            }
        }
        let inputs = singer_inputs();
        let x_static: Vec<f64> = inputs.iter().map(|i| (i.default - i.lo) / (i.hi - i.lo)).collect();
        let total_renders = opts.joint_evals * targets.len() + opts.vowel_evals * targets.len();
        let mut g = Gauss::new(opts.seed);
        let cma = Cma::new(&x_static, 0.2, &vec![1.0; inputs.len()], 0, &mut g);
        let nv = vowels.len();
        let mut f = SingerFitter {
            inputs, targets, vowels, opts, sample_rate: sr, fft, win, g, cma: Some(cma),
            stage: Stage::Joint, evals: 0, renders: 0, total_renders, x_static: x_static.clone(), x_vowel: vec![None; nv],
            loss_start: f64::NAN, loss_joint: f64::NAN, vowel_loss: vec![(f64::NAN, f64::NAN); nv], stage_best: f64::INFINITY, stage_best_x: x_static.clone(),
        };
        f.loss_start = f.set_loss(&f.x_static.clone(), None, &(0..f.targets.len()).collect::<Vec<_>>());
        f.stage_best = f.loss_start;
        Some(f)
    }

    /// The node values (input units) of the static set, with a vowel's own VOWEL_KEYS values over them.
    fn values(&self, xs: &[f64], vowel: Option<&[f64]>) -> Vec<f64> {
        let mut v: Vec<f64> = self.inputs.iter().zip(xs).map(|(i, u)| i.lo + u.clamp(0.0, 1.0) * (i.hi - i.lo)).collect();
        if let Some(xv) = vowel {
            for (k, u) in xv.iter().enumerate() {
                let i = self.inputs[k];
                v[k] = i.lo + u.clamp(0.0, 1.0) * (i.hi - i.lo);
            }
        }
        v
    }

    /// One nucleus rendered with `values`: its band levels (loss edges and report edges) and HNR.
    fn features(&mut self, t: usize, values: &[f64]) -> Option<(Vec<f64>, Vec<f64>, Vec<f64>)> {
        let mut patch = self.targets[t].patch.clone();
        let node = patch.graph.nodes.iter_mut().find(|n| n.kind == "chimera")?;
        for (i, v) in self.inputs.iter().zip(values) {
            node.params.insert(i.key.to_string(), json!(v));
        }
        let frames = self.targets[t].frames;
        let mut r = Renderer::new(self.sample_rate, frames);
        let y: Vec<f64> = r.render(&patch)?.iter().map(|v| *v as f64).collect();
        self.renders += 1;
        let tg = &self.targets[t];
        let (p, hnr) = analyze(&y, &tg.centres, &tg.f0, self.sample_rate, &mut self.fft, &self.win);
        let edges = third_octave();
        Some((band_levels(&p, &edges, self.sample_rate, edges[BAND_TOP]), hnr, band_levels(&p, &REPORT_EDGES, self.sample_rate, edges[BAND_TOP])))
    }

    fn nucleus_loss(&self, t: usize, bands: &[f64], hnr: &[f64]) -> f64 {
        let tg = &self.targets[t];
        let edges = third_octave();
        let (mut s, mut w) = (0.0, 0.0);
        for (i, (a, b)) in bands.iter().zip(&tg.bands).enumerate() {
            let wt = if edges[i] >= HALF_WEIGHT_HZ { 0.5 } else { 1.0 };
            s += wt * (a - b).powi(2);
            w += wt;
        }
        let hn = (hnr.iter().zip(&tg.hnr).map(|(a, b)| (a - b).powi(2)).sum::<f64>() / 4.0).sqrt();
        (s / w).sqrt() + HNR_WEIGHT * hn
    }

    /// Mean loss over `targets` (no pull), the static set `xs`, a vowel's own values `xv`.
    fn set_loss(&mut self, xs: &[f64], xv: Option<&[f64]>, targets: &[usize]) -> f64 {
        let values = self.values(xs, xv);
        let mut s = 0.0;
        for &t in targets {
            match self.features(t, &values) {
                Some((b, h, _)) => s += self.nucleus_loss(t, &b, &h),
                None => return f64::INFINITY,
            }
        }
        s / targets.len().max(1) as f64
    }

    fn pull(&self, x: &[f64], keys: usize) -> f64 {
        x.iter().take(keys).enumerate().map(|(k, u)| {
            let i = self.inputs[k];
            (u - (i.default - i.lo) / (i.hi - i.lo)).powi(2)
        }).sum::<f64>() * PULL
    }

    fn start_vowel(&mut self, v: usize) {
        let xv: Vec<f64> = self.x_static[..VOWEL_KEYS.len()].to_vec();
        let ts = self.vowels[v].1.clone();
        let base = self.set_loss(&self.x_static.clone(), None, &ts);
        self.vowel_loss[v] = (base, base);
        self.stage_best = base;
        self.stage_best_x = xv.clone();
        self.evals = 0;
        self.cma = Some(Cma::new(&xv, 0.15, &vec![1.0; xv.len()], 0, &mut self.g));
        self.stage = Stage::Vowel(v);
    }

    /// Runs at most `max_renders` nucleus renders; false once the fit is done.
    pub fn step(&mut self, max_renders: usize) -> bool {
        let stop_at = self.renders + max_renders.max(1);
        while self.renders < stop_at {
            match self.stage {
                Stage::Done => return false,
                Stage::Joint => {
                    if self.evals >= self.opts.joint_evals {
                        self.x_static = self.stage_best_x.clone();
                        self.loss_joint = self.stage_best;
                        self.start_vowel(0);
                        continue;
                    }
                    let cand = self.cma.as_ref().map(|c| c.ask().to_vec()).unwrap_or_default();
                    let all: Vec<usize> = (0..self.targets.len()).collect();
                    let l = self.set_loss(&cand, None, &all);
                    let lp = l + self.pull(&cand, cand.len());
                    if lp < self.stage_best + self.pull(&self.stage_best_x.clone(), cand.len()) {
                        self.stage_best = l;
                        self.stage_best_x = cand.clone();
                    }
                    self.tell(lp, cand.len(), 0.12);
                }
                Stage::Vowel(v) => {
                    let ts = self.vowels[v].1.clone();
                    if self.evals >= self.opts.vowel_evals {
                        // a vowel keeps its own values only when they gain enough
                        if self.vowel_loss[v].0 - self.stage_best >= VOWEL_GAIN_DB {
                            self.x_vowel[v] = Some(self.stage_best_x.clone());
                            self.vowel_loss[v].1 = self.stage_best;
                        }
                        if v + 1 < self.vowels.len() {
                            self.start_vowel(v + 1);
                        } else {
                            self.stage = Stage::Done;
                            self.cma = None;
                        }
                        continue;
                    }
                    let cand = self.cma.as_ref().map(|c| c.ask().to_vec()).unwrap_or_default();
                    let l = self.set_loss(&self.x_static.clone(), Some(&cand), &ts);
                    // the pull toward the static values, not the defaults
                    let pull: f64 = cand.iter().zip(&self.x_static).map(|(a, b)| (a - b).powi(2)).sum::<f64>() * PULL;
                    let best_pull: f64 = self.stage_best_x.iter().zip(&self.x_static).map(|(a, b)| (a - b).powi(2)).sum::<f64>() * PULL;
                    if l + pull < self.stage_best + best_pull {
                        self.stage_best = l;
                        self.stage_best_x = cand.clone();
                    }
                    self.tell(l + pull, cand.len(), 0.08);
                }
            }
        }
        self.stage != Stage::Done
    }

    fn tell(&mut self, loss: f64, n: usize, restart_sigma: f64) {
        self.evals += 1;
        let restart = {
            let Some(c) = self.cma.as_mut() else { return };
            c.tell(loss, &mut self.g);
            c.converged() && c.at_generation_start()
        };
        if restart {
            self.cma = Some(Cma::new(&self.stage_best_x.clone(), restart_sigma, &vec![1.0; n], 0, &mut self.g));
        }
    }

    /// A nucleus as rendered with the inputs at their defaults (`fitted` false) or the fit's values: (ipa, start,
    /// mono samples from the warm-up's start).
    pub fn nucleus_audio(&self, t: usize, fitted: bool) -> Option<(String, f64, Vec<f32>)> {
        let tg = self.targets.get(t)?;
        let v = self.vowels.iter().position(|q| q.0 == tg.ipa).unwrap_or(0);
        let values = if fitted { self.values(&self.x_static, self.x_vowel[v].as_deref()) } else { self.inputs.iter().map(|i| i.default).collect() };
        let mut patch = tg.patch.clone();
        let node = patch.graph.nodes.iter_mut().find(|n| n.kind == "chimera")?;
        for (i, v) in self.inputs.iter().zip(&values) {
            node.params.insert(i.key.to_string(), json!(v));
        }
        let mut r = Renderer::new(self.sample_rate, tg.frames);
        let y = r.render(&patch)?;
        Some((tg.ipa.clone(), tg.start, y.to_vec()))
    }

    /// Nuclei kept.
    pub fn nucleus_count(&self) -> usize {
        self.targets.len()
    }

    /// Renders done and the total the fit plans.
    pub fn progress(&self) -> (usize, usize) {
        (self.renders, self.total_renders.max(self.renders))
    }

    /// The fit so far: `{"statics": {key: value}, "vowels": {ipa: {key: value}}, "vowelKeys", "loss": {"start",
    /// "joint", "final"}, "nuclei": [{"ipa", "start", "end", "before": [dB per report band, render - stem], "after"}],
    /// "reportEdges", "renders"}` (each vowel lists only the values it keeps apart from the statics).
    pub fn result(&mut self) -> Value {
        let r = |v: f64| (v * 1e4).round() / 1e4;
        let fin = self.values(&self.x_static.clone(), None);
        let mut statics = Map::new();
        for (i, v) in self.inputs.iter().zip(&fin) {
            statics.insert(i.key.into(), json!(r(*v)));
        }
        let mut vowels = Map::new();
        for (k, (ipa, _)) in self.vowels.iter().enumerate() {
            if let Some(xv) = &self.x_vowel[k] {
                let v = self.values(&self.x_static, Some(xv));
                let mut m = Map::new();
                for (j, key) in VOWEL_KEYS.iter().enumerate() {
                    m.insert((*key).into(), json!(r(v[j])));
                }
                vowels.insert(ipa.clone(), Value::Object(m));
            }
        }
        // per nucleus: the report bands before (defaults) and after (the fit), render minus stem
        let defaults: Vec<f64> = self.inputs.iter().map(|i| i.default).collect();
        let mut nuclei = Vec::new();
        let mut total = 0.0;
        for t in 0..self.targets.len() {
            let v = self.vowels.iter().position(|q| q.0 == self.targets[t].ipa).unwrap_or(0);
            let after = self.values(&self.x_static.clone(), self.x_vowel[v].clone().as_deref());
            let fb = self.features(t, &defaults);
            let fa = self.features(t, &after);
            let tg = &self.targets[t];
            let diff = |f: &Option<(Vec<f64>, Vec<f64>, Vec<f64>)>| f.as_ref().map(|f| f.2.iter().zip(&tg.report).map(|(a, b)| r(a - b)).collect::<Vec<_>>()).unwrap_or_default();
            if let Some((b, h, _)) = &fa {
                total += self.nucleus_loss(t, b, h);
            }
            let tg = &self.targets[t];
            nuclei.push(json!({ "ipa": tg.ipa, "start": r(tg.start), "end": r(tg.end), "before": diff(&fb), "after": diff(&fa) }));
        }
        json!({
            "statics": statics, "vowels": vowels, "vowelKeys": VOWEL_KEYS,
            "loss": { "start": r(self.loss_start), "joint": r(self.loss_joint), "final": r(total / self.targets.len().max(1) as f64) },
            "nuclei": nuclei, "reportEdges": REPORT_EDGES, "renders": self.renders,
        })
    }
}

/// The nuclei of one line from its phones timed on the stem (`[ipa, start]` in order, the line ending at `end`): each
/// vowel phone (until the next phone starts) without its first and last fifth; a diphthong (two vowel letters) its
/// first part, from 15 % to 60 % of its span. `(ipa, start, end)`.
pub fn nuclei_from_phones(phones: &[(String, f64)], end: f64) -> Vec<(String, f64, f64)> {
    let mut out = Vec::new();
    for (k, (ipa, s)) in phones.iter().enumerate() {
        if !crate::phonemes::is_vowel_phone(ipa) {
            continue;
        }
        let e = phones.get(k + 1).map_or(end, |p| p.1);
        let d = e - s;
        if !(d > 0.0) {
            continue;
        }
        let vowels = ipa.chars().filter(|c| "iyɨʉɯuɪʏʊeøɘɵɤoəɛœɜɞʌɔæɐaɶɑɒɝɚᵻ".contains(*c)).count();
        let (a, b) = if vowels >= 2 { (s + 0.15 * d, s + 0.6 * d) } else { (s + 0.2 * d, e - 0.2 * d) };
        out.push((ipa.clone(), a, b));
    }
    out
}

/// A fit from a JSON request: `{"base": {chimera params}, "phrases": [{"phones": [[ipa, stem s], ...], "end": s}]` (or
/// `"nuclei": [{"ipa", "start", "end", "params"?}]`), `"timbre": {"hop": s, "values": {input: [value per hop frame]}}`
/// (what the phrase curves play: each nucleus takes their values at its middle), and the options `jointEvals`,
/// `vowelEvals`, `perVowel`, `seed`, `fLo`, `fHi`. Err names what is missing.
pub fn singer_from_request(pcm: &[f32], sample_rate: f64, req: &Value) -> Result<SingerFitter, String> {
    let base = req.get("base").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut nuclei = parse_nuclei(req.get("nuclei").unwrap_or(&Value::Null));
    if let Some(phrases) = req.get("phrases").and_then(Value::as_array) {
        for p in phrases {
            let phones: Vec<(String, f64)> = p
                .get("phones")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|q| Some((q.get(0)?.as_str()?.to_string(), q.get(1)?.as_f64()?))).collect())
                .unwrap_or_default();
            let end = p.get("end").and_then(Value::as_f64).unwrap_or_else(|| phones.last().map_or(0.0, |q| q.1 + 0.1));
            for (ipa, start, end) in nuclei_from_phones(&phones, end) {
                nuclei.push(Nucleus { ipa, start, end, params: Map::new() });
            }
        }
    }
    if let Some(t) = req.get("timbre") {
        let hop = t.get("hop").and_then(Value::as_f64).filter(|h| *h > 0.0).unwrap_or(0.01);
        if let Some(values) = t.get("values").and_then(Value::as_object) {
            for n in nuclei.iter_mut() {
                let i = (0.5 * (n.start + n.end) / hop).round() as usize;
                for (k, arr) in values {
                    if let Some(v) = arr.as_array().and_then(|a| a.get(i)).and_then(Value::as_f64) {
                        n.params.entry(k.clone()).or_insert(json!(v));
                    }
                }
            }
        }
    }
    let num = |k: &str| req.get(k).and_then(Value::as_f64);
    let d = SingerOptions::default();
    let opts = SingerOptions {
        joint_evals: num("jointEvals").map_or(d.joint_evals, |v| v.max(0.0) as usize),
        vowel_evals: num("vowelEvals").map_or(d.vowel_evals, |v| v.max(0.0) as usize),
        per_vowel: num("perVowel").map_or(d.per_vowel, |v| v.max(1.0) as usize),
        seed: num("seed").map_or(d.seed, |v| v as u32),
        f_lo: num("fLo").unwrap_or(d.f_lo),
        f_hi: num("fHi").unwrap_or(d.f_hi),
    };
    if nuclei.is_empty() {
        return Err("no vowel nucleus in the request".into());
    }
    SingerFitter::new(pcm, sample_rate, &base, &nuclei, opts).ok_or_else(|| "no usable vowel nucleus (too short, unvoiced or unplayable)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A held vowel sung by the chimera node with `habit` (node inputs) over `base`: (mono samples, sample rate).
    fn sung(ipa: &str, f0: f64, base: &Map<String, Value>, habit: &[(&str, f64)], seconds: f64) -> Vec<f32> {
        let sr = 44_100.0;
        let mut params = base.clone();
        for (k, v) in habit {
            params.insert((*k).into(), json!(v));
        }
        params.insert("mode".into(), json!("lyrics"));
        params.insert("language".into(), json!("ipa"));
        params.insert("text".into(), json!(format!("0.02 {ipa}")));
        params.insert("rate".into(), json!(0.1));
        params.insert("gain".into(), json!(0.5));
        params.insert("pitch".into(), json!(f0));
        let patch = json!({
            "haa": 1, "id": "t", "title": "t", "inputs": [],
            "graph": { "duration": seconds, "loop": false, "mode": "timeline",
                "nodes": [{ "id": "voice", "type": "chimera", "label": "v", "params": params }, { "id": "out", "type": "output", "label": "o", "params": { "gain": 1 } }],
                "connections": [{ "id": "c0", "from": "voice", "to": "out", "target": "audio", "amount": 1 }] },
            "master": 1,
        });
        let (p, _) = load_patch(&patch.to_string()).unwrap();
        let frames = (seconds * sr) as usize;
        Renderer::new(sr, frames).render(&p).unwrap().to_vec()
    }

    #[test]
    fn singer_fit_finds_a_habit_the_vowel_tables_lack_and_cuts_nuclei_from_phones() {
        // the nuclei: a vowel's middle three fifths, a diphthong's first part, consonants skipped
        let n = nuclei_from_phones(&[("m".into(), 0.0), ("a".into(), 0.1), ("eɪ".into(), 0.3), ("n".into(), 0.5)], 0.7);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].0, "a");
        assert!((n[0].1 - 0.14).abs() < 1e-9 && (n[0].2 - 0.26).abs() < 1e-9);
        assert!((n[1].1 - 0.33).abs() < 1e-9 && (n[1].2 - 0.42).abs() < 1e-9);

        // a "stem": /a/ sung with a lowered larynx, an advanced tongue root and a firmer closure; the fit starts from the
        // defaults
        let mut base = Map::new();
        for (k, v) in [("female", 0.0), ("pressure", 10.0), ("tension", 0.6), ("breath", 0.1), ("brightness", 0.25)] {
            base.insert(k.into(), json!(v));
        }
        let habit = [("larynx", -1.0), ("tongueRoot", 1.0), ("closureSharpness", 0.6)];
        let stem = sung("a", 130.0, &base, &habit, 0.6);
        let nuclei = [Nucleus { ipa: "a".into(), start: 0.3, end: 0.44, params: Map::new() }];
        let opts = SingerOptions { joint_evals: 120, vowel_evals: 30, per_vowel: 1, seed: 3, f_lo: 70.0, f_hi: 400.0 };
        let mut f = SingerFitter::new(&stem, 44_100.0, &base, &nuclei, opts).expect("a usable nucleus");
        while f.step(40) {}
        let r = f.result();
        let (start, fin) = (r["loss"]["start"].as_f64().unwrap(), r["loss"]["final"].as_f64().unwrap());
        // the defaults are far from the stem; the fit lands within a fraction of that distance
        assert!(start > 2.0, "start loss {start}");
        assert!(fin < 0.6 * start, "final loss {fin} from {start}");
        // the habit inputs default to the node's own defaults, so a voice without the fit is unchanged
        let inputs = singer_inputs();
        let i = inputs.iter().position(|i| i.key == "larynx").unwrap();
        assert_eq!(inputs[i].default, 0.0);
        assert!(r["statics"]["larynx"].as_f64().unwrap() < -0.3, "larynx {}", r["statics"]["larynx"]);
    }
}
