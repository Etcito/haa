//! Impact decomposition: a one-shot sound effect (a drop, a knock, a step, a shot's body) written as
//! a graph measured from the sample itself, the start point the fitter then refines.
//!
//! The template library holds instruments, engines and textures; none of them is a struck object,
//! so a drop fitted from the library alone ends on a rain or noise patch. This module builds the
//! struck-object graph from the target:
//! - the strongest resonances (spectral peaks standing over their local floor during the first
//!   `MODE_WINDOW` after the peak) become the `modes` of a `body` node, each with its frequency,
//!   its T60 (the slope of its level over time) and its gain;
//! - a `blast` excites the body: an instant rise (the contact) plus turbulence noise whose decay
//!   and tone come from the sample's early decay and its first frame's centroid; `direct` passes
//!   that excitation through for the non-modal part, the body's `wood` cloud covers the dense
//!   upper modes;
//! - analysis by synthesis: the graph is rendered and every mode's level and decay, measured the
//!   same way on the render, is corrected toward the target (`PASSES` passes), so the estimator's
//!   bias (a short mode smeared by the analysis window) cancels.
//!
//! Pure and deterministic like the rest of haa-fit.

use serde_json::{Value, json};

use crate::eval::Renderer;
use crate::fft::{RealFft, hann};
use crate::params::round_sig;

/// How long after the peak the spectrum is averaged to find the modes, s.
const MODE_WINDOW: f64 = 0.12;
/// At most this many modes (the `body` node keeps 32).
pub const MAX_MODES: usize = 24;
/// A peak counts as a mode when it stands this far over its local floor, dB.
const PROMINENCE_DB: f64 = 6.0;
/// ... and lies within this range of the strongest peak, dB (weaker peaks are the tail's noise).
const MODE_RANGE_DB: f64 = 30.0;
/// Analysis-by-synthesis passes.
const PASSES: usize = 5;
/// A mode's decay is measured over at most this long after its peak, s (later, the tail's noise
/// rides on its bin and reads as an endless ring).
const MAX_TRACK: f64 = 0.3;
/// Breakpoints of the noise tail's level curve.
const TAIL_POINTS: usize = 12;
/// The tail curve's amount: its breakpoints (0..1) scale the noise level up to this.
const TAIL_AMOUNT: f64 = 1.0;

/// One measured mode: frequency (Hz), T60 (s), level (dB re the strongest).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub hz: f64,
    pub t60: f64,
    pub db: f64,
}

/// What the analysis measures on a sample.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub modes: Vec<Mode>,
    /// Spectral centroid of the first frame at the peak, Hz.
    pub attack_centroid: f64,
    /// Spectral centroid over the mode window, Hz.
    pub centroid: f64,
    /// T60 of the broadband level from the peak, extrapolated from its first 20 dB, s.
    pub early_t60: f64,
    /// STFT bins of the modes (the correction measures the render there).
    bins: Vec<usize>,
    /// Spectral centroid after the mode window (the tail's noise), Hz.
    pub tail_centroid: f64,
    /// Time of the loudest frame, s.
    pub peak_s: f64,
}

/// Magnitude STFT in dB, frames of `n` with hop `hop`.
struct Stft {
    db: Vec<Vec<f64>>,
    bin_hz: f64,
    hop_s: f64,
    /// Frame holding the loudest moment.
    peak: usize,
}

