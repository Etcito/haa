//! Spectral judge for pitched instrument stems: how far a render's spectrum is from the stem's,
//! measured on the spectrum itself and on every note, not on a few band levels.
//!
//! Global terms (means over the cells/frames where the target is audible, about 0..1 each):
//! - `mel`: multi-resolution log-mel distance (Hann 512/2048/4096, hop n/4, 64 bands 40 Hz..16 kHz),
//!   mean |dB| / 20 per resolution;
//! - `lin`: log-magnitude distance on the linear 4096 spectrum up to 5 kHz (harmonic lines show
//!   there; mel bands blur them), mean |dB| / 20.
//!
//! Both compare against the stem's own noise floor: per band (bin), the 20th percentile of the
//! stem over time (at least 80 dB under its peak). Each side is raised to that floor, and a cell
//! counts when either side stands 3 dB above it: a separated stem's leakage hiss is not a sound
//! the instrument should make, but energy the render adds over the floor is still an error.
//! - `centroid`, `rolloff`: mean |log2(candidate / target)| of the per-frame spectral centroid and
//!   85 % roll-off (octaves, capped at 2), over the frames where both sides sound;
//! - `flatness`: mean |dB difference| / 20 of the per-frame spectral flatness;
//! - `mfcc`: mean |difference| / 20 of MFCC 1..12 (orthonormal DCT of the 2048 log-mel in dB).
//!   These four read the 2048 log-mel above the stem's noise floor (see below).
//!
//! Note terms (one value per note, then their mean), from the note list the render plays:
//! - `harm`: relative harmonic amplitudes, harmonics 1..12 at the note's start (peak of each
//!   harmonic's window on an FFT long enough to split the partials), each harmonic in dB relative
//!   to the strongest, mean |dB| / 20 over the harmonics the target holds within 50 dB;
//! - `lines`: harmonic line positions, mean |cents difference| / 100 of each partial's measured
//!   frequency (inharmonicity, detuning), over the partials the target holds within 30 dB;
//! - `env`: attack/decay envelope, the summed magnitude of harmonics 1..6 frame by frame from the
//!   onset to the note's end (+150 ms, at most 1.5 s), each side relative to its own peak, mean
//!   |dB| / 20, plus |decay slope difference| / 40 dB/s;
//! - `attack`: the pick transient, the loudest energy above max(800 Hz, 4.5 x f0) within 60 ms of
//!   the onset relative to the note's harmonic peak, |dB difference| / 20 (capped at 2).
//!
//! The target is analysed once (`SpectralTarget::new`); `score` reuses its scratch buffers. Levels
//! are compared as they are (match the loudness first: the global terms see a level offset).

use crate::fft::{RealFft, hann};

/// One note the render plays: seconds and MIDI pitch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteSpan {
    pub start: f64,
    pub end: f64,
    pub midi: f64,
}

/// Term weights of the total.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpectralWeights {
    pub mel: f64,
    pub lin: f64,
    pub centroid: f64,
    pub flatness: f64,
    pub rolloff: f64,
    pub mfcc: f64,
    pub harm: f64,
    pub lines: f64,
    pub env: f64,
    pub attack: f64,
}

impl Default for SpectralWeights {
    fn default() -> Self {
        SpectralWeights { mel: 1.0, lin: 0.5, centroid: 0.5, flatness: 0.25, rolloff: 0.25, mfcc: 0.5, harm: 1.0, lines: 0.5, env: 1.0, attack: 0.5 }
    }
}

/// The note terms of one note (`None` where the note could not be measured on the target).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NoteTerms {
    pub harm: Option<f64>,
    pub lines: Option<f64>,
    pub env: Option<f64>,
    pub attack: Option<f64>,
}

/// The weighted total and every term.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpectralTerms {
    pub total: f64,
    pub mel: f64,
    pub lin: f64,
    pub centroid: f64,
    pub flatness: f64,
    pub rolloff: f64,
    pub mfcc: f64,
    pub harm: f64,
    pub lines: f64,
    pub env: f64,
    pub attack: f64,
    pub notes: Vec<NoteTerms>,
}

