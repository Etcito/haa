//! `speak` source node: tiny-voca English speech (crates/tinyvoca, a port of n4ze3m/tiny-voca, Apache-2.0).
//!
//! The model is the official GGUF the host loaded once with `haa_tinyvoca_load` (wasm) or `tinyvoca::install`
//! (native): nothing is bundled. Load it before the patch; without it the node is silent.
//! `text` is plain English (built with the `dizione` feature; the phonemizer comes from the same one-file GGUF or
//! the tiny-voca `phonemizer/` folder, see dizione::install): it is cut into utterances as tiny-voca's page cuts
//! it (dizione::utterances: sentences, then clauses, at most `max_tokens - 12` tokens each), each utterance is
//! synthesized with seed `seed + its index`, and the clip is limited to a 0.99 peak, as upstream `say.mjs` does.
//! Script markup in `text` (tags, emphasis, pauses, inline sounds: crate::voice_script) shapes the speech. When `text` is
//! empty, `input` is used: JSON, either a `perf.json` v1 performance (it has `durations`: its durations and curves are used) or
//! `{"tokens": [...], "stress": [...]}` (dizione ids: the model predicts the performance).
//! A voice is only these parameters (voice.tvv.json v3 is their import/export form): `m2` blends the GGUF's M2 and
//! F2 speakers (1 = M2, 0 = F2, a little outside extrapolates); `warp`, `tilt`, `envelope` (32 band offsets in dB,
//! 1/6 octave from 150 Hz, as text), `breath` and `phonation` edit the spectral envelope before the head; `shift`,
//! `range`, `rate` and `jitter` edit the curves (see tinyvoca::edit).
//! `configure` renders the whole utterance (24 kHz, resampled to the patch rate) and may allocate: changing a
//! voice parameter renders again; only `gain` follows modulation per sample. `process_sample` reads the audio at
//! the patch clock, so a loop or a restart replays it, and never allocates.
//!
//! Clips: speech is rendered utterance by utterance, and every utterance (a part) is kept in a small process-wide
//! store, keyed by what it was rendered from (text, input, envelope, voice params, seed: [`clip_key`]). A node
//! whose key is in the store plays the parts there, at the patch clock, and needs no model; parts that are not
//! there yet are silence. So a host with the model (a worker) plans a patch ([`plan_patch`], [`SpeechJobs`]) and
//! renders one part at a time ([`SpeechJobs::step`]) while a host without it (the AudioWorklet player,
//! `haa_speak_part_put`) takes each part as it comes and loads the patch again: playback can start with the first
//! utterance, and nothing heavy runs on the audio thread (a part is resampled to the patch rate when it arrives).
//! Each utterance is brought under a 0.99 peak on its own (say.mjs `limited` does it on the whole clip), so a
//! streamed clip and a whole one are the same samples.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

/// Rendered speech: the utterances (parts) said so far, mono at `rate`, in order; `complete` once the last is in.
#[derive(Clone, Debug)]
pub struct Clip {
    pub rate: f64,
    pub parts: Vec<Arc<Vec<f32>>>,
    pub complete: bool,
}

impl Clip {
    /// A complete clip of one part.
    pub fn whole(rate: f64, audio: Vec<f32>) -> Self {
        Clip { rate, parts: vec![Arc::new(audio)], complete: true }
    }

    /// Frames over all parts.
    pub fn frames(&self) -> usize {
        self.parts.iter().map(|p| p.len()).sum()
    }

    /// The parts one after the other.
    pub fn audio(&self) -> Vec<f32> {
        self.parts.iter().flat_map(|p| p.iter().copied()).collect()
    }
}

/// What a `speak` node asked for when it configured: enough to render its speech in another host.
#[derive(Clone, Debug, PartialEq)]
pub struct Wanted {
    pub key: String,
    pub text: String,
    pub input: String,
    pub envelope: String,
    pub params: Vec<f64>,
    pub seed: u32,
}