fn stft(pcm: &[f32], sample_rate: f64) -> Stft {
    let n = ((sample_rate * 0.085) as usize).next_power_of_two().clamp(256, 16384);
    let hop = n / 16;
    let win = hann(n);
    let mut fft = RealFft::new(n);
    let mut frame = vec![0.0f64; n];
    let mut mag = vec![0.0f64; n / 2 + 1];
    let mut db = Vec::new();
    let mut energy = Vec::new();
    // Frames start half a window before the first sample, so the onset sits at a frame centre.
    let mut start: isize = -(n as isize) / 2;
    while start < pcm.len() as isize {
        for (i, f) in frame.iter_mut().enumerate() {
            let at = start + i as isize;
            let x = if at >= 0 && (at as usize) < pcm.len() { pcm[at as usize] as f64 } else { 0.0 };
            *f = x * win[i];
        }
        fft.magnitudes(&frame, &mut mag);
        energy.push(mag.iter().map(|m| m * m).sum::<f64>());
        db.push(mag.iter().map(|m| 20.0 * (m + 1e-12).log10()).collect());
        start += hop as isize;
    }
    let peak = energy.iter().enumerate().fold(0, |b, (i, e)| if *e > energy[b] { i } else { b });
    Stft { db, bin_hz: sample_rate / n as f64, hop_s: hop as f64 / sample_rate, peak }
}

/// dB of the mean power of frames `a..b` per bin.
fn mean_db(s: &Stft, a: usize, b: usize) -> Vec<f64> {
    let b = b.min(s.db.len()).max(a + 1);
    let bins = s.db[0].len();
    let mut out = vec![0.0; bins];
    for (k, o) in out.iter_mut().enumerate() {
        let p: f64 = (a..b).map(|f| 10f64.powf(s.db[f][k] / 10.0)).sum::<f64>() / (b - a) as f64;
        *o = 10.0 * (p + 1e-24).log10();
    }
    out
}

fn centroid(db: &[f64], bin_hz: f64) -> f64 {
    let (mut num, mut den) = (0.0, 0.0);
    for (k, d) in db.iter().enumerate().skip(1) {
        let p = 10f64.powf(d / 10.0);
        num += p * k as f64 * bin_hz;
        den += p;
    }
    if den > 0.0 { num / den } else { 1000.0 }
}

/// Level (dB) of one bin region (`bin` ± 1, the loudest) per frame from `from`.
fn track(s: &Stft, bin: usize, from: usize) -> Vec<f64> {
    let last = s.db[0].len() - 1;
    (from..s.db.len()).map(|f| (bin.saturating_sub(1)..=(bin + 1).min(last)).map(|k| s.db[f][k]).fold(f64::MIN, f64::max)).collect()
}

/// T60 from a level track: the least-squares slope over the frames until it falls 30 dB under its
/// maximum (or reaches the floor `floor_db`), s. Short or rising tracks give `fallback`.
fn t60_of(levels: &[f64], hop_s: f64, floor_db: f64, fallback: f64) -> f64 {
    let Some((imax, &max)) = levels.iter().enumerate().take(4).max_by(|a, b| a.1.partial_cmp(b.1).unwrap()) else { return fallback };
    let stop = max - 30.0;
    let limit = (imax + (MAX_TRACK / hop_s).round() as usize).min(levels.len());
    let end = levels[..limit].iter().enumerate().skip(imax).find(|(_, l)| **l < stop.max(floor_db + 3.0)).map_or(limit, |(i, _)| i);
    let pts: Vec<(f64, f64)> = (imax..end).map(|i| (i as f64 * hop_s, levels[i])).collect();
    if pts.len() < 3 {
        // Fell 30 dB within two frames: at most that long.
        return if pts.len() <= 1 { (2.0 * hop_s).min(fallback) } else { fallback };
    }
    let n = pts.len() as f64;
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (mx, my) = (sx / n, sy / n);
    let (num, den) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + (p.0 - mx) * (p.1 - my), a.1 + (p.0 - mx) * (p.0 - mx)));
    let slope = if den > 0.0 { num / den } else { 0.0 };
    if slope < -1.0 { (60.0 / -slope).clamp(0.005, 8.0) } else { fallback }
}

