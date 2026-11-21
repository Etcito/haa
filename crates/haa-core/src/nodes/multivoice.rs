//! `multivoice` source node: HAA voice, the multivoice student (four ONNX graphs the host runs, on WebGPU in a
//! browser; their weights are fetched at run time, never bundled). Everything around the graphs is
//! crate::multivoice.
//!
//! Every input comes from the graph: the voice (age 0..63 = 8..71 years, sex 0..15, weight 0..63 = 20..150 kg, the
//! voice seed, rough / breath / tremble 0..255), the text with its inline tags (crate::multivoice::plan; the
//! emotion comes from the tags only, text before the first tag is neutral), the intensity of a tag written without a
//! level, the speech noise, the event noise and the speed. Four params are overrides:
//! `duration` (a factor on the predicted token durations), `f0` (Hz), `voiced` (0..1) and `energy` (dB). They act
//! only when an Automation curve is wired into them: then, as any modulated param, their value at patch time t is
//! clamp(base + curve(t) x amount, min, max), and it replaces the predicted value between the model's stages
//! (validate attaches the wired curves to the node, [`overrides`]).
//!
//! Playback is the `speak` node's: the speech is rendered utterance by utterance (one per sentence) by a host with
//! the model, and each part lands in the process-wide clip store (crate::nodes::speak::put_part) under the node's
//! [`clip_key`]; the node plays the parts there at the patch clock and renders nothing itself, so the AudioWorklet
//! player and the offline renders need no model. [`plan_patch`] lists what a patch needs, [`MvJobs`] hands the
//! host one part at a time with the encoder inputs ready, then takes the graphs' outputs back between stages.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::multivoice::{self as mv, Drive, Emotion, EmotionInput, Expanded, Override, Overrides, Part, Tokens};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::nodes::speak::{clip, put_part};

pub const TEXT: usize = 0;
pub const INTENSITY: usize = 1;
pub const AGE: usize = 2;
pub const SEX: usize = 3;
pub const WEIGHT: usize = 4;
pub const SEED: usize = 5;
pub const ROUGH: usize = 6;
pub const BREATH: usize = 7;
pub const TREMBLE: usize = 8;
pub const NOISE: usize = 9;
pub const EVENT_NOISE: usize = 10;
pub const SPEED: usize = 11;
pub const DURATION: usize = 12;
pub const F0: usize = 13;
pub const VOICED: usize = 14;
pub const ENERGY: usize = 15;
pub const GAIN: usize = 16;

/// The override params and the name of each in [`Overrides`].
pub const OVERRIDES: [(usize, &str); 4] = [(DURATION, "duration"), (F0, "f0"), (VOICED, "voiced"), (ENERGY, "energy")];
/// The key under which validate hands the wired curves to the node (`NodeInit::curve`).
pub const OVERRIDE_KEY: &str = "multivoiceOverrides";

const fn number(key: &'static str, min: f64, max: f64, default: f64, step: f64, scale: Scale, unit: &'static str, en: &'static str, fr: &'static str) -> ParamDef {
    ParamDef { key, kind: ParamKind::Number { min, max, default, step, scale }, unit, label_en: en, label_fr: fr, modulatable: false, smooth: false }
}

const fn wired(key: &'static str, min: f64, max: f64, default: f64, step: f64, scale: Scale, unit: &'static str, en: &'static str, fr: &'static str) -> ParamDef {
    ParamDef { key, kind: ParamKind::Number { min, max, default, step, scale }, unit, label_en: en, label_fr: fr, modulatable: true, smooth: false }
}