/// At most this many clips are kept; the store is emptied when a new one would pass it.
const MAX_CLIPS: usize = 32;

struct Clips {
    by_key: HashMap<String, Clip>,
    /// What the nodes configured since the last [`take_wanted`], in order (what a patch needs).
    wanted: Vec<Wanted>,
}

static CLIPS: Mutex<Option<Clips>> = Mutex::new(None);

/// While set, `speak` nodes only record what they want ([`plan_patch`]); they neither render nor resample.
static PLANNING: AtomicBool = AtomicBool::new(false);

fn with_clips<R>(f: impl FnOnce(&mut Clips) -> R) -> R {
    let mut guard = CLIPS.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(|| Clips { by_key: HashMap::new(), wanted: Vec::new() }))
}

/// What a `speak` node renders from: its text, input, envelope, voice params (`params[..GAIN]`) and seed.
pub fn clip_key(text: &str, input: &str, envelope: &str, params: &[f64], seed: u32) -> String {
    let voice: Vec<f64> = params.iter().take(GAIN).copied().collect();
    format!("{text}\u{1}{input}\u{1}{envelope}\u{1}{voice:?}\u{1}{seed}")
}

/// Keeps a clip under `key` (replacing one there).
pub fn put_clip(key: &str, clip: Clip) {
    with_clips(|c| {
        if c.by_key.len() >= MAX_CLIPS && !c.by_key.contains_key(key) {
            c.by_key.clear();
        }
        c.by_key.insert(key.to_string(), clip);
    });
}

/// Keeps part `index` of the clip under `key` (the parts before it must be there; `last` completes the clip).
/// Returns false for a part out of order.
pub fn put_part(key: &str, index: usize, last: bool, rate: f64, audio: Vec<f32>) -> bool {
    with_clips(|c| {
        if c.by_key.len() >= MAX_CLIPS && !c.by_key.contains_key(key) {
            c.by_key.clear();
        }
        let clip = c.by_key.entry(key.to_string()).or_insert_with(|| Clip { rate, parts: Vec::new(), complete: false });
        if clip.rate != rate {
            *clip = Clip { rate, parts: Vec::new(), complete: false };
        }
        if index > clip.parts.len() {
            return false;
        }
        if index == clip.parts.len() {
            clip.parts.push(Arc::new(audio));
        }
        if last {
            clip.parts.truncate(index + 1);
            clip.complete = true;
        }
        true
    })
}

/// The clip kept under `key`.
pub fn clip(key: &str) -> Option<Clip> {
    with_clips(|c| c.by_key.get(key).cloned())
}

/// What `speak` nodes asked for since the last call (and forgets it).
pub fn take_wanted() -> Vec<Wanted> {
    with_clips(|c| std::mem::take(&mut c.wanted))
}