/// The modes (bins, frequencies, levels) standing over their local floor.
fn pick_modes(spec: &[f64], bin_hz: f64, sample_rate: f64) -> Vec<(usize, f64, f64)> {
    let bins = spec.len();
    let lo = (40.0 / bin_hz).ceil() as usize;
    let hi = ((0.4 * sample_rate) / bin_hz).floor().min(bins as f64 - 3.0) as usize;
    // Local floor: the median over a third of an octave each side (at least 4 bins).
    let floor: Vec<f64> = (0..bins)
        .map(|k| {
            let w = ((k as f64 * 0.26) as usize).max(4);
            let mut v: Vec<f64> = spec[k.saturating_sub(w)..(k + w + 1).min(bins)].to_vec();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        })
        .collect();
    let top = spec[lo..=hi].iter().cloned().fold(f64::MIN, f64::max);
    let mut peaks = Vec::new();
    for k in lo.max(2)..=hi {
        let v = spec[k];
        if v < top - MODE_RANGE_DB || v - floor[k] < PROMINENCE_DB {
            continue;
        }
        if (k - 2..=k + 2).any(|j| j != k && spec[j] > v) {
            continue;
        }
        // Parabolic interpolation on the dB values.
        let (a, b, c) = (spec[k - 1], v, spec[k + 1]);
        let d = a - 2.0 * b + c;
        let off = if d < 0.0 { (0.5 * (a - c) / d).clamp(-0.5, 0.5) } else { 0.0 };
        peaks.push((k, (k as f64 + off) * bin_hz, v - 0.25 * (a - c) * off));
    }
    peaks.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    peaks.truncate(MAX_MODES);
    peaks.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    peaks
}

/// Per-mode level and T60 measured on `pcm` at the given bins (the same estimator on target and render).
fn measure(s: &Stft, bins: &[usize]) -> Vec<(f64, f64)> {
    let w = (MODE_WINDOW / s.hop_s).round().max(2.0) as usize;
    let spec = mean_db(s, s.peak, s.peak + w);
    bins.iter()
        .map(|&k| {
            let lv = (k.saturating_sub(1)..=(k + 1).min(spec.len() - 1)).map(|j| spec[j]).fold(f64::MIN, f64::max);
            let tr = track(s, k, s.peak);
            let floor = tr.iter().cloned().fold(f64::MAX, f64::min);
            (lv, t60_of(&tr, s.hop_s, floor, 0.1))
        })
        .collect()
}

/// Measures modes, centroids and early decay on a mono sample.
pub fn analyze(pcm: &[f32], sample_rate: f64) -> Analysis {
    let s = stft(pcm, sample_rate);
    let w = (MODE_WINDOW / s.hop_s).round().max(2.0) as usize;
    let spec = mean_db(&s, s.peak, s.peak + w);
    let picked = pick_modes(&spec, s.bin_hz, sample_rate);
    let bins: Vec<usize> = picked.iter().map(|p| p.0).collect();
    let measured = measure(&s, &bins);
    let top = picked.iter().map(|p| p.2).fold(f64::MIN, f64::max);
    let modes = picked.iter().zip(&measured).map(|(p, m)| Mode { hz: round_sig(p.1), t60: round_sig(m.1), db: round_sig(p.2 - top) }).collect();
    let broadband: Vec<f64> = s.db[s.peak..].iter().map(|f| 10.0 * f.iter().map(|d| 10f64.powf(d / 10.0)).sum::<f64>().log10()).collect();
    let floor = broadband.iter().cloned().fold(f64::MAX, f64::min);
    let tail = mean_db(&s, (s.peak + w).min(s.db.len() - 1), s.db.len());
    Analysis {
        modes,
        bins,
        attack_centroid: centroid(&s.db[s.peak], s.bin_hz),
        centroid: centroid(&spec, s.bin_hz),
        tail_centroid: centroid(&tail, s.bin_hz),
        early_t60: t60_of(&broadband, s.hop_s, floor, 0.2),
        peak_s: s.peak as f64 * s.hop_s,
    }
}

fn modes_text(modes: &[Mode]) -> String {
    let rows: Vec<Value> = modes.iter().map(|m| json!([round_sig(m.hz), round_sig(m.t60), round_sig(m.db)])).collect();
    Value::Array(rows).to_string()
}