impl SpectralTerms {
    /// The terms as JSON (`notes`: one `[harm, lines, env, attack]` per note, null where unmeasured).
    pub fn to_json(&self) -> serde_json::Value {
        let r = |x: f64| (x * 1e5).round() / 1e5;
        let o = |x: Option<f64>| x.map_or(serde_json::Value::Null, |v| serde_json::json!(r(v)));
        serde_json::json!({
            "total": r(self.total), "mel": r(self.mel), "lin": r(self.lin), "centroid": r(self.centroid),
            "flatness": r(self.flatness), "rolloff": r(self.rolloff), "mfcc": r(self.mfcc),
            "harm": r(self.harm), "lines": r(self.lines), "env": r(self.env), "attack": r(self.attack),
            "notes": self.notes.iter().map(|n| serde_json::json!([o(n.harm), o(n.lines), o(n.env), o(n.attack)])).collect::<Vec<_>>(),
        })
    }
}

const FFTS: [usize; 3] = [512, 2048, 4096];
/// Index in `FFTS` of the frame features (centroid, flatness, roll-off, MFCC, note envelopes).
const FRAME_RES: usize = 1;
/// Index in `FFTS` of the linear log-magnitude term.
const LIN_RES: usize = 2;
const LIN_MAX_HZ: f64 = 5_000.0;
const MEL_BANDS: usize = 64;
const MFCC: usize = 12;
const HARMONICS: usize = 12;
const ENV_HARMONICS: usize = 6;
/// The noise floor is at least this far under the target's peak.
const FLOOR_DB: f64 = 80.0;
/// A cell counts when either side stands this far over the noise floor.
const ABOVE_FLOOR_DB: f64 = 3.0;
const ENV_FLOOR_DB: f64 = -50.0;
const MAG_FLOOR: f64 = 1e-7;

struct Res {
    fft: RealFft,
    window: Vec<f64>,
    hop: usize,
    frames: usize,
    bins: usize,
    norm: f64,
    /// Sparse mel triangles: (first bin, weights).
    mel: Vec<(usize, Vec<f64>)>,
    target_mel: Vec<f64>,
    /// Per mel band: the stem's noise floor (dB).
    target_floor: Vec<f64>,
}

/// Per-frame descriptors of one signal (FRAME_RES frames).
#[derive(Clone, Default)]
struct Frames {
    centroid: Vec<f64>,
    rolloff: Vec<f64>,
    flatness_db: Vec<f64>,
    mfcc: Vec<f64>,
    energy_db: Vec<f64>,
    /// FRAME_RES magnitudes (frames x bins), kept for the note envelopes.
    mags: Vec<f64>,
}

#[derive(Clone, Default)]
struct NoteFeatures {
    /// dB relative to the strongest harmonic, `None` where no peak was found.
    harm_db: Vec<Option<f64>>,
    /// Partial frequency over k * nominal f0, in cents.
    cents: Vec<Option<f64>>,
    env_db: Vec<f64>,
    slope: f64,
    /// Transient energy over the harmonic peak (dB).
    attack_db: f64,
}

/// Target features plus the scratch used to score candidates against them.
pub struct SpectralTarget {
    sample_rate: f64,
    len: usize,
    weights: SpectralWeights,
    res: Vec<Res>,
    lin_bins: usize,
    target_lin: Vec<f64>,
    lin_floor: Vec<f64>,
    dct: Vec<f64>,
    mel_centres: Vec<f64>,
    notes: Vec<NoteSpan>,
    note_ffts: Vec<RealFft>,
    target_frames: Frames,
    target_notes: Vec<Option<NoteFeatures>>,
    pcm: Vec<f64>,
    frame: Vec<f64>,
    mag: Vec<f64>,
}