/// The speech a patch needs with voice seed `seed`: one [`Wanted`] per distinct `speak` source, in node order.
/// Nothing is rendered.
pub fn plan_patch(patch: &str, seed: u32) -> Result<Vec<Wanted>, String> {
    take_wanted();
    PLANNING.store(true, Ordering::SeqCst);
    let mut voice = crate::Voice::new(48_000.0, seed);
    let outcome = voice.load(patch, false);
    PLANNING.store(false, Ordering::SeqCst);
    let mut out: Vec<Wanted> = Vec::new();
    for w in take_wanted() {
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

/// One part [`SpeechJobs::step`] made available in the store.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub key: String,
    pub index: usize,
    pub last: bool,
    pub rate: f64,
    pub frames: usize,
    /// Why the source could not be rendered (its clip is then complete and silent).
    pub error: Option<String>,
}

struct Job {
    wanted: Wanted,
    /// The rate the parts are kept at (the player's; 0 = the model's).
    rate: f64,
    utterances: Option<Vec<Utterance>>,
    next: usize,
    frames: usize,
}

/// The sources waiting to be rendered, one part at a time; the host must have the model. Parts already in the
/// store are handed out again without rendering, so a repeat costs nothing.
#[derive(Default)]
pub struct SpeechJobs {
    jobs: Vec<Job>,
}

impl SpeechJobs {
    /// Queues every source of `wanted` that is not queued yet, its parts to be kept at `rate` (the rate of the
    /// player that takes them, so it only copies them; 0 = the model's rate).
    pub fn add(&mut self, wanted: &[Wanted], rate: f64) {
        for w in wanted {
            if !self.jobs.iter().any(|j| j.wanted.key == w.key) {
                self.jobs.push(Job { wanted: w.clone(), rate, utterances: None, next: 0, frames: 0 });
            }
        }
    }

    /// Whether nothing is left to hand out.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Puts the next part of the source with the least audio so far in the store (rendering it unless it is
    /// there already) and says which part it is; `None` once every source is complete.
    pub fn step(&mut self) -> Option<Step> {
        let i = (0..self.jobs.len()).min_by_key(|&i| self.jobs[i].frames)?;
        let job = &mut self.jobs[i];
        let key = job.wanted.key.clone();
        let kept = clip(&key);
        let model_rate = tinyvoca::shared().map_or(24_000.0, |m| m.config.sample_rate as f64);
        let rate = if job.rate > 0.0 { job.rate } else { model_rate };
        let (index, last, rate, frames, error) = match kept.as_ref().filter(|c| job.next < c.parts.len() && c.rate == rate) {
            Some(c) => (job.next, c.complete && job.next + 1 == c.parts.len(), c.rate, c.parts[job.next].len(), None),
            None => {
                let (audio, last, error) = match render_part(job) {
                    Ok((audio, last)) => (tinyvoca::resample(&audio, model_rate, rate), last, None),
                    Err(e) => (Vec::new(), true, Some(e)),
                };
                let frames = audio.len();
                put_part(&key, job.next, last, rate, audio);
                (job.next, last, rate, frames, error)
            }
        };
        job.next += 1;
        job.frames += frames;
        if last {
            self.jobs.remove(i);
        }
        Some(Step { key, index, last, rate, frames, error })
    }
}

/// Renders part `job.next` of a job's source.
fn render_part(job: &mut Job) -> Result<(Vec<f32>, bool), String> {
    let w = &job.wanted;
    let (m2, post) = voice_params(&w.params, &w.envelope)?;
    if w.text.trim().is_empty() {
        return Ok((render_input(&w.input, m2, post, w.seed)?, true));
    }
    let model = tinyvoca::shared().ok_or("no tiny-voca model loaded (haa_tinyvoca_load)")?;
    if job.utterances.is_none() {
        job.utterances = Some(utterances_of(&w.text)?.0);
    }
    let utterances = job.utterances.as_deref().unwrap_or(&[]);
    let Some(u) = utterances.get(job.next) else { return Ok((Vec::new(), true)) };
    let voice = model.blend_voice(m2, post)?;
    let audio = say_utterance(&model, &voice, u, w.seed.wrapping_add(job.next as u32))?;
    Ok((audio, job.next + 1 >= utterances.len()))
}

/// One utterance of a text: its tokens and stresses, and the script's edits when its markup changes anything.
struct Utterance {
    ids: Vec<u32>,
    stress: Vec<u32>,
    plan: Option<crate::voice_script::UtterancePlan>,
}

/// One utterance in a voice, under a 0.99 peak (say.mjs `limited`, per utterance). An utterance with script edits
/// goes through crate::voice_script::synthesize_edited (the emotion edits between the model's stages).
fn say_utterance(model: &tinyvoca::Model, voice: &tinyvoca::Voice, u: &Utterance, seed: u32) -> Result<Vec<f32>, String> {
    let mut audio = match &u.plan {
        Some(plan) => crate::voice_script::with_emotions(|e| crate::voice_script::synthesize_edited(model, voice, plan, e, seed))?.audio,
        None => model.synthesize(&u.ids, &u.stress, voice, &tinyvoca::Options { seed, ..Default::default() }, None)?.audio,
    };
    let peak = audio.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    if peak > 0.99 {
        let k = 0.99 / peak;
        for v in &mut audio {
            *v *= k;
        }
    }
    Ok(audio)
}

/// How `text` is cut, phonemized and edited by its script markup (crate::voice_script): the utterances (none when
/// there is nothing to say) and the script's warnings.
#[cfg(feature = "dizione")]
fn utterances_of(text: &str) -> Result<(Vec<Utterance>, Vec<String>), String> {
    use crate::voice_script::{UtteranceWords, WordPos, plan_utterances};
    let model = tinyvoca::shared().ok_or("no tiny-voca model loaded (haa_tinyvoca_load)")?;
    let said = parse_script(text);
    let mut warnings = said.warnings.clone();
    let limit = model.config.max_tokens.saturating_sub(12).max(1);
    let utterances = dizione::with_shared(|p| p.utterances_g2p(&said.plain, limit))
        .ok_or("no phonemizer loaded (a one-file tiny-voca GGUF, or haa_dizione_*)")?
        .map_err(|e| e.0)?;
    let stresses: Vec<Vec<u32>> = utterances.iter().map(|(_, g)| g.tokens.stress.iter().map(|&s| s as u32).collect()).collect();
    let plans = if said.edits() {
        let words: Vec<UtteranceWords<'_>> = utterances
            .iter()
            .zip(&stresses)
            .map(|((text, g), stress)| UtteranceWords {
                text,
                tokens: &g.tokens.ids,
                stress,
                words: g.words.iter().map(|w| WordPos { tokens: w.tokens, span: w.span, inline: w.source == "inline" }).collect(),
            })
            .collect();
        plan_utterances(&said, &words, &mut warnings)
    } else {
        Vec::new()
    };
    let mut plans = plans.into_iter();
    let out = utterances
        .iter()
        .zip(stresses)
        .map(|((_, g), stress)| Utterance { ids: g.tokens.ids.clone(), stress, plan: plans.next().filter(|p| !p.is_empty()) })
        .collect();
    Ok((out, warnings))
}

/// Without the `dizione` feature there is no text front end.
#[cfg(not(feature = "dizione"))]
fn utterances_of(_text: &str) -> Result<(Vec<Utterance>, Vec<String>), String> {
    Err("this build has no text front end (haa-core feature dizione)".into())
}

pub const TEXT: usize = 0;
pub const INPUT: usize = 1;
pub const M2: usize = 2;
pub const WARP: usize = 3;
pub const TILT: usize = 4;
pub const ENVELOPE: usize = 5;
pub const BREATH: usize = 6;
pub const PHONATION: usize = 7;
pub const SHIFT: usize = 8;
pub const RANGE: usize = 9;
pub const RATE: usize = 10;
pub const JITTER: usize = 11;
pub const GAIN: usize = 12;

const fn number(key: &'static str, min: f64, max: f64, default: f64, step: f64, scale: Scale, unit: &'static str, en: &'static str, fr: &'static str) -> ParamDef {
    ParamDef { key, kind: ParamKind::Number { min, max, default, step, scale }, unit, label_en: en, label_fr: fr, modulatable: false, smooth: false }
}

/// The `speak` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "speak",
    category: Category::Source,
    label_en: "Speak (tiny-voca)",
    label_fr: "Parole (tiny-voca)",
    audio_in: false,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "text",
            kind: ParamKind::Text { default: "" },
            unit: "",
            label_en: "Text",
            label_fr: "Texte",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "input",
            kind: ParamKind::Text { default: "" },
            unit: "",
            label_en: "Tokens or performance (JSON)",
            label_fr: "Tokens ou performance (JSON)",
            modulatable: false,
            smooth: false,
        },
        number("m2", -0.5, 1.5, 1.0, 0.01, Scale::Lin, "", "Voice blend (F2 0 - M2 1)", "Mélange de voix (F2 0 - M2 1)"),
        number("warp", 0.7, 1.4, 1.0, 0.005, Scale::Log, "×", "Tract warp", "Échelle du conduit"),
        number("tilt", -12.0, 12.0, 0.0, 0.1, Scale::Lin, "dB/oct", "Spectral tilt", "Pente spectrale"),
        ParamDef {
            key: "envelope",
            kind: ParamKind::Text { default: "" },
            unit: "dB",
            label_en: "Envelope offsets (32 bands)",
            label_fr: "Décalages d'enveloppe (32 bandes)",
            modulatable: false,
            smooth: false,
        },
        number("breath", -24.0, 24.0, 0.0, 0.1, Scale::Lin, "dB", "Breath", "Souffle"),
        number("phonation", -12.0, 12.0, 0.0, 0.1, Scale::Lin, "dB", "Phonation (H1-H2)", "Phonation (H1-H2)"),
        number("shift", -12.0, 12.0, 0.0, 0.1, Scale::Lin, "st", "Pitch shift", "Transposition"),
        number("range", 0.0, 3.0, 1.0, 0.01, Scale::Lin, "×", "Pitch range", "Ambitus"),
        number("rate", 0.25, 4.0, 1.0, 0.01, Scale::Log, "×", "Rate", "Débit"),
        number("jitter", 1.0, 5.0, 1.0, 0.01, Scale::Lin, "×", "Jitter", "Instabilité"),
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

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let mut node = Speak { audio: Vec::new(), key: String::new(), sample_rate: init.sample_rate, source: String::new(), from: 0.0, parts: 0 };
    node.configure(init);
    Box::new(node)
}

