//! The emotion and delivery transform of the `speak` node: a port of the reference `emo/apply.py` (NPC voice,
//! workstream B, SCRIPT FORMAT v2), float64 throughout, matched to its golden pairs to 1e-4.
//!
//! A performance (tiny-voca tokens, stress, durations and, after frame transformer A, the curves `log_f0`, `voiced`,
//! `energy`) is edited in three stages, each where its inputs exist in the model:
//! - [`Edit::durations`] after `encode`: rate, emphasis stretch, pause share;
//! - [`Edit::curves`] after `prosody`: register, range, declination, final movement, accent, emphasis, jitter, energy;
//! - [`Edit::cep`] before `vocode`: spectral tilt, breath (CPP), phonation (H1-H2) and a head gain for `energy_db`, as
//!   `cep_delta [T x 112]` (80 harmonic causal then 32 noise coefficients, natural-log cepstrum, added to the
//!   liftered cepstra); [`head_delta`] lays it out as the head's raw `cep_delta` (divided by the lifter).
//!
//! Spans are token ranges with keys `{name: N}` (N 0..10; emotions and delivery modes of `emotions.json`, `emph`,
//! author overrides). Every per-run quantity crossfades over 60 ms at run boundaries; see apply.py's docstring for
//! the full rules, which this file follows line by line.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use serde_json::Value;

/// The 12 parameters, in apply.py's order.
pub const PARAMS: [&str; 12] = [
    "register_st",
    "range_x",
    "declination_st_s",
    "final_st",
    "accent_st",
    "jitter_x",
    "rate_x",
    "pause",
    "energy_db",
    "tilt_db_oct",
    "cpp_db",
    "h1h2_db",
];
const REGISTER: usize = 0;
const RANGE: usize = 1;
const DECLINATION: usize = 2;
const FINAL: usize = 3;
const ACCENT: usize = 4;
const JITTER: usize = 5;
const RATE: usize = 6;
const PAUSE: usize = 7;
const ENERGY: usize = 8;
const TILT: usize = 9;
const CPP: usize = 10;
const H1H2: usize = 11;

/// One value per parameter.
pub type Params = [f64; 12];

fn is_ratio(i: usize) -> bool {
    matches!(i, RANGE | JITTER | RATE)
}
fn neutral(i: usize) -> f64 {
    if is_ratio(i) { 1.0 } else { 0.0 }
}
fn param_index(name: &str) -> Option<usize> {
    PARAMS.iter().position(|p| *p == name)
}

const XFADE_S: f64 = 0.060;
const ST: f64 = std::f64::consts::LN_2 / 12.0;
const ENERGY_PER_DB: f64 = 0.0576;
const SMOOTH_HALF: usize = 4;
const BUMP_HALF: usize = 3;
const FINAL_START: f64 = 0.75;
const FINAL_NORM: f64 = 0.62;
const JITTER_ST: f64 = 0.40;
const ENERGY_GATE: (f64, f64) = (-3.5, 0.5);
const EMPH_DUR_X: f64 = 0.12;
const EMPH_F0_ST: f64 = 1.5;
const EMPH_ENERGY_DB: f64 = 2.0;
const PAUSE_MIN_FRAMES: f64 = 8.0;
/// Energy of a silent pause token's frames.
pub const PAUSE_SILENCE: f64 = -4.0;
const PAUSE_SIL_MIN: u32 = 3;
const ENERGY_HEAD: f64 = 1.0;
const CPP_NOISE_DB: f64 = 1.0;
/// Realisation gains (apply.py `CAL`), in the internal domain (ratios as ln x).
const CAL: Params = [1.0, 1.3, 1.0, 1.0, 2.1, 1.0, 1.0, 1.0, 1.0, 1.145, 3.1, 1.21];
const TILT_BAND_HZ: (f64, f64) = (500.0, 5000.0);
/// Harmonic causal coefficients of `cep_delta`.
pub const CEP_CAUSAL: usize = 80;
/// Noise coefficients of `cep_delta`.
pub const CEP_NOISE: usize = 32;
/// Width of one `cep_delta` row.
pub const CEP_WIDTH: usize = CEP_CAUSAL + CEP_NOISE;
const SYNTH_FFT: usize = 512;
const F0_MIN: f64 = 50.0;
const F0_MAX: f64 = 600.0;
const F0_REF: f64 = 200.0;

// tiny-voca symbol ids (dizione symbols.rs order)
fn is_edge(t: u32) -> bool {
    t <= 2
}
fn is_space(t: u32) -> bool {
    t == 3
}
fn is_punct(t: u32) -> bool {
    (5..17).contains(&t)
}
fn is_vowel(t: u32) -> bool {
    (43..76).contains(&t)
}
fn is_phone(t: u32) -> bool {
    (17..76).contains(&t) || t == 4
}