impl SpectralTarget {
    /// Analyses the mono target and the notes once.
    pub fn new(pcm: &[f32], sample_rate: f64, notes: &[NoteSpan], weights: SpectralWeights) -> SpectralTarget {
        let len = pcm.len().max(1);
        let sample_rate = sample_rate.max(1.0);
        let res: Vec<Res> = FFTS
            .iter()
            .map(|&n| {
                let window = hann(n);
                let norm = window.iter().sum::<f64>() / 2.0;
                let hop = n / 4;
                Res {
                    fft: RealFft::new(n),
                    window,
                    hop,
                    frames: len.div_ceil(hop).max(1),
                    bins: n / 2 + 1,
                    norm,
                    mel: mel_bank(n, sample_rate),
                    target_mel: Vec::new(),
                    target_floor: Vec::new(),
                }
            })
            .collect();
        let lin_bins = ((LIN_MAX_HZ * FFTS[LIN_RES] as f64 / sample_rate) as usize).min(FFTS[LIN_RES] / 2) + 1;
        let mut dct = vec![0.0; MFCC * MEL_BANDS];
        for c in 0..MFCC {
            for b in 0..MEL_BANDS {
                let k = (c + 1) as f64;
                dct[c * MEL_BANDS + b] = (2.0 / MEL_BANDS as f64).sqrt() * (core::f64::consts::PI * k * (b as f64 + 0.5) / MEL_BANDS as f64).cos();
            }
        }
        let max_n = 8192;
        let mut t = SpectralTarget {
            sample_rate,
            len,
            weights,
            res,
            lin_bins,
            target_lin: Vec::new(),
            lin_floor: Vec::new(),
            dct,
            mel_centres: mel_centres(sample_rate),
            notes: notes.iter().copied().filter(|n| n.end > n.start && n.midi.is_finite()).collect(),
            note_ffts: [1024usize, 2048, 4096, 8192].iter().map(|&n| RealFft::new(n)).collect(),
            target_frames: Frames::default(),
            target_notes: Vec::new(),
            pcm: vec![0.0; len],
            frame: vec![0.0; max_n],
            mag: vec![0.0; max_n / 2 + 1],
        };
        t.load(pcm);
        let mut lin = Vec::new();
        let mut frames = Frames::default();
        for r in 0..t.res.len() {
            let mut mel = Vec::new();
            t.analyse(r, &mut mel, if r == LIN_RES { Some(&mut lin) } else { None }, if r == FRAME_RES { Some(&mut frames) } else { None });
            t.res[r].target_floor = noise_floor(&mel, MEL_BANDS);
            t.res[r].target_mel = mel;
        }
        t.lin_floor = noise_floor(&lin, t.lin_bins);
        t.target_lin = lin;
        let mel = core::mem::take(&mut t.res[FRAME_RES].target_mel);
        t.describe(&mel, &mut frames);
        t.res[FRAME_RES].target_mel = mel;
        let notes = t.notes.clone();
        t.target_notes = notes.iter().map(|n| t.note_features(n, &frames)).collect();
        t.target_frames = frames;
        t
    }

    /// Number of notes the judge measures.
    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    fn load(&mut self, pcm: &[f32]) {
        for (i, x) in self.pcm.iter_mut().enumerate() {
            *x = pcm.get(i).map_or(0.0, |v| f64::from(*v));
        }
    }

    /// Windowed frame `f` of resolution `r` (centred) into `self.frame`.
    fn window_frame(&mut self, r: usize, f: usize) {
        let res = &self.res[r];
        let n = res.fft.len();
        let start = (f * res.hop) as isize - (n / 2) as isize;
        for i in 0..n {
            let j = start + i as isize;
            let x = if j >= 0 && (j as usize) < self.len { self.pcm[j as usize] } else { 0.0 };
            self.frame[i] = x * res.window[i];
        }
    }