struct Speak {
    /// The utterance at the patch rate.
    audio: Vec<f32>,
    /// What `audio` was made from (model, parts kept, source, rate), so a live swap of `gain` alone does nothing.
    key: String,
    sample_rate: f64,
    /// The source (`clip_key`) of `audio`, the rate of its clip and how many of its parts `audio` holds.
    source: String,
    from: f64,
    parts: usize,
}

/// The 32 band offsets of the `envelope` text (numbers separated by commas or spaces); empty = none.
pub fn envelope_bands(text: &str) -> Result<Vec<f32>, String> {
    let values: Result<Vec<f32>, _> = text.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(str::parse).collect();
    let values = values.map_err(|_| "envelope: expected numbers".to_string())?;
    if !values.is_empty() && values.len() != tinyvoca::voice::ENV_BANDS {
        return Err(format!("envelope: expected {} values", tinyvoca::voice::ENV_BANDS));
    }
    Ok(values)
}

/// The voice the node's parameters describe: (M2 weight, post edits).
pub fn voice_params(params: &[f64], envelope: &str) -> Result<(f32, tinyvoca::Post), String> {
    let p = |i: usize| params.get(i).copied().unwrap_or(0.0) as f32;
    Ok((
        p(M2),
        tinyvoca::Post {
            warp: p(WARP),
            tilt_db_oct: p(TILT),
            env_offset_db: envelope_bands(envelope)?,
            breath_db: p(BREATH),
            phonation: p(PHONATION),
            f0_shift_st: p(SHIFT),
            f0_range_x: p(RANGE),
            rate_x: p(RATE),
            jitter_x: p(JITTER),
        },
    ))
}