/// The levels the correction loop moves: mode gains, the body's and the strike's gain, the tail curve.
#[derive(Clone, Debug)]
struct Levels {
    modes: Vec<Mode>,
    body_gain: f64,
    strike: f64,
    /// (x in 0..1 of the duration, y in 0..1).
    tail: Vec<(f64, f64)>,
}

fn patch_json(id: &str, a: &Analysis, l: &Levels, seconds: f64) -> String {
    let tone = (2.0 * a.attack_centroid).clamp(200.0, 20_000.0);
    // The contact lasts about a quarter period of the attack's centre: a thud strikes long, a click short.
    let contact = (0.25 / a.attack_centroid.max(50.0)).clamp(0.0002, 0.01);
    let curve: Vec<Value> = l.tail.iter().map(|(x, y)| json!([round_sig(*x), round_sig(y.clamp(0.0, 1.0))])).collect();
    let patch = json!({
        "haa": 1,
        "id": id,
        "title": "Struck object (measured)",
        "inputs": [],
        "master": 1,
        "graph": {
            "mode": "timeline",
            "duration": round_sig(seconds.max(0.05)),
            "loop": false,
            "nodes": [
                {"id": "strike", "type": "blast", "x": 40, "y": 40, "label": "Contact", "params": {
                    "duration": round_sig(contact), "negative": 0.2, "noise": 0.5,
                    "noiseDecay": round_sig((a.early_t60 * 0.25).clamp(0.002, 1.0)), "tone": round_sig(tone), "delay": 0,
                    "level": round_sig(l.strike.clamp(0.0, 4.0))}},
                // The modes are measured at their frequencies: `size` (which divides them all) stays 1. Free, the
                // fit drove it to 2 on the round-15 grenade, an octave under its steel and a giant's thud.
                {"id": "body", "type": "body", "x": 300, "y": 40, "label": "Resonances (measured)", "fitFixed": ["size"], "params": {
                    "modes": modes_text(&l.modes), "size": 1, "decay": 1, "low": 1, "wood": 0.3,
                    "woodFreq": round_sig(a.centroid.clamp(300.0, 10_000.0)), "woodWidth": 3,
                    "woodDecay": round_sig((a.early_t60 * 0.3).clamp(0.003, 0.5)), "direct": 0.1,
                    "gain": round_sig(l.body_gain.clamp(0.0, 4.0)), "woodModes": 48}},
                {"id": "tail-env", "type": "curve", "x": 40, "y": 240, "label": "Tail level (measured)", "params": {"amount": 1}, "curve": curve},
                {"id": "tail", "type": "bandNoise", "x": 300, "y": 240, "label": "Tail noise", "params": {
                    "frequency": round_sig(a.tail_centroid.clamp(20.0, 18_000.0)), "bandwidth": 3, "tilt": 0, "level": 0, "width": 0}},
                {"id": "out", "type": "output", "x": 560, "y": 40, "label": "Out", "params": {"gain": 0.5}}
            ],
            "connections": [
                {"id": "c1", "from": "strike", "to": "body", "target": "audio", "amount": 1},
                {"id": "c2", "from": "body", "to": "out", "target": "audio", "amount": 1},
                {"id": "c3", "from": "tail-env", "to": "tail", "target": "level", "amount": TAIL_AMOUNT},
                {"id": "c4", "from": "tail", "to": "out", "target": "audio", "amount": 1}
            ],
            "performance": null
        }
    });
    patch.to_string()
}

/// RMS level (dB) of `pcm` in a 20 ms window centred on each time (s).
fn levels_at(pcm: &[f32], sample_rate: f64, times: &[f64]) -> Vec<f64> {
    let half = (0.01 * sample_rate) as usize;
    times
        .iter()
        .map(|t| {
            let c = (t * sample_rate) as usize;
            let (a, b) = (c.saturating_sub(half), (c + half).min(pcm.len()));
            let e = if b > a { pcm[a..b].iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / (b - a) as f64 } else { 0.0 };
            10.0 * (e + 1e-14).log10()
        })
        .collect()
}