    /// Log-mel (dB) of every frame of resolution `r`; optionally the linear log-spectrum and the
    /// frame descriptors.
    fn analyse(&mut self, r: usize, mel_out: &mut Vec<f64>, mut lin_out: Option<&mut Vec<f64>>, mut frames_out: Option<&mut Frames>) {
        mel_out.clear();
        if let Some(l) = lin_out.as_deref_mut() {
            l.clear();
        }
        if let Some(fr) = frames_out.as_deref_mut() {
            *fr = Frames::default();
        }
        let (frames, bins) = (self.res[r].frames, self.res[r].bins);
        let n = self.res[r].fft.len();
        let mut mel_frame = vec![0.0; MEL_BANDS];
        for f in 0..frames {
            self.window_frame(r, f);
            let res = &mut self.res[r];
            res.fft.magnitudes(&self.frame[..n], &mut self.mag[..bins]);
            let norm = res.norm;
            for m in &mut self.mag[..bins] {
                *m /= norm;
            }
            for (b, (first, w)) in res.mel.iter().enumerate() {
                let mut e = 0.0;
                for (i, wi) in w.iter().enumerate() {
                    let m = self.mag[first + i];
                    e += wi * m * m;
                }
                mel_frame[b] = 10.0 * e.max(MAG_FLOOR * MAG_FLOOR).log10();
            }
            mel_out.extend_from_slice(&mel_frame);
            if let Some(l) = lin_out.as_deref_mut() {
                for m in &self.mag[..self.lin_bins] {
                    l.push(20.0 * m.max(MAG_FLOOR).log10());
                }
            }
            if let Some(fr) = frames_out.as_deref_mut() {
                fr.mags.extend_from_slice(&self.mag[..bins]);
            }
        }
    }

    /// Frame descriptors from the FRAME_RES log-mel, above the stem's noise floor: each band's
    /// power minus the floor's (centroid, roll-off, flatness, energy) and the MFCC of both raised
    /// to the floor.
    fn describe(&self, mel: &[f64], fr: &mut Frames) {
        let floor = &self.res[FRAME_RES].target_floor;
        let frames = mel.len() / MEL_BANDS;
        let mut pw = [0.0f64; MEL_BANDS];
        let mut raised = [0.0f64; MEL_BANDS];
        for f in 0..frames {
            let row = &mel[f * MEL_BANDS..(f + 1) * MEL_BANDS];
            let (mut num, mut den, mut logsum) = (0.0, 0.0, 0.0);
            for b in 0..MEL_BANDS {
                let p = (10f64.powf(row[b] / 10.0) - 10f64.powf(floor[b] / 10.0)).max(0.0);
                pw[b] = p;
                raised[b] = row[b].max(floor[b]);
                num += self.mel_centres[b] * p;
                den += p;
                logsum += (p + 1e-12 * 10f64.powf(floor[b] / 10.0)).ln();
            }
            let centroid = if den > 0.0 { num / den } else { 0.0 };
            let mut acc = 0.0;
            let mut rolloff = 0.0;
            if den > 0.0 {
                for b in 0..MEL_BANDS {
                    acc += pw[b];
                    if acc >= 0.85 * den {
                        rolloff = self.mel_centres[b];
                        break;
                    }
                }
            }
            let mean = den / MEL_BANDS as f64;
            let flat = if mean > 0.0 { (logsum / MEL_BANDS as f64).exp() / mean } else { 1.0 };
            fr.centroid.push(centroid);
            fr.rolloff.push(rolloff);
            fr.flatness_db.push(10.0 * flat.clamp(1e-12, 1.0).log10());
            fr.energy_db.push(10.0 * den.max(1e-20).log10());
            for c in 0..MFCC {
                let d = &self.dct[c * MEL_BANDS..(c + 1) * MEL_BANDS];
                fr.mfcc.push(d.iter().zip(&raised).map(|(x, y)| x * y).sum());
            }
        }
    }