/// Script markup taken out of a `speak` text: what is said, and the tags met on the way.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    /// The text to say, markup removed (inline sounds stand as dizione inline words `{ə}`).
    pub plain: String,
    /// Each `[...]` tag: its byte offset in `plain` and what was inside the brackets.
    pub tags: Vec<(usize, String)>,
}

/// The whole script of a `speak` text (crate::voice_script, SCRIPT FORMAT v2: `[key:N, ...]` tags, `*emphasis*`,
/// `|` pauses, `{...}` inline sounds) with the presets and vocal events in use.
pub fn parse_script(text: &str) -> crate::voice_script::Script {
    crate::voice_script::with_emotions(|e| crate::voice_script::with_events(|ev| crate::voice_script::parse(text, e, ev)))
}

/// Reads the script markup of a `speak` text: the plain text and the tags (a space is left where markup separated
/// two words; an unclosed `[` stays as text). [`parse_script`] gives the spans, emphasis, pauses, sounds and
/// warnings.
pub fn script(text: &str) -> Script {
    let s = parse_script(text);
    Script { plain: s.plain, tags: s.tags }
}

/// What [`speak_script`] said: the audio (24 kHz) and the script's warnings (unknown keys, unclosed marks...).
#[derive(Clone, Debug, Default)]
pub struct Spoken {
    pub audio: Vec<f32>,
    pub warnings: Vec<String>,
}