#[derive(Clone, Copy)]
enum Op {
    Mul,
    Add,
    Sub,
}
/// Author overrides that are not a parameter name (a parameter name replaces the value).
const AUTHOR: [(&str, usize, Op); 8] = [
    ("rate", RATE, Op::Mul),
    ("pitch", REGISTER, Op::Add),
    ("range", RANGE, Op::Mul),
    ("breath", CPP, Op::Sub),
    ("energy", ENERGY, Op::Add),
    ("loud", ENERGY, Op::Add),
    ("pause", PAUSE, Op::Add),
    ("tilt", TILT, Op::Add),
];

fn is_emph_key(k: &str) -> bool {
    k == "emph" || k == "emphasis"
}

/// True for an author override key (a parameter name or one of rate, pitch, range, breath, energy, loud, pause,
/// tilt): its value is a value, not a 0..10 level.
pub fn is_override(key: &str) -> bool {
    param_index(key).is_some() || AUTHOR.iter().any(|a| a.0 == key)
}

/// True for an emphasis key (`emph`, `emphasis`).
pub fn is_emphasis(key: &str) -> bool {
    is_emph_key(key)
}

/// `emotions.json` v2, with every preset resolved to its anchors at N = 5, 8 and 10 (internal domain).
#[derive(Clone, Debug)]
pub struct Emotions {
    aliases: HashMap<String, String>,
    caps: Vec<(usize, f64, f64)>,
    anchors: HashMap<String, [Params; 3]>,
}

fn internal(d: Option<&serde_json::Map<String, Value>>) -> Params {
    let mut out = [0.0; 12];
    for (i, p) in PARAMS.iter().enumerate() {
        let x = d.and_then(|d| d.get(*p)).and_then(Value::as_f64).unwrap_or(neutral(i));
        out[i] = if is_ratio(i) { libm::log(x.max(1e-6)) } else { x };
    }
    out
}

fn sorted_numbers(v: Option<&Value>) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = v
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, x)| x.as_f64().map(|x| (k.clone(), x))).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

impl Emotions {
    /// Reads `emotions.json` v2 and resolves every preset (mixes, preset overrides). A mix cycle or a mix part that
    /// is not a preset is an error.
    pub fn parse(text: &str) -> Result<Emotions, String> {
        let doc: Value = serde_json::from_str(text).map_err(|e| format!("emotions.json: {e}"))?;
        Emotions::from_value(&doc)
    }

    pub fn from_value(doc: &Value) -> Result<Emotions, String> {
        let presets = doc.get("emotions").and_then(Value::as_object).ok_or("emotions.json: no emotions")?;
        let level = |k: &str, d: f64| doc.get("levels").and_then(|l| l.get(k)).and_then(Value::as_f64).unwrap_or(d);
        let (g8, g10) = (level("8", 1.4), level("10", 1.8));
        let aliases: HashMap<String, String> = doc
            .get("aliases")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
            .unwrap_or_default();
        let mut caps = Vec::new();
        if let Some(m) = doc.get("caps").and_then(Value::as_object) {
            for (p, range) in m {
                let pair = range.as_array().filter(|a| a.len() == 2).and_then(|a| Some((a[0].as_f64()?, a[1].as_f64()?)));
                match (param_index(p), pair) {
                    (Some(i), Some((lo, hi))) => caps.push((i, lo, hi)),
                    (None, _) => {}
                    (Some(_), None) => return Err(format!("emotions.json: caps.{p} must be [lo, hi]")),
                }
            }
        }
        let mut e = Emotions { aliases, caps, anchors: HashMap::new() };
        let names: Vec<String> = presets.keys().cloned().collect();
        for name in names {
            let mut stack = Vec::new();
            e.resolve(presets, &name, g8, g10, &mut stack)?;
        }
        Ok(e)
    }