/// The struck-object template of a sample: measured, then corrected by analysis by synthesis.
/// Returns the patch JSON and a report (the measured modes before and after correction).
pub fn template(pcm: &[f32], sample_rate: f64, id: &str) -> (String, Value) {
    let a = analyze(pcm, sample_rate);
    let seconds = pcm.len() as f64 / sample_rate;
    let target = stft(pcm, sample_rate);
    let want = measure(&target, &a.bins);
    // Tail breakpoints: from the end of the mode window to the end, closer together early on.
    let t0 = (a.peak_s + 0.5 * MODE_WINDOW).min(0.9 * seconds);
    let times: Vec<f64> = (0..TAIL_POINTS).map(|i| t0 + (seconds - t0) * (i as f64 / (TAIL_POINTS - 1) as f64).powf(1.6)).collect();
    let want_tail = levels_at(pcm, sample_rate, &times);
    let top_tail = want_tail.iter().cloned().fold(f64::MIN, f64::max);
    let mut tail: Vec<(f64, f64)> = vec![(0.0, 0.0)];
    tail.extend(times.iter().zip(&want_tail).map(|(t, d)| ((t / seconds).clamp(0.0, 1.0), 0.3 * 10f64.powf((d - top_tail) / 20.0))));
    let mut l = Levels { modes: a.modes.clone(), body_gain: 1.0, strike: 1.0, tail };
    let mut renderer = Renderer::new(sample_rate, pcm.len());
    let mut passes = Vec::new();
    for _ in 0..PASSES {
        let text = patch_json(id, &a, &l, seconds);
        let Ok((patch, _)) = haa_core::load_patch(&text) else { break };
        let Some(mono) = renderer.render(&patch).map(<[f32]>::to_vec) else { break };
        let got = measure(&stft(&mono, sample_rate), &a.bins);
        let mut err = 0.0;
        for (m, (w, g)) in l.modes.iter_mut().zip(want.iter().zip(&got)) {
            let dl = (w.0 - g.0).clamp(-24.0, 24.0);
            err += dl.abs();
            m.db = round_sig(m.db + dl);
            if g.1 > 0.0 && w.1 > 0.0 {
                m.t60 = round_sig((m.t60 * (w.1 / g.1).clamp(0.5, 2.0)).clamp(0.005, 4.0));
            }
        }
        // Keep the gains inside the node's range: the excess goes to the body's gain, then the strike's.
        let top = l.modes.iter().map(|m| m.db).fold(f64::MIN, f64::max);
        if top > 18.0 || top < -6.0 {
            let shift = top - 6.0;
            let want_gain = l.body_gain * 10f64.powf(shift / 20.0);
            l.body_gain = want_gain.clamp(0.01, 4.0);
            l.strike = (l.strike * want_gain / l.body_gain).clamp(0.01, 4.0);
            for m in &mut l.modes {
                m.db = round_sig(m.db - shift);
            }
        }
        // The tail: each breakpoint moves by what its moment lacks or has too much of.
        let got_tail = levels_at(&mono, sample_rate, &times);
        for (k, (w, g)) in want_tail.iter().zip(&got_tail).enumerate() {
            let p = &mut l.tail[k + 1];
            // Only the noise's share of the moment can move: where the modes alone pass the target, cut the noise.
            p.1 = (p.1 * 10f64.powf((w - g).clamp(-20.0, 20.0) / 20.0)).clamp(0.0, 1.0);
        }
        l.tail[0].1 = l.tail[1].1;
        passes.push(round_sig(err / l.modes.len().max(1) as f64));
    }
    let text = patch_json(id, &a, &l, seconds);
    let report = json!({
        "modes": l.modes.len(),
        "measured": modes_text(&a.modes),
        "corrected": modes_text(&l.modes),
        "meanLevelErrorDb": passes,
        "attackCentroidHz": round_sig(a.attack_centroid),
        "centroidHz": round_sig(a.centroid),
        "tailCentroidHz": round_sig(a.tail_centroid),
        "earlyT60": round_sig(a.early_t60),
        "bodyGain": round_sig(l.body_gain),
        "strike": round_sig(l.strike),
    });
    (text, report)
}