/// Says `text` (plain English, script markup allowed) with the shared model and phonemizer for a voice
/// `(m2, post)`: 24 kHz audio, the utterances one after the other. The wasm `speak` op, the `haa speak` command
/// and the node all say the same samples.
pub fn speak_text(text: &str, m2: f32, post: tinyvoca::Post, seed: u32) -> Result<Vec<f32>, String> {
    Ok(speak_script(text, m2, post, seed)?.audio)
}

/// [`speak_text`] with the script's warnings (crate::voice_script: an utterance whose markup changes nothing is
/// synthesized as plain text, the others with the emotion edits between the model's stages).
pub fn speak_script(text: &str, m2: f32, post: tinyvoca::Post, seed: u32) -> Result<Spoken, String> {
    let (parts, warnings) = speak_parts_warned(text, m2, post, seed)?;
    Ok(Spoken { audio: parts.into_iter().flatten().collect(), warnings })
}

/// [`speak_text`] per utterance.
fn speak_parts(text: &str, m2: f32, post: tinyvoca::Post, seed: u32) -> Result<Vec<Vec<f32>>, String> {
    Ok(speak_parts_warned(text, m2, post, seed)?.0)
}

fn speak_parts_warned(text: &str, m2: f32, post: tinyvoca::Post, seed: u32) -> Result<(Vec<Vec<f32>>, Vec<String>), String> {
    let model = tinyvoca::shared().ok_or("no tiny-voca model loaded (haa_tinyvoca_load)")?;
    let voice = model.blend_voice(m2, post)?;
    let (utterances, warnings) = utterances_of(text)?;
    let parts = utterances
        .iter()
        .enumerate()
        .map(|(i, u)| say_utterance(&model, &voice, u, seed.wrapping_add(i as u32)))
        .collect::<Result<Vec<_>, String>>()?;
    Ok((parts, warnings))
}

/// Renders `text` (or, when it is empty, `input`) at `sample_rate`; see the module comment.
pub fn render_node(text: &str, input: &str, m2: f32, post: tinyvoca::Post, seed: u32, sample_rate: f64) -> Result<Vec<f32>, String> {
    let c = render_clip(text, input, m2, post, seed)?;
    Ok(c.parts.iter().flat_map(|p| tinyvoca::resample(p, c.rate, sample_rate)).collect())
}

/// Renders `text` (or, when it is empty, `input`) at the model's rate: a complete clip, one part per utterance.
pub fn render_clip(text: &str, input: &str, m2: f32, post: tinyvoca::Post, seed: u32) -> Result<Clip, String> {
    let rate = tinyvoca::shared().map_or(24_000.0, |m| m.config.sample_rate as f64);
    let parts = if text.trim().is_empty() { vec![render_input(input, m2, post, seed)?] } else { speak_parts(text, m2, post, seed)? };
    Ok(Clip { rate, parts: parts.into_iter().map(Arc::new).collect(), complete: true })
}