/// The `multivoice` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "multivoice",
    category: Category::Source,
    label_en: "Speak (HAA voice)",
    label_fr: "Parole (voix HAA)",
    audio_in: false,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef { key: "text", kind: ParamKind::Text { default: "" }, unit: "", label_en: "Text", label_fr: "Texte", modulatable: false, smooth: false },
        number("intensity", 0.0, 1.0, 0.9, 0.01, Scale::Lin, "", "Intensity", "Intensité"),
        number("age", 0.0, 63.0, 16.0, 1.0, Scale::Lin, "", "Age (0 = 8 y, 63 = 71 y)", "Âge (0 = 8 ans, 63 = 71 ans)"),
        number("sex", 0.0, 15.0, 1.0, 1.0, Scale::Lin, "", "Sex (0 male - 15 female)", "Sexe (0 homme - 15 femme)"),
        number("weight", 0.0, 63.0, 28.0, 1.0, Scale::Lin, "", "Weight (0 = 20 kg, 63 = 150 kg)", "Poids (0 = 20 kg, 63 = 150 kg)"),
        number("seed", 0.0, 99_999.0, 0.0, 1.0, Scale::Lin, "", "Voice seed", "Graine de voix"),
        number("rough", 0.0, 255.0, 0.0, 1.0, Scale::Lin, "", "Rough", "Raucité"),
        number("breath", 0.0, 255.0, 0.0, 1.0, Scale::Lin, "", "Breath", "Souffle"),
        number("tremble", 0.0, 255.0, 0.0, 1.0, Scale::Lin, "", "Tremble", "Tremblement"),
        number("noise", 0.0, 1.0, 0.333, 0.001, Scale::Lin, "", "Speech noise", "Bruit de parole"),
        number("eventNoise", 0.0, 2.0, 1.0, 0.01, Scale::Lin, "", "Event noise", "Bruit des sons"),
        number("speed", 0.5, 2.0, 1.0, 0.01, Scale::Log, "×", "Speed", "Vitesse"),
        wired("duration", 0.25, 4.0, 1.0, 0.01, Scale::Log, "×", "Duration override (curve)", "Durées imposées (courbe)"),
        wired("f0", 50.0, 500.0, 130.0, 0.1, Scale::Log, "Hz", "F0 override (curve)", "F0 imposée (courbe)"),
        wired("voiced", 0.0, 1.0, 1.0, 0.01, Scale::Lin, "", "Voicing override (curve)", "Voisement imposé (courbe)"),
        wired("energy", -80.0, 0.0, -30.0, 0.1, Scale::Lin, "dB", "Energy override (curve)", "Énergie imposée (courbe)"),
        ParamDef {
            key: "gain",
            kind: ParamKind::Number { min: 0.0, max: 2.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "×",
            label_en: "Gain",
            label_fr: "Gain",
            modulatable: true,
            smooth: true,
        },
    ],
    create,
};

/// What a `multivoice` node asked for when it configured: enough for a host with the model to render it.
#[derive(Clone, Debug, PartialEq)]
pub struct Wanted {
    pub key: String,
    pub text: String,
    /// The node params, `gain` excluded.
    pub params: Vec<f64>,
    pub overrides: Overrides,
}

static WANTED: Mutex<Vec<Wanted>> = Mutex::new(Vec::new());

fn wanted_list<R>(f: impl FnOnce(&mut Vec<Wanted>) -> R) -> R {
    f(&mut WANTED.lock().unwrap_or_else(|p| p.into_inner()))
}

/// The overrides wired into a node, read back from its `NodeInit::curve` (see [`overrides_json`]).
pub fn overrides(params: &[f64], curve: Option<&Value>) -> Overrides {
    let Some(o) = curve.and_then(|c| c.get(OVERRIDE_KEY)) else { return Overrides::default() };
    let mut out = Overrides { patch_seconds: o.get("patchSeconds").and_then(Value::as_f64).unwrap_or(1.0), ..Overrides::default() };
    for (index, name) in OVERRIDES {
        let Some(list) = o.get(name).and_then(Value::as_array) else { continue };
        let drives: Vec<Drive> = list
            .iter()
            .map(|d| Drive {
                points: d.get("points").and_then(Value::as_array).map_or_else(Vec::new, |ps| {
                    ps.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect()
                }),
                gain: d.get("gain").and_then(Value::as_f64).unwrap_or(0.0),
            })
            .collect();
        if drives.is_empty() {
            continue;
        }
        let ParamKind::Number { min, max, .. } = DEF.params[index].kind else { continue };
        let ov = Some(Override { base: params.get(index).copied().unwrap_or(0.0), min, max, drives });
        match name {
            "duration" => out.duration = ov,
            "f0" => out.f0 = ov,
            "voiced" => out.voiced = ov,
            _ => out.energy = ov,
        }
    }
    out
}