    /// Harmonic amplitudes, partial positions and the envelope of one note in the loaded signal.
    fn note_features(&mut self, note: &NoteSpan, frames: &Frames) -> Option<NoteFeatures> {
        let sr = self.sample_rate;
        let f0 = 440.0 * 2f64.powf((note.midi - 69.0) / 12.0);
        if !(20.0..sr / 4.0).contains(&f0) {
            return None;
        }
        // An FFT long enough to put six bins between partials, within 1024..8192.
        let want = 6.0 * sr / f0;
        let fi = [1024.0, 2048.0, 4096.0].iter().position(|&n| n >= want).unwrap_or(3);
        let n = 1024usize << fi;
        let start = ((note.start + 0.012) * sr) as isize;
        let dur = ((note.end - note.start).max(0.05) * sr) as isize;
        // Frame starts 12 ms after the onset (past the pick), cut to the note when the note is short.
        let window = hann(n);
        let norm = window.iter().sum::<f64>() / 2.0;
        let span = (n as isize).min(dur.max(n as isize / 2));
        for i in 0..n {
            let j = start + i as isize;
            let x = if (i as isize) < span && j >= 0 && (j as usize) < self.len { self.pcm[j as usize] } else { 0.0 };
            self.frame[i] = x * window[i];
        }
        let bins = n / 2 + 1;
        self.note_ffts[fi].magnitudes(&self.frame[..n], &mut self.mag[..bins]);
        let bin_hz = sr / n as f64;
        let mut harm_db = vec![None; HARMONICS];
        let mut cents = vec![None; HARMONICS];
        let mut top = -400.0f64;
        for k in 1..=HARMONICS {
            let fk = k as f64 * f0;
            if fk > 0.45 * sr {
                break;
            }
            // Partials may run sharp (stiffness): the window leans up.
            let lo = ((fk * 0.97) / bin_hz).floor().max(1.0) as usize;
            let hi = (((fk * (1.03 + 0.002 * k as f64)) / bin_hz).ceil() as usize).min(bins - 2);
            if hi <= lo {
                continue;
            }
            let (mut best, mut bk) = (0.0, lo);
            for b in lo..=hi {
                if self.mag[b] > best {
                    best = self.mag[b];
                    bk = b;
                }
            }
            if best <= MAG_FLOOR * norm {
                continue;
            }
            let (a, b, c) = (
                self.mag[bk - 1].max(1e-12).ln(),
                self.mag[bk].max(1e-12).ln(),
                self.mag[bk + 1].max(1e-12).ln(),
            );
            let d = a - 2.0 * b + c;
            let shift = if d.abs() > 1e-12 { (0.5 * (a - c) / d).clamp(-0.5, 0.5) } else { 0.0 };
            let freq = (bk as f64 + shift) * bin_hz;
            let level = 20.0 * (best / norm).log10();
            top = top.max(level);
            harm_db[k - 1] = Some(level);
            cents[k - 1] = Some(1200.0 * (freq / fk).log2());
        }
        if top < -90.0 {
            return None;
        }
        for h in harm_db.iter_mut().flatten() {
            *h -= top;
        }
        // Envelope: harmonics 1..6 summed per FRAME_RES frame.
        let res = &self.res[FRAME_RES];
        let bins_f = res.bins;
        let hop_s = res.hop as f64 / sr;
        let bin_f = sr / res.fft.len() as f64;
        let f_from = ((note.start - 0.02) / hop_s).floor().max(0.0) as usize;
        let f_to = (((note.end + 0.15).min(note.start + 1.5)) / hop_s).ceil() as usize;
        let f_to = f_to.min(frames.mags.len() / bins_f);
        if f_to <= f_from + 2 {
            return None;
        }
        let mut env = Vec::with_capacity(f_to - f_from);
        for f in f_from..f_to {
            let row = &frames.mags[f * bins_f..(f + 1) * bins_f];
            let mut e = 0.0;
            for k in 1..=ENV_HARMONICS {
                let b = (k as f64 * f0 / bin_f).round() as usize;
                if b + 1 >= bins_f {
                    break;
                }
                let m = row[b - 1].max(row[b]).max(row[b + 1]);
                e += m * m;
            }
            env.push(10.0 * e.max(1e-20).log10());
        }
        let peak = env.iter().copied().fold(-400.0, f64::max);
        let pi = env.iter().position(|v| *v == peak).unwrap_or(0);
        // Transient: the loudest frame of the energy above the harmonics' band near the onset.
        let first_bin = ((800.0f64).max(4.5 * f0) / bin_f).ceil() as usize;
        let a_to = ((note.start + 0.06) / hop_s).ceil() as usize;
        let mut transient = 1e-20f64;
        for f in f_from..a_to.min(f_to) {
            let row = &frames.mags[f * bins_f..(f + 1) * bins_f];
            let e: f64 = row[first_bin.min(bins_f)..].iter().map(|m| m * m).sum();
            transient = transient.max(e);
        }
        let attack_db = (10.0 * transient.log10() - peak).max(-80.0);
        for v in &mut env {
            *v = (*v - peak).max(ENV_FLOOR_DB);
        }
        // Least-squares decay slope (dB/s) after the peak.
        let tail = &env[pi..];
        let slope = if tail.len() >= 3 {
            let m = tail.len() as f64;
            let mx = (m - 1.0) / 2.0;
            let my = tail.iter().sum::<f64>() / m;
            let (mut sxy, mut sxx) = (0.0, 0.0);
            for (i, y) in tail.iter().enumerate() {
                let dx = i as f64 - mx;
                sxy += dx * (y - my);
                sxx += dx * dx;
            }
            if sxx > 0.0 { sxy / sxx / hop_s } else { 0.0 }
        } else {
            0.0
        };
        Some(NoteFeatures { harm_db, cents, env_db: env, slope, attack_db })
    }