/// Renders `input` with the shared model for a voice `(m2, post)`, at `sample_rate`.
pub fn render(input: &str, m2: f32, post: tinyvoca::Post, seed: u32, sample_rate: f64) -> Result<Vec<f32>, String> {
    let rate = tinyvoca::shared().map_or(24_000.0, |m| m.config.sample_rate as f64);
    Ok(tinyvoca::resample(&render_input(input, m2, post, seed)?, rate, sample_rate))
}

fn render_input(input: &str, m2: f32, post: tinyvoca::Post, seed: u32) -> Result<Vec<f32>, String> {
    let model = tinyvoca::shared().ok_or("no tiny-voca model loaded (haa_tinyvoca_load)")?;
    let voice = model.blend_voice(m2, post)?;
    let value: serde_json::Value = serde_json::from_str(input).map_err(|e| format!("speak input: {e}"))?;
    let audio = if value.get("durations").is_some() {
        let perf: tinyvoca::Perf = serde_json::from_value(value).map_err(|e| format!("perf.json: {e}"))?;
        model.perform(&perf, &voice, None, seed)?
    } else {
        let list = |k: &str| -> Result<Vec<u32>, String> {
            value.get(k).and_then(|v| v.as_array()).ok_or(format!("speak input: no {k}"))?.iter()
                .map(|v| v.as_u64().map(|n| n as u32).ok_or(format!("speak input: {k} must be integers")))
                .collect()
        };
        let options = tinyvoca::Options { seed, ..Default::default() };
        model.synthesize(&list("tokens")?, &list("stress")?, &voice, &options, None)?.audio
    };
    Ok(audio)
}