    fn resolve(&mut self, presets: &serde_json::Map<String, Value>, key: &str, g8: f64, g10: f64, stack: &mut Vec<String>) -> Result<[Params; 3], String> {
        if let Some(a) = self.anchors.get(key) {
            return Ok(*a);
        }
        if stack.iter().any(|s| s == key) {
            return Err(format!("emotions.json: mix cycle {} -> {key}", stack.join(" -> ")));
        }
        let e = presets.get(key).and_then(Value::as_object).ok_or_else(|| format!("emotions.json: no preset {key}"))?;
        let a = if let Some(mix) = e.get("mix") {
            stack.push(key.to_string());
            let mut a = [[0.0; 12]; 3];
            for (part, w) in sorted_numbers(Some(mix)) {
                let name = self.canon_in(presets, &part).unwrap_or(part);
                let parts = self.resolve(presets, &name, g8, g10, stack)?;
                for (ai, pi) in a.iter_mut().zip(parts) {
                    for j in 0..12 {
                        ai[j] += w * pi[j];
                    }
                }
            }
            stack.pop();
            for (p, v) in sorted_numbers(e.get("overrides")) {
                if let Some(j) = param_index(&p) {
                    let x = if is_ratio(j) { libm::log(v.max(1e-6)) } else { v };
                    a[0][j] = x;
                    a[1][j] = g8 * x;
                    a[2][j] = g10 * x;
                }
            }
            a
        } else {
            let a5 = internal(Some(e));
            let scaled = |g: f64| a5.map(|x| g * x);
            let a8 = e.get("at8").and_then(Value::as_object).map_or_else(|| scaled(g8), |m| internal(Some(m)));
            let a10 = e.get("at10").and_then(Value::as_object).map_or_else(|| scaled(g10), |m| internal(Some(m)));
            [a5, a8, a10]
        };
        self.anchors.insert(key.to_string(), a);
        Ok(a)
    }

    fn canon_in(&self, presets: &serde_json::Map<String, Value>, name: &str) -> Option<String> {
        let n = name.trim().to_lowercase();
        let n = self.aliases.get(&n).cloned().unwrap_or(n);
        presets.contains_key(&n).then_some(n)
    }

    /// The canonical key of `name` (trimmed, lower-cased, aliases folded), or `None` when it is not a preset.
    pub fn canon(&self, name: &str) -> Option<String> {
        let n = name.trim().to_lowercase();
        let n = self.aliases.get(&n).cloned().unwrap_or(n);
        self.anchors.contains_key(&n).then_some(n)
    }

    /// The preset keys, sorted.
    pub fn keys(&self) -> Vec<&str> {
        let mut k: Vec<&str> = self.anchors.keys().map(String::as_str).collect();
        k.sort_unstable();
        k
    }

    /// Internal vector (ratios as ln x) of preset `key` at intensity `n` (0..10, clipped).
    pub fn at_level(&self, key: &str, n: f64) -> Params {
        let Some([a5, a8, a10]) = self.anchors.get(key) else { return [0.0; 12] };
        let n = n.clamp(0.0, 10.0);
        let mut out = [0.0; 12];
        for j in 0..12 {
            out[j] = if n <= 5.0 {
                a5[j] * (n / 5.0)
            } else if n <= 8.0 {
                a5[j] + (a8[j] - a5[j]) * ((n - 5.0) / 3.0)
            } else {
                a8[j] + (a10[j] - a8[j]) * ((n - 8.0) / 2.0)
            };
        }
        out
    }

    /// The parameters (ratios as ratios) of a span's keys: presets summed in the internal domain, then the author
    /// overrides, then the caps (apply.py `preset_of`).
    pub fn preset_of(&self, keys: &[(String, f64)]) -> Params {
        let mut sorted: Vec<(String, f64)> = keys.to_vec();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut v = [0.0; 12];
        let mut over: Vec<(String, f64)> = Vec::new();
        for (name, n) in &sorted {
            let k = name.trim().to_lowercase();
            if is_emph_key(&k) || k == "neutral" {
                continue;
            }
            if let Some(c) = self.canon(&k) {
                let l = self.at_level(&c, *n);
                for j in 0..12 {
                    v[j] += l[j];
                }
            } else if is_override(&k) {
                over.push((k, *n));
            }
        }
        let mut out = [0.0; 12];
        for j in 0..12 {
            out[j] = if is_ratio(j) { libm::exp(v[j]) } else { v[j] };
        }
        for (k, val) in over {
            if let Some(j) = param_index(&k) {
                out[j] = val;
            } else if let Some(&(_, j, op)) = AUTHOR.iter().find(|a| a.0 == k) {
                out[j] = match op {
                    Op::Mul => out[j] * val,
                    Op::Add => out[j] + val,
                    Op::Sub => out[j] - val,
                };
            }
        }
        for &(j, lo, hi) in &self.caps {
            out[j] = out[j].max(lo).min(hi);
        }
        out
    }
}

/// The `emotions.json` this crate ships (agent B's presets, with their source and licence fields).
pub const BUILTIN_JSON: &str = include_str!("emotions.json");

static INSTALLED: LazyLock<RwLock<Emotions>> =
    LazyLock::new(|| RwLock::new(Emotions::parse(BUILTIN_JSON).expect("the shipped emotions.json parses")));

/// Replaces the presets every `speak` node uses (a host-loaded `emotions.json` v2).
pub fn install(emotions: Emotions) {
    *INSTALLED.write().unwrap_or_else(|p| p.into_inner()) = emotions;
}

/// Runs `f` on the presets in use (the shipped ones unless the host installed others).
pub fn with_emotions<R>(f: impl FnOnce(&Emotions) -> R) -> R {
    f(&INSTALLED.read().unwrap_or_else(|p| p.into_inner()))
}