/// The `NodeInit::curve` value validate gives a node with curves wired into its overrides: per override name the
/// list of `{points, gain}`, and the patch duration.
pub fn overrides_json(wired: &[(&'static str, Vec<(f64, f64)>, f64)], patch_seconds: f64) -> Value {
    let mut o = serde_json::Map::new();
    for (name, points, gain) in wired {
        let entry = o.entry(name.to_string()).or_insert_with(|| json!([]));
        if let Some(list) = entry.as_array_mut() {
            list.push(json!({ "points": points.iter().map(|(x, y)| json!([x, y])).collect::<Vec<_>>(), "gain": gain }));
        }
    }
    o.insert("patchSeconds".into(), json!(patch_seconds));
    json!({ OVERRIDE_KEY: o })
}

/// What a node renders from: its text, params (`gain` excluded) and wired curves.
pub fn clip_key(text: &str, params: &[f64], curve: Option<&Value>) -> String {
    let voice: Vec<f64> = params.iter().take(GAIN).copied().collect();
    let wired = curve.and_then(|c| c.get(OVERRIDE_KEY)).map_or(String::new(), Value::to_string);
    format!("multivoice\u{1}{text}\u{1}{voice:?}\u{1}{wired}")
}

/// The speech a patch needs: one [`Wanted`] per distinct `multivoice` source, in node order. Nothing is rendered.
pub fn plan_patch(patch: &str, seed: u32) -> Result<Vec<Wanted>, String> {
    wanted_list(std::mem::take);
    let mut voice = crate::Voice::new(48_000.0, seed);
    let outcome = voice.load(patch, false);
    let mut out: Vec<Wanted> = Vec::new();
    for w in wanted_list(std::mem::take) {
        if !out.iter().any(|o| o.key == w.key) {
            out.push(w);
        }
    }
    if !outcome.playable {
        let first = outcome.issues.iter().find(|i| i.severity == crate::issues::Severity::Error);
        return Err(first.map_or("the patch cannot play".into(), |i| format!("{}: {}", i.code, i.message)));
    }
    Ok(out)
}

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let mut node = Multivoice { audio: Vec::new(), key: String::new(), sample_rate: init.sample_rate, source: String::new(), from: 0.0, parts: 0 };
    node.configure(init);
    Box::new(node)
}

struct Multivoice {
    /// The parts received so far at the patch rate.
    audio: Vec<f32>,
    /// What `audio` was made from (parts kept, source, rate), so a live swap of `gain` alone does nothing.
    key: String,
    sample_rate: f64,
    source: String,
    from: f64,
    parts: usize,
}

impl Node for Multivoice {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, _input: (f64, f64), params: &[f64]) -> NodeOut {
        let at = (ctx.seconds * self.sample_rate).floor();
        let v = if at >= 0.0 { self.audio.get(at as usize).copied().unwrap_or(0.0) as f64 * params[GAIN] } else { 0.0 };
        let v = if v.is_finite() { v } else { 0.0 };
        NodeOut { l: v, r: v, value: 0.0 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        let text = init.texts.get(TEXT).and_then(|t| t.as_deref()).unwrap_or("").to_string();
        let silent = text.trim().is_empty();
        let source = clip_key(&text, init.params, init.curve);
        if !silent {
            let w = Wanted { key: source.clone(), text, params: init.params.iter().take(GAIN).copied().collect(), overrides: overrides(init.params, init.curve) };
            wanted_list(|list| {
                if list.len() >= 256 {
                    list.clear(); // a player never takes them: keep the list bounded
                }
                list.push(w);
            });
        }
        let kept = clip(&source);
        let (parts, complete) = kept.as_ref().map_or((0, false), |c| (c.parts.len(), c.complete));
        let key = format!("{parts}\u{1}{complete}\u{1}{source}\u{1}{}", init.sample_rate);
        if key == self.key {
            return;
        }
        self.key = key;
        let Some(c) = kept.filter(|_| !silent) else {
            self.audio.clear();
            self.source.clear();
            self.parts = 0;
            return;
        };
        if self.source != source || self.sample_rate != init.sample_rate || self.from != c.rate || c.parts.len() < self.parts {
            self.audio.clear();
            self.parts = 0;
        }
        for part in &c.parts[self.parts..] {
            self.audio.extend(tinyvoca::resample(part, c.rate, init.sample_rate));
        }
        (self.source, self.sample_rate, self.from, self.parts) = (source, init.sample_rate, c.rate, c.parts.len());
    }

    fn reset(&mut self) {}
}

/// The model interface the host's graphs have: `io` 1 = emotion class ids + intensity and one decode noise_scale,
/// 2 = emotion mix vectors of width `emotion_dim` and a per-frame noise_scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interface {
    pub io: u32,
    pub emotion_dim: usize,
}

impl Default for Interface {
    fn default() -> Self {
        Interface { io: 1, emotion_dim: 16 }
    }
}