    /// Scores a mono candidate (cut or zero-padded to the target length).
    pub fn score(&mut self, candidate: &[f32]) -> SpectralTerms {
        self.load(candidate);
        if self.pcm.iter().any(|x| !x.is_finite()) {
            return SpectralTerms { total: 1e3, ..SpectralTerms::default() };
        }
        let mut mel_sum = 0.0;
        let mut lin = Vec::new();
        let mut frames = Frames::default();
        let mut cand_mel = Vec::new();
        for r in 0..self.res.len() {
            self.analyse(r, &mut cand_mel, if r == LIN_RES { Some(&mut lin) } else { None }, if r == FRAME_RES { Some(&mut frames) } else { None });
            let res = &self.res[r];
            mel_sum += floor_db_distance(&res.target_mel, &cand_mel, &res.target_floor);
            if r == FRAME_RES {
                self.describe(&cand_mel, &mut frames);
            }
        }
        let mel = mel_sum / self.res.len() as f64;
        let lin_d = floor_db_distance(&self.target_lin, &lin, &self.lin_floor);

        // Frame descriptors over the frames where both sides sound (within 40 dB of their own
        // loudest frame): a missing or extra note is the mel term's error, these measure timbre.
        let tf = &self.target_frames;
        let loud = tf.energy_db.iter().copied().fold(-400.0, f64::max);
        let cand_loud = frames.energy_db.iter().copied().fold(-400.0, f64::max);
        let (mut c, mut ro, mut fl, mut mf, mut count) = (0.0, 0.0, 0.0, 0.0, 0usize);
        let nfr = tf.centroid.len().min(frames.centroid.len());
        for i in 0..nfr {
            if tf.energy_db[i] < loud - 40.0 || frames.energy_db[i] < cand_loud - 40.0 {
                continue;
            }
            count += 1;
            c += octaves(tf.centroid[i], frames.centroid[i]);
            ro += octaves(tf.rolloff[i], frames.rolloff[i]);
            fl += (tf.flatness_db[i] - frames.flatness_db[i]).abs().min(40.0) / 20.0;
            let a = &tf.mfcc[i * MFCC..(i + 1) * MFCC];
            let b = &frames.mfcc[i * MFCC..(i + 1) * MFCC];
            mf += a.iter().zip(b).map(|(x, y)| (x - y).abs().min(60.0)).sum::<f64>() / MFCC as f64 / 20.0;
        }
        let k = count.max(1) as f64;
        let (centroid, rolloff, flatness, mfcc) = (c / k, ro / k, fl / k, mf / k);

        // Notes.
        let notes = self.notes.clone();
        let mut per_note = Vec::with_capacity(notes.len());
        let (mut hs, mut hn, mut ls, mut ln, mut es, mut en, mut at, mut an) = (0.0, 0, 0.0, 0, 0.0, 0, 0.0, 0);
        for (i, note) in notes.iter().enumerate() {
            let Some(t) = self.target_notes[i].clone() else {
                per_note.push(NoteTerms::default());
                continue;
            };
            let cand = self.note_features(note, &frames);
            let terms = note_terms(&t, cand.as_ref());
            if let Some(v) = terms.harm {
                hs += v;
                hn += 1;
            }
            if let Some(v) = terms.lines {
                ls += v;
                ln += 1;
            }
            if let Some(v) = terms.env {
                es += v;
                en += 1;
            }
            if let Some(v) = terms.attack {
                at += v;
                an += 1;
            }
            per_note.push(terms);
        }
        let mean = |s: f64, n: usize| if n > 0 { s / n as f64 } else { 0.0 };
        let (harm, lines, env, attack) = (mean(hs, hn), mean(ls, ln), mean(es, en), mean(at, an));
        let w = self.weights;
        let total = w.mel * mel
            + w.lin * lin_d
            + w.centroid * centroid
            + w.flatness * flatness
            + w.rolloff * rolloff
            + w.mfcc * mfcc
            + w.harm * harm
            + w.lines * lines
            + w.env * env
            + w.attack * attack;
        SpectralTerms { total, mel, lin: lin_d, centroid, flatness, rolloff, mfcc, harm, lines, env, attack, notes: per_note }
    }
}