/// A span of a performance: tokens `[from, to)` and the keys the author wrote (any case, aliases, overrides).
#[derive(Clone, Debug, PartialEq)]
pub struct TokenSpan {
    pub from: usize,
    pub to: usize,
    pub keys: Vec<(String, f64)>,
}

/// The per-frame curves of a performance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curves {
    pub log_f0: Vec<f64>,
    pub voiced: Vec<f64>,
    pub energy: Vec<f64>,
}

#[derive(Clone, Debug)]
struct Unit {
    from: usize,
    emph: f64,
}

#[derive(Clone, Debug)]
struct Run {
    from: usize,
    to: usize,
    key: Vec<(String, f64)>,
}

/// The edit of one performance: its spans resolved into runs and parameter vectors. Build it once per utterance
/// and run the stages where the model gives their inputs.
#[derive(Clone, Debug)]
pub struct Edit {
    spans: Vec<Unit>,
    runs: Vec<Run>,
    vecs: Vec<Params>,
    width: f64,
    rate: f64,
    hop: f64,
}

/// apply.py `_weights`: partition weights `[units][t]`, unit k fading into k + 1 around `starts[k + 1]`.
fn weights(starts: &[f64], t: &[f64], width: f64) -> Vec<Vec<f64>> {
    let mut w = vec![vec![0.0; t.len()]; starts.len()];
    if starts.is_empty() {
        return w;
    }
    w[0].fill(1.0);
    for k in 1..starts.len() {
        for (i, &ti) in t.iter().enumerate() {
            let a = ((ti - starts[k] + width / 2.0) / width).clamp(0.0, 1.0);
            w[k][i] = a;
            for row in w.iter_mut().take(k) {
                row[i] *= 1.0 - a;
            }
        }
    }
    w
}

/// Σ_k w[k][i] q[k].
fn mix(w: &[Vec<f64>], q: impl Fn(usize) -> f64, n: usize) -> Vec<f64> {
    let mut out = vec![0.0; n];
    for (k, row) in w.iter().enumerate() {
        let qk = q(k);
        for (o, &wk) in out.iter_mut().zip(row) {
            *o += wk * qk;
        }
    }
    out
}

/// apply.py `_box`: a (2 half + 1)-frame mean with the ends held.
fn boxed(x: &[f64], half: usize) -> Vec<f64> {
    let n = x.len();
    let h = half as isize;
    (0..n as isize)
        .map(|i| (-h..=h).map(|o| x[(i + o).clamp(0, n as isize - 1) as usize]).sum::<f64>() / (2 * half + 1) as f64)
        .collect()
}

/// apply.py `_noise`: unit-variance noise per frame, xorshift32 of (frame * 2654435761 + 12345), 4 uniforms summed.
fn frame_noise(count: usize) -> Vec<f64> {
    (0..count)
        .map(|i| {
            let mut x = (i as u32).wrapping_mul(2_654_435_761).wrapping_add(12_345);
            let mut acc = 0.0;
            for _ in 0..4 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                acc += x as f64 / 4_294_967_296.0;
            }
            (acc - 2.0) * 3f64.sqrt()
        })
        .collect()
}

fn starts_of(d: &[u32]) -> Vec<f64> {
    let mut s = Vec::with_capacity(d.len() + 1);
    let mut acc = 0u64;
    s.push(0.0);
    for &x in d {
        acc += x as u64;
        s.push(acc as f64);
    }
    s
}

/// The least-squares line `slope t + icpt` through `(t, y)`.
fn line_fit(t: &[f64], y: &[f64]) -> (f64, f64) {
    let n = t.len() as f64;
    let (tm, ym) = (t.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut sty, mut stt) = (0.0, 0.0);
    for (&a, &b) in t.iter().zip(y) {
        sty += (a - tm) * (b - ym);
        stt += (a - tm) * (a - tm);
    }
    let slope = if stt > 0.0 { sty / stt } else { 0.0 };
    (slope, ym - slope * tm)
}