impl Node for Speak {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, _input: (f64, f64), params: &[f64]) -> NodeOut {
        let at = (ctx.seconds * self.sample_rate).floor();
        let v = if at >= 0.0 { self.audio.get(at as usize).copied().unwrap_or(0.0) as f64 * params[GAIN] } else { 0.0 };
        let v = if v.is_finite() { v } else { 0.0 };
        NodeOut { l: v, r: v, value: 0.0 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        let text = |i: usize| init.texts.get(i).and_then(|t| t.as_deref()).unwrap_or("").to_string();
        let (said, input, envelope) = (text(TEXT), text(INPUT), text(ENVELOPE));
        let silent = said.trim().is_empty() && input.trim().is_empty();
        let source = clip_key(&said, &input, &envelope, init.params, init.seed);
        if !silent {
            let wanted = Wanted { key: source.clone(), text: said.clone(), input: input.clone(), envelope: envelope.clone(), params: init.params.iter().take(GAIN).copied().collect(), seed: init.seed };
            with_clips(|c| {
                if c.wanted.len() >= 256 {
                    c.wanted.clear(); // a player never takes them: keep the list bounded
                }
                c.wanted.push(wanted);
            });
        }
        if PLANNING.load(Ordering::SeqCst) {
            return;
        }
        let kept = clip(&source);
        let loaded = tinyvoca::shared().is_some();
        let (parts, complete) = kept.as_ref().map_or((0, false), |c| (c.parts.len(), c.complete));
        let key = format!("{loaded}\u{1}{parts}\u{1}{complete}\u{1}{source}\u{1}{}", init.sample_rate);
        if key == self.key {
            return;
        }
        self.key = key;
        if silent {
            self.audio.clear();
            self.source.clear();
            self.parts = 0;
            return;
        }
        if let Some(c) = kept {
            // a part arrived: resample only the new ones
            if self.source != source || self.sample_rate != init.sample_rate || self.from != c.rate || c.parts.len() < self.parts {
                self.audio.clear();
                self.parts = 0;
            }
            for part in &c.parts[self.parts..] {
                self.audio.extend(tinyvoca::resample(part, c.rate, init.sample_rate));
            }
            (self.source, self.sample_rate, self.from, self.parts) = (source, init.sample_rate, c.rate, c.parts.len());
            return;
        }
        self.audio.clear();
        self.parts = 0;
        self.source.clear();
        self.sample_rate = init.sample_rate;
        // an input or voice the model refuses leaves the node silent; the wasm `synthesize`/`perform` ops report
        // the same error as text
        if let Ok((m2, post)) = voice_params(init.params, &envelope)
            && let Ok(c) = render_clip(&said, &input, m2, post, init.seed)
        {
            self.audio = c.parts.iter().flat_map(|p| tinyvoca::resample(p, c.rate, init.sample_rate)).collect();
            (self.source, self.from, self.parts) = (source.clone(), c.rate, c.parts.len());
            put_clip(&source, c);
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_without_input_and_params_map_to_a_voice() {
        let params = [0.0, 0.0, 0.25, 1.1, -2.0, 0.0, 3.0, 1.0, 2.0, 1.2, 0.9, 1.5, 1.0];
        let texts = [Some(String::new()), Some(String::new()), None, None, None, Some("1 ".repeat(32)), None, None, None, None, None, None, None];
        let (m2, post) = voice_params(&params, texts[ENVELOPE].as_deref().unwrap()).unwrap();
        assert_eq!((m2, post.warp, post.rate_x, post.env_offset_db.len()), (0.25, 1.1, 0.9, 32));
        let mut node = (DEF.create)(&NodeInit {
            id: "speak1",
            seed: 0,
            sample_rate: 48_000.0,
            params: &params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        });
        let ctx = SampleCtx {
            sample_rate: 48_000.0,
            dt: 1.0 / 48_000.0,
            t: 0.0,
            seconds: 0.01,
            duration: 1.0,
            mode: crate::patch::GraphMode::Timeline,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        };
        let out = node.process_sample(&ctx, (0.0, 0.0), &params);
        assert_eq!((out.l, out.r), (0.0, 0.0));

        // a kept clip plays without a model once the node configures again (a player's reload)
        let said_texts = [Some("Hi.".to_string()), Some(String::new()), None, None, None, Some("1 ".repeat(32)), None, None, None, None, None, None, None];
        let init = NodeInit { id: "speak1", seed: 7, sample_rate: 48_000.0, params: &params, texts: &said_texts, curve: None, inputs: &[], duration: 1.0 };
        node.configure(&init);
        assert_eq!(node.process_sample(&ctx, (0.0, 0.0), &params).l, 0.0);
        let key = clip_key("Hi.", "", &"1 ".repeat(32), &params, 7);
        assert!(take_wanted().iter().any(|w| w.key == key && w.text == "Hi." && w.seed == 7));
        // parts arrive one by one: the first plays at once, the second lands after it, at its own time
        assert!(put_part(&key, 0, false, 24_000.0, vec![0.5; 2_400]));
        assert!(!put_part(&key, 2, true, 24_000.0, vec![0.0; 10]));
        node.configure(&init);
        assert!((node.process_sample(&ctx, (0.0, 0.0), &params).l - 0.5).abs() < 1e-3);
        let later = SampleCtx { seconds: 0.15, ..ctx };
        assert_eq!(node.process_sample(&later, (0.0, 0.0), &params).l, 0.0);
        assert!(put_part(&key, 1, true, 24_000.0, vec![-0.25; 2_400]));
        node.configure(&init);
        assert!((node.process_sample(&later, (0.0, 0.0), &params).l + 0.25).abs() < 1e-3);
        assert!(clip(&key).is_some_and(|c| c.complete && c.frames() == 4_800));
        let said = script("[calm] Hello[laughs]there, [scared:4,question:10]you. [open");
        assert_eq!(said.plain, " Hello there, you. [open");
        assert_eq!(said.tags, [(0, "calm".to_string()), (6, "laughs".to_string()), (14, "scared:4,question:10".to_string())]);
    }
}