/// A fitted one-shot as a scene preset: its `id` and `title`, and an `energy` input (0..1, default 1)
/// that scales the output level linearly, `level` times the fitted level at energy 1. The sfx engine
/// leaves the level of a preset that reads `energy` to the preset (no fallback gain), so a soft hit
/// is quieter than the fitted one by the same law as the hand-built presets (level = energy).
pub fn sfx_preset(patch_json: &str, id: &str, title: &str, level: f64) -> Result<String, String> {
    let mut p: Value = serde_json::from_str(patch_json).map_err(|e| e.to_string())?;
    p["id"] = Value::from(id);
    p["title"] = Value::from(title);
    p["inputs"] = json!([{"name": "energy", "kind": "continuous", "unit": "", "min": 0, "max": 1, "default": 1, "smooth": 0, "curve": "lin"}]);
    let graph = p.get_mut("graph").ok_or("no graph")?;
    let nodes = graph.get_mut("nodes").and_then(Value::as_array_mut).ok_or("no nodes")?;
    let out = nodes.iter_mut().find(|n| n["type"] == "output").ok_or("no output node")?;
    let out_id = out["id"].clone();
    let mut gain = out["params"]["gain"].as_f64().unwrap_or(0.75) * level;
    out["params"]["gain"] = Value::from(0);
    nodes.push(json!({"id": "in-energy", "type": "input", "label": "Energy", "x": 20, "y": 20, "params": {"name": "energy"}}));
    let conns = graph.get_mut("connections").and_then(Value::as_array_mut).ok_or("no connections")?;
    // The output gain stops at 1: a level above it goes into the audio wires that feed the output (a
    // wire's amount is a plain factor), or the preset would stay under the level it was asked for.
    if gain > 1.0 {
        for c in conns.iter_mut().filter(|c| c["to"] == out_id && c["target"] == "audio") {
            let amount = c["amount"].as_f64().unwrap_or(1.0);
            c["amount"] = Value::from(round_sig(amount * gain));
        }
        gain = 1.0;
    }
    conns.push(json!({"id": "c-energy-level", "from": "in-energy", "to": out_id, "target": "gain", "amount": round_sig(gain)}));
    serde_json::to_string_pretty(&p).map(|s| s + "\n").map_err(|e| e.to_string())
}

/// The slip speed (the sfx contract's `speed`, m/s / 10) of the dragged stroke a slide preset is fitted
/// to: a crate pulled by hand at about 0.8 m/s.
pub const STROKE_SPEED: f64 = 0.08;