impl Edit {
    /// Resolves `spans` (sorted by `from`, not overlapping; uncovered tokens join the span before) over `n_tokens`
    /// tokens. `None` when there is no span: the performance stays as it is.
    pub fn new(n_tokens: usize, spans: &[TokenSpan], emotions: &Emotions, rate: f64, hop: f64) -> Result<Option<Edit>, String> {
        let mut sp: Vec<&TokenSpan> = spans.iter().collect();
        sp.sort_by_key(|s| s.from);
        if sp.is_empty() {
            return Ok(None);
        }
        let mut units = Vec::new();
        let mut keyed: Vec<Vec<(String, f64)>> = Vec::new();
        let mut bounds = Vec::new();
        for (i, s) in sp.iter().enumerate() {
            let a = if i == 0 { 0 } else { s.from };
            let b = if i == sp.len() - 1 { n_tokens } else { sp[i + 1].from };
            if i > 0 && s.from < sp[i - 1].to {
                return Err("spans overlap".into());
            }
            if b <= a {
                continue;
            }
            // a Python dict of the lower-cased keys: the first position, the last value
            let mut keys: Vec<(String, f64)> = Vec::new();
            for (k, v) in &s.keys {
                let k = k.trim().to_lowercase();
                match keys.iter_mut().find(|(n, _)| *n == k) {
                    Some(slot) => slot.1 = *v,
                    None => keys.push((k, *v)),
                }
            }
            let emph = keys.iter().find(|(k, _)| is_emph_key(k)).map_or(0.0, |(_, n)| n.clamp(0.0, 10.0) / 5.0);
            units.push(Unit { from: a, emph });
            keyed.push(keys);
            bounds.push((a, b));
        }
        if units.is_empty() {
            return Ok(None);
        }
        let mut runs: Vec<Run> = Vec::new();
        for (keys, &(a, b)) in keyed.iter().zip(&bounds) {
            let mut key: Vec<(String, f64)> = keys.iter().filter(|(k, _)| !is_emph_key(k)).cloned().collect();
            key.sort_by(|x, y| x.0.cmp(&y.0));
            match runs.last_mut() {
                Some(last) if last.key == key => last.to = b,
                _ => runs.push(Run { from: a, to: b, key }),
            }
        }
        let vecs = runs
            .iter()
            .map(|r| {
                let p = emotions.preset_of(&r.key);
                let mut v = [0.0; 12];
                for j in 0..12 {
                    v[j] = (if is_ratio(j) { libm::log(p[j].max(1e-6)) } else { p[j] }) * CAL[j];
                }
                v
            })
            .collect();
        Ok(Some(Edit { spans: units, runs, vecs, width: XFADE_S * rate / hop, rate, hop }))
    }

    /// The resolved parameters of each run (internal domain times the realisation gains), with its token range.
    pub fn runs(&self) -> Vec<(usize, usize, Params)> {
        self.runs.iter().zip(&self.vecs).map(|(r, v)| (r.from, r.to, *v)).collect()
    }