/// The encoder inputs of the part a host must render now (graph input names in the comments).
#[derive(Clone, Debug, PartialEq)]
pub struct EncodeInputs {
    /// voice.onnx `attrs` [1,3], `z` [1,8]; `quality` [1,3] for encode, prosody and decode.
    pub attrs: [f32; 3],
    pub z: [f32; 8],
    pub quality: [f32; 3],
    /// `input`, `stress` [1,t].
    pub tokens: Tokens,
    /// `emotion` (and io 1 `intensity`).
    pub emotion: EmotionInput,
    /// `scales` [2] = noise_w, length_scale (1 / speed).
    pub scales: [f32; 2],
    /// `dp_noise` [1,2,t].
    pub dp_noise: Vec<f32>,
}

/// One part handed to the host.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Already in the store: nothing to render.
    Kept { key: String, index: usize, last: bool, rate: f64, frames: usize },
    /// To render: run voice + encode on `inputs`, then [`MvJobs::expand`], prosody, [`MvJobs::curves`], decode with
    /// [`MvJobs::decode_noise`], then [`MvJobs::finish`].
    Render { key: String, index: usize, last: bool, inputs: Box<EncodeInputs> },
    /// The part cannot be rendered (its source is then complete and silent from here).
    Failed { key: String, index: usize, error: String },
}

struct Job {
    wanted: Wanted,
    rate: f64,
    utterances: Vec<Vec<Part>>,
    next: usize,
    frames: usize,
    /// Patch time where the next part starts (seconds), for the overrides.
    offset: f64,
}

/// The part being rendered between [`MvJobs::step`] and [`MvJobs::finish`].
struct Current {
    job: usize,
    tokens: Tokens,
    emotion: EmotionInput,
    expanded: Option<Expanded>,
}

/// The `multivoice` sources waiting to be rendered by a host with the model, one part at a time.
#[derive(Default)]
pub struct MvJobs {
    pub interface: Interface,
    jobs: Vec<Job>,
    current: Option<Current>,
}

fn param(w: &Wanted, i: usize) -> f64 {
    w.params.get(i).copied().unwrap_or(0.0)
}

impl MvJobs {
    /// Queues every source of `wanted` not queued yet, its parts kept at `rate` (the player's).
    pub fn add(&mut self, wanted: &[Wanted], rate: f64) {
        for w in wanted {
            if !self.jobs.iter().any(|j| j.wanted.key == w.key) {
                let utterances = mv::utterances(&mv::plan(&w.text));
                self.jobs.push(Job { wanted: w.clone(), rate, utterances, next: 0, frames: 0, offset: 0.0 });
            }
        }
    }