/// The note terms of a candidate note against the target's (a missing candidate note scores 1).
fn note_terms(t: &NoteFeatures, c: Option<&NoteFeatures>) -> NoteTerms {
    let Some(c) = c else {
        return NoteTerms { harm: Some(1.0), lines: None, env: Some(1.0), attack: Some(1.0) };
    };
    let (mut hs, mut hn, mut ls, mut ln) = (0.0, 0, 0.0, 0);
    for k in 0..HARMONICS {
        let Some(th) = t.harm_db[k] else { continue };
        if th < -50.0 {
            continue;
        }
        let ch = c.harm_db[k].unwrap_or(-60.0).max(-60.0);
        hs += (th - ch).abs().min(30.0) / 20.0;
        hn += 1;
        if th >= -30.0
            && let (Some(tc), Some(cc)) = (t.cents[k], c.cents[k])
        {
            ls += (tc - cc).abs().min(100.0) / 100.0;
            ln += 1;
        }
    }
    let n = t.env_db.len().min(c.env_db.len());
    let env = if n > 0 {
        let d = t.env_db[..n].iter().zip(&c.env_db[..n]).map(|(a, b)| (a - b).abs()).sum::<f64>() / n as f64 / 20.0;
        Some(d + (t.slope - c.slope).abs().min(80.0) / 40.0)
    } else {
        None
    };
    let attack = Some((t.attack_db - c.attack_db).abs().min(40.0) / 20.0);
    NoteTerms { harm: (hn > 0).then(|| hs / hn as f64), lines: (ln > 0).then(|| ls / ln as f64), env, attack }
}