/// A fitted stroke as a continuous slide preset: the measured `stroke` level curve gives way to the
/// `speed` input (0 silent, `STROKE_SPEED` the stroke's loudest moment, twice it 1.5x louder), the
/// graph loops (a slide lasts as long as the contact), and `energy` scales the level linearly as in
/// `sfx_preset`. Err when the patch has no `stroke` curve or a time-dependent node besides it (a
/// fitter mutation's curve would pulse once per loop: fit strokes with `mutations: 0`).
pub fn slide_preset(patch_json: &str, id: &str, title: &str, level: f64) -> Result<String, String> {
    let mut p: Value = serde_json::from_str(patch_json).map_err(|e| e.to_string())?;
    {
        let graph = p.get_mut("graph").ok_or("no graph")?;
        let nodes = graph.get_mut("nodes").and_then(Value::as_array_mut).ok_or("no nodes")?;
        if let Some(n) = nodes.iter().find(|n| n["id"] != "stroke" && (n["type"] == "curve" || n["type"] == "envelope")) {
            return Err(format!("time-dependent node {} would repeat every loop", n["id"]));
        }
        let stroke = nodes.iter_mut().find(|n| n["id"] == "stroke").ok_or("no stroke curve")?;
        let peak = stroke["curve"].as_array().map_or(0.0, |c| c.iter().filter_map(|pt| pt[1].as_f64()).fold(0.0, f64::max));
        *stroke = json!({"id": "stroke", "type": "map", "label": "Speed to friction level", "x": 40, "y": 40, "params": {
            "in": 0, "inMin": 0, "inMax": 2.0 * STROKE_SPEED, "scale": "lin", "outMin": 0, "outMax": round_sig(1.5 * peak)},
            "curve": [[0, 0], [0.5, 0.6667], [1, 1]]});
        nodes.push(json!({"id": "in-speed", "type": "input", "label": "Speed", "x": 20, "y": 120, "params": {"name": "speed"}}));
        let conns = graph.get_mut("connections").and_then(Value::as_array_mut).ok_or("no connections")?;
        conns.push(json!({"id": "c-speed", "from": "in-speed", "to": "stroke", "target": "in", "amount": 1}));
        graph["loop"] = Value::from(true);
        graph["duration"] = Value::from(3);
    }
    let with_energy: Value = serde_json::from_str(&sfx_preset(&p.to_string(), id, title, level)?).map_err(|e| e.to_string())?;
    let mut out = with_energy;
    out["inputs"].as_array_mut().ok_or("no inputs")?.push(json!(
        {"name": "speed", "kind": "continuous", "unit": "", "min": 0, "max": 1, "default": STROKE_SPEED, "smooth": 0.05, "curve": "lin"}));
    serde_json::to_string_pretty(&out).map(|s| s + "\n").map_err(|e| e.to_string())
}

// --- strokes: a scrape, a drag, a slide -------------------------------------------------------

/// Breakpoints of a stroke's friction level curve.
const STROKE_POINTS: usize = 24;

/// Level (dB) of each mode bin averaged over the whole sound: a stroke excites its modes all along.
fn stroke_levels(s: &Stft, bins: &[usize]) -> Vec<f64> {
    let spec = mean_db(s, 0, s.db.len());
    bins.iter().map(|&k| (k.saturating_sub(1)..=(k + 1).min(spec.len() - 1)).map(|j| spec[j]).fold(f64::MIN, f64::max)).collect()
}

fn stroke_json(id: &str, modes: &[Mode], curve: &[(f64, f64)], centroid: f64, body_gain: f64, seconds: f64) -> String {
    let points: Vec<Value> = curve.iter().map(|(x, y)| json!([round_sig(*x), round_sig(y.clamp(0.0, 1.0))])).collect();
    json!({
        "haa": 1, "id": id, "title": "Stroke (measured)", "inputs": [], "master": 1,
        "graph": {
            "mode": "timeline", "duration": round_sig(seconds.max(0.05)), "loop": false,
            "nodes": [
                {"id": "stroke", "type": "curve", "x": 40, "y": 40, "label": "Stroke level (measured)", "params": {"amount": 1}, "curve": points},
                {"id": "friction", "type": "bandNoise", "x": 300, "y": 40, "label": "Friction", "params": {
                    "frequency": round_sig(centroid.clamp(20.0, 18_000.0)), "bandwidth": 4, "tilt": 0, "level": 0, "width": 0.3}},
                {"id": "body", "type": "body", "x": 560, "y": 40, "label": "Resonances (measured)", "fitFixed": ["size"], "params": {
                    "modes": modes_text(modes), "size": 1, "decay": 1, "low": 1, "wood": 0.3,
                    "woodFreq": round_sig(centroid.clamp(300.0, 10_000.0)), "woodWidth": 3, "woodDecay": 0.03,
                    "direct": 0.5, "gain": round_sig(body_gain.clamp(0.0, 4.0)), "woodModes": 48}},
                {"id": "out", "type": "output", "x": 820, "y": 40, "label": "Out", "params": {"gain": 0.5}}
            ],
            "connections": [
                {"id": "c1", "from": "stroke", "to": "friction", "target": "level", "amount": 1},
                {"id": "c2", "from": "friction", "to": "body", "target": "audio", "amount": 1},
                {"id": "c3", "from": "body", "to": "out", "target": "audio", "amount": 1}
            ],
            "performance": null
        }
    })
    .to_string()
}