    /// Drops every queued source.
    pub fn clear(&mut self) {
        self.jobs.clear();
        self.current = None;
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// The next part of the source with the least audio so far.
    pub fn step(&mut self) -> Option<Step> {
        self.current = None;
        let i = (0..self.jobs.len()).min_by_key(|&i| self.jobs[i].frames)?;
        let job = &mut self.jobs[i];
        let key = job.wanted.key.clone();
        let index = job.next;
        let total = job.utterances.len();
        if total == 0 {
            put_part(&key, 0, true, job.rate, Vec::new());
            self.jobs.remove(i);
            return Some(Step::Kept { key, index: 0, last: true, rate: 0.0, frames: 0 });
        }
        let last = index + 1 >= total;
        if let Some(c) = clip(&key).filter(|c| index < c.parts.len() && c.rate == job.rate) {
            let frames = c.parts[index].len();
            let last = c.complete && index + 1 == c.parts.len();
            job.next += 1;
            job.frames += frames;
            job.offset += frames as f64 / job.rate;
            if last {
                self.jobs.remove(i);
            }
            return Some(Step::Kept { key, index, last, rate: c.rate, frames });
        }
        let w = job.wanted.clone();
        let utterance = job.utterances[index].clone();
        let w = &w;
        let intensity = param(w, INTENSITY) as f32;
        // the emotion comes from the text's tags only: text before the first tag is neutral
        let tokens = match mv::tokens(&utterance, Emotion::NEUTRAL, intensity) {
            Ok(t) if t.ids.len() > 2 => t,
            Ok(_) => return Some(self.fail(i, "nothing to say in this sentence".into())),
            Err(e) => return Some(self.fail(i, e)),
        };
        let seed = param(w, SEED).round().clamp(0.0, 99_999.0) as u32;
        let (attrs, quality, z) = mv::voice_inputs(param(w, AGE), param(w, SEX), param(w, WEIGHT), param(w, ROUGH), param(w, BREATH), param(w, TREMBLE), seed);
        let speed = param(w, SPEED).clamp(0.25, 4.0);
        let emotion = EmotionInput::new(self.interface.io, self.interface.emotion_dim, &tokens.emotions);
        let t = tokens.ids.len();
        let inputs = EncodeInputs {
            attrs,
            z,
            quality,
            tokens: tokens.clone(),
            emotion: emotion.clone(),
            scales: [0.0, (1.0 / speed) as f32],
            dp_noise: mv::Gauss::take(seed, mv::stream(index, 1), 2 * t),
        };
        self.current = Some(Current { job: i, tokens, emotion, expanded: None });
        Some(Step::Render { key, index, last, inputs: Box::new(inputs) })
    }

    /// A part that cannot be rendered: the source ends there, silent.
    fn fail(&mut self, i: usize, error: String) -> Step {
        let job = self.jobs.remove(i);
        put_part(&job.wanted.key, job.next, true, job.rate, Vec::new());
        Step::Failed { key: job.wanted.key, index: job.next, error }
    }

    fn current(&mut self) -> Result<(&mut Current, &Job), String> {
        let c = self.current.as_mut().ok_or("no part is being rendered (step first)")?;
        let job = self.jobs.get(c.job).ok_or("the part's source is gone")?;
        Ok((c, job))
    }

    /// encode's `durations`, `h`, `m_p`, `logs_p` -> the frame inputs of prosody and decode.
    pub fn expand(&mut self, durations: &[f32], h: &[f32], m: &[f32], logs: &[f32]) -> Result<&Expanded, String> {
        let (c, job) = self.current()?;
        let t = c.tokens.ids.len();
        if durations.len() != t || h.len() != mv::CHANNELS * t || m.len() != h.len() || logs.len() != h.len() {
            return Err(format!("expand: expected {t} durations and {}x{t} columns", mv::CHANNELS));
        }
        let w = &job.wanted;
        let noise = (param(w, NOISE) as f32, param(w, EVENT_NOISE) as f32);
        let x = mv::expand(durations, h, m, logs, &c.tokens.ids, &c.emotion, noise, &w.overrides, job.offset);
        c.expanded = Some(x);
        Ok(c.expanded.as_ref().expect("just set"))
    }

    /// prosody's `log_f0`, `voiced`, `energy`, replaced in place where a curve is wired; returns the names replaced.
    pub fn curves(&mut self, log_f0: &mut [f32], voiced: &mut [f32], energy: &mut [f32]) -> Result<Vec<&'static str>, String> {
        let (_, job) = self.current()?;
        Ok(job.wanted.overrides.apply_curves(log_f0, voiced, energy, job.offset))
    }

    /// decode's `noise_scale`, `z_noise`, `src_noise` for the expanded part.
    pub fn decode_noise(&mut self) -> Result<(Vec<f32>, Vec<f32>, Vec<f32>), String> {
        let io = self.interface.io;
        let (c, job) = self.current()?;
        let x = c.expanded.as_ref().ok_or("decode noise before expand")?;
        let seed = param(&job.wanted, SEED).round().clamp(0.0, 99_999.0) as u32;
        Ok(mv::decode_noise(io, seed, job.next, &x.noise))
    }

    /// decode's `audio` (22050 Hz) -> the part at the source's rate, kept in the store; returns it with the step.
    pub fn finish(&mut self, audio: &[f32]) -> Result<(Step, Arc<Vec<f32>>), String> {
        let c = self.current.take().ok_or("no part is being rendered (step first)")?;
        let job = self.jobs.get_mut(c.job).ok_or("the part's source is gone")?;
        let pcm = mv::finish(audio, job.rate);
        let index = job.next;
        let last = index + 1 >= job.utterances.len();
        let key = job.wanted.key.clone();
        put_part(&key, index, last, job.rate, pcm.clone());
        job.next += 1;
        job.frames += pcm.len();
        job.offset += audio.len() as f64 / mv::SAMPLE_RATE;
        let (rate, frames) = (job.rate, pcm.len());
        if last {
            self.jobs.remove(c.job);
        }
        Ok((Step::Kept { key, index, last, rate, frames }, Arc::new(pcm)))
    }
}