/// Per column of a frames x `width` dB matrix: the 20th percentile over time, at least
/// `FLOOR_DB` under the matrix's peak.
fn noise_floor(db: &[f64], width: usize) -> Vec<f64> {
    let peak = db.iter().copied().fold(-400.0, f64::max);
    let frames = db.len() / width.max(1);
    let mut col = Vec::with_capacity(frames);
    (0..width)
        .map(|b| {
            col.clear();
            col.extend((0..frames).map(|f| db[f * width + b]));
            col.sort_by(|a, b| a.total_cmp(b));
            let q = col.get(frames / 5).copied().unwrap_or(peak - FLOOR_DB);
            q.max(peak - FLOOR_DB)
        })
        .collect()
}

/// Mean |dB| / 20 between both sides raised to the target's per-column noise floor, over the
/// cells where either side stands `ABOVE_FLOOR_DB` over that floor.
fn floor_db_distance(t: &[f64], c: &[f64], floor: &[f64]) -> f64 {
    let width = floor.len().max(1);
    let n = t.len().min(c.len());
    let (mut s, mut k) = (0.0, 0usize);
    for i in 0..n {
        let fl = floor[i % width];
        let (a, b) = (t[i].max(fl), c[i].max(fl));
        if a.max(b) < fl + ABOVE_FLOOR_DB {
            continue;
        }
        s += (a - b).abs().min(60.0);
        k += 1;
    }
    if k == 0 { 0.0 } else { s / k as f64 / 20.0 }
}

/// |log2(c / t)| in octaves, capped at 2 (a silent side counts 2).
fn octaves(t: f64, c: f64) -> f64 {
    if t <= 0.0 || c <= 0.0 {
        return if t <= 0.0 && c <= 0.0 { 0.0 } else { 2.0 };
    }
    (c / t).log2().abs().min(2.0)
}

/// Centre frequency (Hz) of each mel band of `mel_bank`.
fn mel_centres(sr: f64) -> Vec<f64> {
    let hz_to_mel = |f: f64| 2595.0 * (1.0 + f / 700.0).log10();
    let mel_to_hz = |m: f64| 700.0 * (10f64.powf(m / 2595.0) - 1.0);
    let (lo, hi) = (hz_to_mel(40.0), hz_to_mel((sr / 2.0).min(16_000.0)));
    (1..=MEL_BANDS).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (MEL_BANDS + 1) as f64)).collect()
}

/// Sparse mel triangles (MEL_BANDS, 40 Hz .. min(16 kHz, Nyquist)) for an `n`-point FFT.
fn mel_bank(n: usize, sr: f64) -> Vec<(usize, Vec<f64>)> {
    let bins = n / 2 + 1;
    let hz_to_mel = |f: f64| 2595.0 * (1.0 + f / 700.0).log10();
    let mel_to_hz = |m: f64| 700.0 * (10f64.powf(m / 2595.0) - 1.0);
    let (lo, hi) = (hz_to_mel(40.0), hz_to_mel((sr / 2.0).min(16_000.0)));
    let edges: Vec<f64> = (0..MEL_BANDS + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (MEL_BANDS + 1) as f64) * n as f64 / sr)
        .collect();
    (0..MEL_BANDS)
        .map(|b| {
            let (l, c, r) = (edges[b], edges[b + 1], edges[b + 2]);
            let first = l.floor().max(0.0) as usize;
            let last = (r.ceil() as usize).min(bins - 1);
            let w: Vec<f64> = (first..=last)
                .map(|k| {
                    let x = k as f64;
                    let v = if x <= c { (x - l) / (c - l).max(1e-9) } else { (r - x) / (r - c).max(1e-9) };
                    v.clamp(0.0, 1.0)
                })
                .collect();
            if w.iter().all(|v| *v == 0.0) { ((c.round() as usize).min(bins - 1), vec![1.0]) } else { (first, w) }
        })
        .collect()
}

/// Parses `[{"start", "end", "midi"}, ...]` (extra keys ignored).
pub fn parse_notes(json: &str) -> Vec<NoteSpan> {
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|n| {
            let g = |k: &str| n.get(k).and_then(serde_json::Value::as_f64);
            Some(NoteSpan { start: g("start")?, end: g("end")?, midi: g("midi")? })
        })
        .collect()
}