    /// The durations stage: rate, emphasis stretch and pause share, rounded on the cumulative sum.
    pub fn durations(&self, tokens: &[u32], durations: &[u32]) -> Vec<u32> {
        let n = durations.len();
        let d: Vec<f64> = durations.iter().map(|&x| x as f64).collect();
        let starts = starts_of(durations);
        let centre: Vec<f64> = (0..n).map(|j| starts[j] + d[j] / 2.0).collect();
        let wr = weights(&self.runs.iter().map(|r| starts[r.from]).collect::<Vec<_>>(), &centre, self.width);
        let ws = weights(&self.spans.iter().map(|s| starts[s.from]).collect::<Vec<_>>(), &centre, self.width);
        let ln_rate = mix(&wr, |k| self.vecs[k][RATE], n);
        let emph = mix(&ws, |k| self.spans[k].emph, n);
        let mut new: Vec<f64> = (0..n)
            .map(|j| d[j] * libm::exp(-ln_rate[j]) * if is_phone(tokens[j]) { 1.0 + EMPH_DUR_X * emph[j] } else { 1.0 })
            .collect();
        let last_real = (0..n).rev().find(|&j| !is_edge(tokens[j]));
        for (r, v) in self.runs.iter().zip(&self.vecs) {
            let dp = v[PAUSE];
            if dp == 0.0 {
                continue;
            }
            let js: Vec<usize> =
                (r.from..r.to).filter(|&j| (is_punct(tokens[j]) || is_space(tokens[j])) && Some(j) != last_real).collect();
            if js.is_empty() {
                continue;
            }
            let total: f64 = (r.from..r.to).filter(|&j| !is_edge(tokens[j])).map(|j| new[j]).sum();
            if dp > 0.0 {
                let add = dp * total;
                let mut order = js.clone();
                let key = |j: usize| (if is_punct(tokens[j]) { 0 } else { 1 }, -new[j], j);
                order.sort_by(|&a, &b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
                let k = (order.len() as f64).min((add / PAUSE_MIN_FRAMES).ceil().max(1.0)) as usize;
                for &j in &order[..k] {
                    new[j] += add / k as f64;
                }
            } else {
                let cur: Vec<f64> = js.iter().map(|&j| new[j]).collect();
                let sum: f64 = cur.iter().sum();
                if sum > 0.0 {
                    let take = sum.min(-dp * total);
                    for (&j, &c) in js.iter().zip(&cur) {
                        new[j] -= take * c / sum;
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(n);
        let (mut acc, mut prev) = (0.0f64, 0.0f64);
        for j in 0..n {
            acc += new[j];
            let end = (acc + 0.5).floor();
            let mut x = end - prev;
            prev = end;
            if is_phone(tokens[j]) && durations[j] > 0 && x < 1.0 {
                x = 1.0;
            }
            out.push(x.max(0.0) as u32);
        }
        out
    }

    /// The curves stage on the frame grid of `durations` (the edited ones), in place.
    pub fn curves(&self, tokens: &[u32], stress: &[u32], durations: &[u32], c: &mut Curves) -> Result<(), String> {
        let t_len: usize = durations.iter().map(|&d| d as usize).sum();
        if c.log_f0.len() != t_len || c.voiced.len() != t_len || c.energy.len() != t_len {
            return Err(format!("curves have {} frames, durations sum to {t_len}", c.log_f0.len()));
        }
        let starts = starts_of(durations);
        let t: Vec<f64> = (0..t_len).map(|i| i as f64 + 0.5).collect();
        let wr = weights(&self.runs.iter().map(|r| starts[r.from]).collect::<Vec<_>>(), &t, self.width);
        let ws = weights(&self.spans.iter().map(|s| starts[s.from]).collect::<Vec<_>>(), &t, self.width);
        let mut owner = Vec::with_capacity(t_len);
        for (j, &d) in durations.iter().enumerate() {
            owner.extend(std::iter::repeat_n(j, d as usize));
        }
        let p = &c.log_f0;
        let voiced: Vec<bool> = c.voiced.iter().map(|&v| v > 0.5).collect();
        let e = &c.energy;
        let sec: Vec<f64> = (0..t_len).map(|i| (i as f64 * self.hop) / self.rate).collect();
        let stressed: Vec<f64> =
            owner.iter().map(|&j| if is_vowel(tokens[j]) && stress[j] == 1 { 1.0 } else { 0.0 }).collect();
        let bump = boxed(&stressed, BUMP_HALF);
        let mut delta = vec![0.0; t_len];
        let mut dr = vec![0.0; t_len];
        for (k, (r, v)) in self.runs.iter().zip(&self.vecs).enumerate() {
            let (a, b) = (starts[r.from] as usize, starts[r.to] as usize);
            let idx: Vec<usize> = (a..b).filter(|&i| voiced[i]).collect();
            for i in 0..t_len {
                dr[i] = v[REGISTER] * ST + v[ACCENT] * ST * bump[i];
            }
            if idx.len() >= 4 {
                let ts: Vec<f64> = idx.iter().map(|&i| sec[i]).collect();
                let ys: Vec<f64> = idx.iter().map(|&i| p[i]).collect();
                let (slope, icpt) = line_fit(&ts, &ys);
                let t_mean = ts.iter().sum::<f64>() / ts.len() as f64;
                let widen = libm::exp(v[RANGE]) - 1.0;
                let (f, l) = (idx[0] as f64, *idx.last().unwrap() as f64);
                let span = (l - f).max(1.0);
                for i in 0..t_len {
                    dr[i] += (p[i] - (slope * sec[i] + icpt)) * widen;
                    dr[i] += v[DECLINATION] * ST * (sec[i] - t_mean);
                    let x = (i as f64 - f) / span;
                    dr[i] += v[FINAL] * ST * ((x - FINAL_START) / (1.0 - FINAL_START)).clamp(0.0, 1.0) / FINAL_NORM;
                }
            }
            for i in 0..t_len {
                delta[i] += wr[k][i] * dr[i];
            }
        }
        let emph = mix(&ws, |k| self.spans[k].emph, t_len);
        let p1: Vec<f64> = (0..t_len).map(|i| p[i] + delta[i] + emph[i] * EMPH_F0_ST * ST).collect();
        let lnj = mix(&wr, |k| self.vecs[k][JITTER], t_len);
        let sm = boxed(&p1, SMOOTH_HALF);
        let nz: Vec<f64> = boxed(&frame_noise(t_len), 1).iter().map(|x| x * 3f64.sqrt()).collect();
        let p2: Vec<f64> = (0..t_len)
            .map(|i| {
                let kj = libm::exp(lnj[i]);
                sm[i] + (p1[i] - sm[i]) * kj.min(1.0) + JITTER_ST * ST * (kj * kj - 1.0).max(0.0).sqrt() * nz[i]
            })
            .collect();
        let edb_runs = mix(&wr, |k| self.vecs[k][ENERGY], t_len);
        let mut e2: Vec<f64> = (0..t_len)
            .map(|i| {
                let gate = ((e[i] - ENERGY_GATE.0) / ENERGY_GATE.1).clamp(0.0, 1.0);
                e[i] + ENERGY_PER_DB * (edb_runs[i] + emph[i] * EMPH_ENERGY_DB) * gate
            })
            .collect();
        let mut sil = vec![0.0; t_len];
        for (r, v) in self.runs.iter().zip(&self.vecs) {
            if v[PAUSE] <= 0.0 {
                continue;
            }
            for j in r.from..r.to {
                if (is_punct(tokens[j]) || is_space(tokens[j])) && durations[j] >= PAUSE_SIL_MIN {
                    sil[starts[j] as usize..starts[j + 1] as usize].fill(1.0);
                }
            }
        }
        if sil.iter().any(|&s| s > 0.0) {
            let m = boxed(&sil, 1);
            for i in 0..t_len {
                e2[i] += (e2[i].min(PAUSE_SILENCE) - e2[i]) * m[i];
                if sil[i] > 0.0 {
                    c.voiced[i] = 0.0;
                }
            }
        }
        c.log_f0 = p2;
        c.energy = e2;
        Ok(())
    }

    /// The cepstral stage: `cep_delta [T x 112]` (row-major) for the head, from the curves' `log_f0`.
    pub fn cep(&self, durations: &[u32], log_f0: &[f64]) -> Vec<f64> {
        let t_len = log_f0.len();
        let starts = starts_of(durations);
        let t: Vec<f64> = (0..t_len).map(|i| i as f64 + 0.5).collect();
        let wr = weights(&self.runs.iter().map(|r| starts[r.from]).collect::<Vec<_>>(), &t, self.width);
        let ws = weights(&self.spans.iter().map(|s| starts[s.from]).collect::<Vec<_>>(), &t, self.width);
        let tilt = mix(&wr, |k| self.vecs[k][TILT], t_len);
        let cpp = mix(&wr, |k| self.vecs[k][CPP], t_len);
        let h1h2 = mix(&wr, |k| self.vecs[k][H1H2], t_len);
        let emph = mix(&ws, |k| self.spans[k].emph, t_len);
        let energy: Vec<f64> = mix(&wr, |k| self.vecs[k][ENERGY], t_len).iter().zip(&emph).map(|(a, b)| a + b * EMPH_ENERGY_DB).collect();
        let f0: Vec<f64> = log_f0.iter().map(|&l| F0_REF * libm::exp(l)).collect();
        cep_from_controls(&tilt, &cpp, &h1h2, &f0, self.rate, Some(&energy))
    }
}

/// apply.py `_resample`: curves of the `old` frame grid onto the `new` one, token by token (linear inside a token's
/// block; a token that had no frames gets a ramp of log_f0 between its neighbours, unvoiced, at the lowest energy).
pub fn resample(c: &Curves, old: &[u32], new: &[u32]) -> Curves {
    let (lf, vo, en) = (&c.log_f0, &c.voiced, &c.energy);
    let sil = if en.is_empty() { -4.0 } else { en.iter().copied().fold(f64::INFINITY, f64::min) };
    let mut out = Curves::default();
    let mut s = 0usize;
    for (&n_old, &n_new) in old.iter().zip(new) {
        let (n_old_u, n_new_u) = (n_old as usize, n_new as usize);
        if n_new_u > 0 {
            if n_old_u > 0 {
                for k in 0..n_new_u {
                    let x = (((k as f64 + 0.5) * n_old as f64) / n_new as f64 - 0.5).clamp(0.0, n_old as f64 - 1.0);
                    let i0 = x.floor() as usize;
                    let i1 = (i0 + 1).min(n_old_u - 1);
                    let fr = x - i0 as f64;
                    out.log_f0.push(lf[s + i0] * (1.0 - fr) + lf[s + i1] * fr);
                    out.voiced.push(vo[s + i0] * (1.0 - fr) + vo[s + i1] * fr);
                    out.energy.push(en[s + i0] * (1.0 - fr) + en[s + i1] * fr);
                }
            } else {
                let a = if s > 0 { lf[s - 1] } else if s < lf.len() { lf[s] } else { 0.0 };
                let b = if s < lf.len() { lf[s] } else { a };
                for k in 0..n_new_u {
                    let fr = (k as f64 + 1.0) / (n_new as f64 + 1.0);
                    out.log_f0.push(a * (1.0 - fr) + b * fr);
                    out.voiced.push(0.0);
                    out.energy.push(sil);
                }
            }
        }
        s += n_old_u;
    }
    for v in &mut out.voiced {
        *v = if *v >= 0.5 { 1.0 } else { 0.0 };
    }
    out
}

/// apply.py `_dct_rows`: least-squares (trapezoid-weighted, exact DCT-I) projection of a log magnitude on bins
/// 0..N onto cos(k w), `[bins][coefs]`.
fn dct_rows(bins: usize, coefs: usize) -> Vec<f64> {
    let n = (bins - 1) as f64;
    let mut p = vec![0.0; bins * coefs];
    for k in 0..bins {
        let w = std::f64::consts::PI * k as f64 / n;
        let trap = if k == 0 || k == bins - 1 { 0.5 } else { 1.0 };
        for c in 0..coefs {
            let mut x = libm::cos(w * c as f64) * trap * (2.0 / n);
            if c == 0 {
                x *= 0.5;
            }
            p[k * coefs + c] = x;
        }
    }
    p
}

/// apply.py `cep_from_controls`: per-frame controls to `cep_delta [T x 112]`. `tilt` dB/octave about 1 kHz inside
/// 0.5-5 kHz (harmonic branch); `h1h2` dB at f <= f0 ramping to 0 at 2 f0 in log frequency (harmonic branch); `cpp`
/// a noise branch gain of -cpp dB; `energy` dB on coefficient 0 of both branches.
pub fn cep_from_controls(tilt: &[f64], cpp: &[f64], h1h2: &[f64], f0_hz: &[f64], rate: f64, energy: Option<&[f64]>) -> Vec<f64> {
    let nb = SYNTH_FFT / 2 + 1;
    let hz: Vec<f64> = (0..nb).map(|k| (k as f64 * rate) / SYNTH_FFT as f64).collect();
    let db2ln = std::f64::consts::LN_10 / 20.0;
    let ph = dct_rows(nb, CEP_CAUSAL);
    let tilt_shape: Vec<f64> = hz.iter().map(|&h| libm::log2(h.clamp(TILT_BAND_HZ.0, TILT_BAND_HZ.1) / 1000.0) * db2ln).collect();
    let mut tilt_coef = [0.0; CEP_CAUSAL];
    for k in 0..nb {
        for c in 0..CEP_CAUSAL {
            tilt_coef[c] += tilt_shape[k] * ph[k * CEP_CAUSAL + c];
        }
    }
    let frames = f0_hz.len();
    let mut out = vec![0.0; frames * CEP_WIDTH];
    let mut shelf = vec![0.0; nb];
    for t in 0..frames {
        let f0 = f0_hz[t].clamp(F0_MIN, F0_MAX);
        for k in 0..nb {
            let r = hz[k].max(1.0) / f0;
            shelf[k] = libm::log2(2.0 / r).clamp(0.0, 1.0) * db2ln;
        }
        let row = &mut out[t * CEP_WIDTH..(t + 1) * CEP_WIDTH];
        for c in 0..CEP_CAUSAL {
            let mut s = 0.0;
            for k in 0..nb {
                s += shelf[k] * ph[k * CEP_CAUSAL + c];
            }
            row[c] = tilt[t] * tilt_coef[c] + h1h2[t] * s;
        }
        row[CEP_CAUSAL] = -CPP_NOISE_DB * cpp[t] * db2ln;
        if let Some(e) = energy {
            let g = ENERGY_HEAD * e[t] * db2ln;
            row[0] += g;
            row[CEP_CAUSAL] += g;
        }
    }
    out
}

/// tiny-voca's cepstral lifter (head.rs): 2 for c0, 1 / (1 + n / 4) after.
fn lifter(n: usize) -> f64 {
    if n == 0 { 2.0 } else { 1.0 / (1.0 + n as f64 / 4.0) }
}

/// `cep_delta [T x 112]` (liftered, natural log) as the head's raw `cep_delta`: harmonic rows `[T x hw]` and noise
/// rows `[T x nw]`, the first 80 / 32 coefficients divided by the lifter, the anticausal and gain columns 0.
/// This is how the reference render adds it (`CepAdd`: `h[:80] += delta / causal_lifter`).
pub fn head_delta(delta: &[f64], hw: usize, nw: usize) -> tinyvoca::Cepstra {
    let frames = delta.len() / CEP_WIDTH;
    let mut harmonic = vec![0.0f32; frames * hw];
    let mut noise = vec![0.0f32; frames * nw];
    for t in 0..frames {
        let row = &delta[t * CEP_WIDTH..(t + 1) * CEP_WIDTH];
        for c in 0..CEP_CAUSAL.min(hw) {
            harmonic[t * hw + c] = (row[c] / lifter(c)) as f32;
        }
        for c in 0..CEP_NOISE.min(nw) {
            noise[t * nw + c] = (row[CEP_CAUSAL + c] / lifter(c)) as f32;
        }
    }
    tinyvoca::Cepstra { harmonic, noise }
}

/// One semitone in ln f0.
pub fn semitone() -> f64 {
    ST
}

/// The energy condition per dB (tiny-voca energy = (ln rms - mean) / 2).
pub fn energy_per_db() -> f64 {
    ENERGY_PER_DB
}