/// The stroke template of a sample (a scrape or a drag): friction noise whose level follows the
/// measured envelope (a `curve` node, the part the scene drives by `speed`), shaped by the measured
/// resonances of the dragged object. Corrected by analysis by synthesis like `template`.
pub fn stroke_template(pcm: &[f32], sample_rate: f64, id: &str) -> (String, Value) {
    let seconds = pcm.len() as f64 / sample_rate;
    let s = stft(pcm, sample_rate);
    let spec = mean_db(&s, 0, s.db.len());
    let picked = pick_modes(&spec, s.bin_hz, sample_rate);
    let bins: Vec<usize> = picked.iter().map(|p| p.0).collect();
    let top = picked.iter().map(|p| p.2).fold(f64::MIN, f64::max);
    // A resonance under steady excitation shows its decay as its width: T60 = 2.2 / bandwidth, at
    // most what the analysis window resolves; the fitter's `decay` refines it.
    let t60 = round_sig(2.2 / (2.0 * s.bin_hz));
    let mut modes: Vec<Mode> = picked.iter().map(|p| Mode { hz: round_sig(p.1), t60, db: round_sig(p.2 - top) }).collect();
    let centroid = centroid(&spec, s.bin_hz);
    let want_modes = stroke_levels(&s, &bins);
    let times: Vec<f64> = (0..STROKE_POINTS).map(|i| seconds * i as f64 / (STROKE_POINTS - 1) as f64).collect();
    let want_env = levels_at(pcm, sample_rate, &times);
    let top_env = want_env.iter().cloned().fold(f64::MIN, f64::max);
    let mut curve: Vec<(f64, f64)> = times.iter().zip(&want_env).map(|(t, d)| (t / seconds, 0.5 * 10f64.powf((d - top_env) / 20.0))).collect();
    let mut body_gain = 1.0;
    let mut renderer = Renderer::new(sample_rate, pcm.len());
    let mut passes = Vec::new();
    for _ in 0..PASSES {
        let text = stroke_json(id, &modes, &curve, centroid, body_gain, seconds);
        let Ok((patch, _)) = haa_core::load_patch(&text) else { break };
        let Some(mono) = renderer.render(&patch).map(<[f32]>::to_vec) else { break };
        let got = stroke_levels(&stft(&mono, sample_rate), &bins);
        let mut err = 0.0;
        for (m, (w, g)) in modes.iter_mut().zip(want_modes.iter().zip(&got)) {
            let dl = (w - g).clamp(-24.0, 24.0);
            err += dl.abs();
            m.db = round_sig(m.db + dl);
        }
        let top = modes.iter().map(|m| m.db).fold(f64::MIN, f64::max);
        if top > 18.0 || top < -6.0 {
            let shift = top - 6.0;
            body_gain = (body_gain * 10f64.powf(shift / 20.0)).clamp(0.01, 4.0);
            for m in &mut modes {
                m.db = round_sig(m.db - shift);
            }
        }
        // The whole level follows the envelope: each breakpoint moves by its moment's error.
        let got_env = levels_at(&mono, sample_rate, &times);
        for (p, (w, g)) in curve.iter_mut().zip(want_env.iter().zip(&got_env)) {
            p.1 = (p.1 * 10f64.powf((w - g).clamp(-20.0, 20.0) / 20.0)).clamp(0.0, 1.0);
        }
        passes.push(round_sig(err / modes.len().max(1) as f64));
    }
    let text = stroke_json(id, &modes, &curve, centroid, body_gain, seconds);
    let report = json!({"modes": modes.len(), "corrected": modes_text(&modes), "meanLevelErrorDb": passes,
        "centroidHz": round_sig(centroid), "bodyGain": round_sig(body_gain)});
    (text, report)
}
