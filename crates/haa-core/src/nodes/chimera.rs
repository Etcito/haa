//! `chimera` source node: CHIMERA's speech and beatbox engines as one HAA node.
//!
//! This is the real thing, not an approximation: the node owns a `chimera::Engine` and renders
//! it into HAA's graph. `text` is IPA for speech (`sa va`, `bɔ̃ʒuʁ`) or a beatbox pattern
//! (`B t tss B`) depending on `mode`.
//!
//! CHIMERA is MIT-licensed; its crate is a sibling checkout (see `Cargo.toml`).
//! Blocks: the engine renders 128 frames at a time into an internal buffer; `process_sample`
//! serves them one by one, so the node keeps HAA's per-sample contract.
//! `configure` (live swap, load time) is where text is planned and the body rebuilt; it may
//! allocate. `process_sample` and `reset` never allocate.

use ::chimera::anatomy::human::BodyMix;
use ::chimera::engine::Engine;
use ::chimera::control::score::Gesture;
use ::chimera::phonetics::beatbox::parse_beat;
use ::chimera::phonetics::english::EnglishG2P;
use ::chimera::phonetics::ipa::tokenize_ipa;
use ::chimera::anatomy::human::ArticPatch;
use ::chimera::phonetics::planner::{plan_phrases, plan_utterance, vowel_artic, vowel_table, PhraseInput, PlanOptions};
use ::chimera::phonetics::Intonation;
use ::chimera::phonetics::sing::{plan_song, SongOptions, SungNote};
use ::chimera::phonetics::syllables::lyrics_to_syllables;
use ::chimera::sources::valves::Whistle;
use ::chimera::voice::human_voice::SourcePatch;

use std::io::Read;
use std::sync::OnceLock;

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;

const MODE: usize = 0;
const TEXT: usize = 1;
const PITCH: usize = 2;
const PRESSURE: usize = 3;
const RATE: usize = 4;
const FEMALE: usize = 5;
const CHILD: usize = 6;
const BPM: usize = 7;
const GAIN: usize = 8;
const BREATH: usize = 9;
const TENSION: usize = 10;
const LANGUAGE: usize = 11;
const MELODY: usize = 12;
const EXPRESSION: usize = 13;
const PERFORMANCE: usize = 14;
const WHISTLE_BODY: usize = 15;
const WHISTLE_EDGE: usize = 16;
const VOWEL_F1: usize = 17;
const VOWEL_F2: usize = 18;

const SPEECH: usize = 0;
const VOICE: usize = 2;
const WHISTLE: usize = 3;
const SING: usize = 4;
const ENGLISH: usize = 0;
const IPA: usize = 1;
/// Frames rendered per engine call; matches the graph block.
const BLOCK: usize = 128;

/// The `chimera` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "chimera",
    category: Category::Source,
    label_en: "Chimera voice",
    label_fr: "Voix Chimera",
    audio_in: false,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "mode",
            kind: ParamKind::Enum {
                values: &["speech", "beatbox", "voice", "whistle", "sing"],
                default: SPEECH,
            },
            unit: "",
            label_en: "Mode",
            label_fr: "Mode",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "text",
            kind: ParamKind::Text { default: "Hello." },
            unit: "",
            label_en: "Text or beat",
            label_fr: "Texte ou beat",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "pitch",
            kind: ParamKind::Number {
                min: 60.0,
                max: 3_000.0,
                default: 120.0,
                step: 1.0,
                scale: Scale::Log,
            },
            unit: "Hz",
            label_en: "Pitch",
            label_fr: "Hauteur",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "pressure",
            kind: ParamKind::Number {
                min: 2.0,
                max: 22.0,
                default: 8.0,
                step: 0.1,
                scale: Scale::Lin,
            },
            unit: "cmH2O",
            label_en: "Pressure",
            label_fr: "Pression",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "rate",
            kind: ParamKind::Number {
                min: 0.5,
                max: 2.0,
                default: 1.0,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "×",
            label_en: "Rate",
            label_fr: "Débit",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "female",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Female body",
            label_fr: "Corps féminin",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "child",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Child body",
            label_fr: "Corps enfant",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "bpm",
            kind: ParamKind::Number {
                min: 40.0,
                max: 200.0,
                default: 100.0,
                step: 1.0,
                scale: Scale::Lin,
            },
            unit: "BPM",
            label_en: "Beat tempo",
            label_fr: "Tempo du beat",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "gain",
            kind: ParamKind::Number {
                min: 0.0,
                max: 2.0,
                default: 0.8,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "×",
            label_en: "Level",
            label_fr: "Niveau",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "breath",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.1,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Breathiness",
            label_fr: "Souffle",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "tension",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Tension",
            label_fr: "Tension",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "language",
            kind: ParamKind::Enum {
                values: &["english", "ipa"],
                default: ENGLISH,
            },
            unit: "",
            label_en: "Text language",
            label_fr: "Langue du texte",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "melody",
            kind: ParamKind::Text { default: "" },
            unit: "",
            label_en: "Notes (MIDI:beats per syllable)",
            label_fr: "Notes (MIDI:temps par syllabe)",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "expression",
            kind: ParamKind::Number { min: 0.0, max: 0.5, default: 0.14, step: 0.01, scale: Scale::Lin },
            unit: "",
            label_en: "Speech intonation",
            label_fr: "Intonation parlée",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "performance",
            kind: ParamKind::Text { default: "" },
            unit: "",
            label_en: "Measured delivery (JSON)",
            label_fr: "Phrasé mesuré (JSON)",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "whistleBody",
            kind: ParamKind::Number { min: 0.0, max: 4.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Whistle body", label_fr: "Corps du sifflet",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "whistleEdge",
            kind: ParamKind::Number { min: 0.1, max: 2.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Whistle edge", label_fr: "Mordant du sifflet",
            modulatable: true, smooth: true,
        },
        // `voice` mode: the vowel as two formant targets. 0 keeps the text vowel; driven by curves, the tract glides
        // between the calibrated vowels nearest to (F1, F2), so a measured vowel trajectory plays on the real tract.
        ParamDef {
            key: "vowelF1",
            kind: ParamKind::Number { min: 0.0, max: 1_200.0, default: 0.0, step: 1.0, scale: Scale::Lin },
            unit: "Hz", label_en: "Vowel F1 (open)", label_fr: "Voyelle F1 (ouverture)",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "vowelF2",
            kind: ParamKind::Number { min: 0.0, max: 3_200.0, default: 0.0, step: 1.0, scale: Scale::Lin },
            unit: "Hz", label_en: "Vowel F2 (front)", label_fr: "Voyelle F2 (avant)",
            modulatable: true, smooth: true,
        },
    ],
    create,
};

/// CMUdict loaded once per process; `to_phrases` is read-only after this.
static ENGLISH_G2P: OnceLock<EnglishG2P> = OnceLock::new();

const CMUDICT_GZ: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/cmudict.txt.gz"));

fn english_g2p() -> &'static EnglishG2P {
    ENGLISH_G2P.get_or_init(|| {
        let mut text = String::new();
        flate2::read::GzDecoder::new(CMUDICT_GZ)
            .read_to_string(&mut text)
            .expect("cmudict gzip");
        let mut g2p = EnglishG2P::new();
        g2p.load_dict(&text);
        g2p
    })
}

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    ::chimera::init();
    let mut engine = Engine::new(init.sample_rate);
    engine.set_gate(1.0);
    let mut node = ChimeraNode {
        engine,
        sample_rate: init.sample_rate,
        buffer: [0.0; BLOCK],
        pos: BLOCK,
        text: String::new(),
        melody: String::new(),
        performance: String::new(),
        planned_lead: 0.05,
        planned_pitch: f64::NAN,
        expression: f64::NAN,
        mode: usize::MAX,
        rate: f64::NAN,
        female: f64::NAN,
        child: f64::NAN,
        bpm: f64::NAN,
        language: usize::MAX,
        body_set: false,
        whistle: Whistle::new(init.sample_rate),
        whistle_noise: 0.0,
        whistle_air: [0.0; 2],
        whistle_jitter: 0.0,
        whistle_flow: [[0.0; 2]; 2],
        rng: Rng::new(init.seed ^ 0x9e37_79b9),
        planned_gestures: Vec::new(),
        planned_end: 0.0,
        speech_end: None,
        beat_end: None,
        vowels: Vec::new(),
        vowel_at: (0.0, 0.0),
    };
    node.plan(init);
    Box::new(node)
}

struct ChimeraNode {
    engine: Engine,
    sample_rate: f64,
    buffer: [f32; BLOCK],
    pos: usize,
    text: String,
    melody: String,
    performance: String,
    planned_lead: f64,
    planned_pitch: f64,
    expression: f64,
    mode: usize,
    rate: f64,
    female: f64,
    child: f64,
    bpm: f64,
    language: usize,
    body_set: bool,
    /// CHIMERA's van der Pol aerodynamic whistle, used by `mode = whistle`.
    whistle: Whistle,
    whistle_noise: f64,
    whistle_air: [f64; 2],
    whistle_jitter: f64,
    whistle_flow: [[f64; 2]; 2],
    rng: Rng,
    /// Voice-clock time at which the spoken plan ends; the gate closes after it.
    planned_gestures: Vec<Gesture>,
    planned_end: f64,
    speech_end: Option<f64>,
    /// Voice-clock time at which one beat pass ends.
    beat_end: Option<f64>,
    /// `voice` mode: the calibrated vowels of the current body, (ln F1, ln F2, articulation).
    vowels: Vec<(f64, f64, ArticPatch)>,
    /// (F1, F2) the tract was last steered to.
    vowel_at: (f64, f64),
}

/// The calibrated vowels of a body mix: formants blended by the body weights, articulation from `vowel_artic`.
fn body_vowels(body: BodyMix) -> Vec<(f64, f64, ArticPatch)> {
    let table = vowel_table();
    let Some(male) = table.get("male") else { return Vec::new() };
    let mut out = Vec::new();
    for ipa in male.keys() {
        let (mut f1, mut f2, mut wsum) = (0.0, 0.0, 0.0);
        for (b, w) in body.weights() {
            if w <= 0.0 {
                continue;
            }
            let Some(v) = table.get(b.as_str()).and_then(|t| t.get(ipa)).or_else(|| male.get(ipa)) else { continue };
            if v.formants.len() < 2 {
                continue;
            }
            f1 += w * v.formants[0];
            f2 += w * v.formants[1];
            wsum += w;
        }
        if let (true, Some(artic)) = (wsum > 0.0, vowel_artic(ipa, body)) {
            out.push(((f1 / wsum).ln(), (f2 / wsum).ln(), artic));
        }
    }
    out
}

impl ChimeraNode {
    /// Steer the tract toward (F1, F2): inverse-distance blend of the three nearest calibrated vowels (ln formants,
    /// F2 counted at 0.6 of F1's weight: its spread is wider). Never allocates.
    fn steer_vowel(&mut self, f1: f64, f2: f64) {
        if self.vowels.is_empty() || f1 <= 0.0 || f2 <= 0.0 {
            return;
        }
        let (a, b) = self.vowel_at;
        if (f1 / a.max(1.0)).ln().abs() < 0.01 && (f2 / b.max(1.0)).ln().abs() < 0.01 {
            return;
        }
        let (l1, l2) = (f1.ln(), f2.ln());
        let mut near = [(f64::MAX, 0usize); 3];
        for (i, (v1, v2, _)) in self.vowels.iter().enumerate() {
            let d = ((l1 - v1).powi(2) + (0.6 * (l2 - v2)).powi(2)).sqrt();
            if d < near[2].0 {
                near[2] = (d, i);
                near.sort_by(|x, y| x.0.total_cmp(&y.0));
            }
        }
        let mut parts = [(0.0, ArticPatch::default()); 3];
        for (k, (d, i)) in near.iter().enumerate() {
            if *d < f64::MAX {
                parts[k] = (1.0 / (d + 0.02).powi(2), self.vowels[*i].2);
            }
        }
        self.engine.set_artic(&ArticPatch::blend(&parts), None);
        self.vowel_at = (f1, f2);
    }
}

impl ChimeraNode {
    fn plan(&mut self, init: &NodeInit<'_>) {
        let mode = init.params.get(MODE).copied().unwrap_or(0.0).round() as usize;
        let text = init
            .texts
            .get(TEXT)
            .and_then(|value| value.as_deref())
            .unwrap_or("sa va");
        let rate = init.params.get(RATE).copied().unwrap_or(1.0).clamp(0.5, 2.0);
        let female = init.params.get(FEMALE).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let child = init.params.get(CHILD).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let bpm = init.params.get(BPM).copied().unwrap_or(100.0).clamp(40.0, 200.0);
        let language = init.params.get(LANGUAGE).copied().unwrap_or(0.0).round() as usize;
        let melody = init.texts.get(MELODY).and_then(|v| v.as_deref()).unwrap_or("");
        let performance = init.texts.get(PERFORMANCE).and_then(|v| v.as_deref()).unwrap_or("");
        let pitch = init.params.get(PITCH).copied().unwrap_or(120.0);
        let expression = init.params.get(EXPRESSION).copied().unwrap_or(0.14).clamp(0.0, 0.5);
        let body_changed = !self.body_set || female != self.female || child != self.child;
        let changed = text != self.text
            || performance != self.performance
            || expression != self.expression
            || melody != self.melody
            || ((mode == SPEECH || mode == SING) && pitch != self.planned_pitch)
            || mode != self.mode
            || rate != self.rate
            || bpm != self.bpm
            || language != self.language
            || body_changed;

        if body_changed {
            self.engine.set_body(BodyMix::new(female, child));
            self.engine.set_gate(1.0);
            self.body_set = true;
        }

        if changed {
            if mode == SPEECH || mode == SING {
                self.engine.set_beat_playing(false);
                let options = PlanOptions {
                    body: BodyMix::new(female, child),
                    t0: 0.0,
                    rate: Some(rate),
                    f0: init.params.get(PITCH).copied(),
                    intonation: Some(Intonation::Statement),
                    accent: Some(expression),
                    open_end: None,
                    sung: None,
                };
                // English text is decomposed by CMUdict + letter-to-sound; punctuation carries the
                // intonation (`.` statement, `?` question, `!` exclamation, `,` `;` `:` pauses).
                // IPA remains available for exact control.
                let plan = if mode == SPEECH && !performance.is_empty() {
                    // Invalid explicit delivery fails silent rather than playing a different plan.
                    crate::voice_performance::plan_score(performance, english_g2p(), BodyMix::new(female, child)).unwrap_or_default()
                } else if mode == SING {
                    let source = |word: &str, reduce: bool| {
                        if language == IPA { tokenize_ipa(word) }
                        else { english_g2p().word(word, reduce) }
                    };
                    let syllables = lyrics_to_syllables(text, &source);
                    let score: Vec<Option<(f64, f64)>> = melody.split_whitespace().map(|token| {
                        let (note, beats) = token.split_once(':').unwrap_or((token, "1"));
                        let midi = note.parse::<f64>().ok()?;
                        let beats = beats.parse::<f64>().ok()?;
                        (midi.is_finite() && (24.0..=96.0).contains(&midi)
                            && beats.is_finite() && (0.125..=16.0).contains(&beats))
                            .then_some((midi, beats))
                    }).collect();
                    // Missing/invalid notes preserve every syllable at the base pitch, one beat.
                    let base = 69.0 + 12.0 * (pitch.clamp(60.0, 1100.0) / 440.0).log2();
                    let notes: Vec<SungNote> = syllables.into_iter().enumerate().map(|(i, syl)| {
                        let (midi, beats) = score.get(i).copied().flatten().unwrap_or((base, 1.0));
                        SungNote { midi, beats, syl }
                    }).collect();
                    plan_song(&notes, &SongOptions {
                        body: BodyMix::new(female, child), bpm, t0: 0.4, vibrato: 12.0,
                    })
                } else if language == IPA {
                    let segments = tokenize_ipa(text);
                    plan_utterance(&segments, &options)
                } else {
                    let phrases: Vec<PhraseInput> = english_g2p()
                        .to_phrases(text)
                        .into_iter()
                        .map(|phrase| PhraseInput {
                            segments: phrase.segments,
                            intonation: Some(phrase.intonation),
                            pause: Some(phrase.pause),
                        })
                        .collect();
                    plan_phrases(&phrases, &options)
                };
                self.planned_end = plan.end;
                self.planned_gestures = plan.gestures;
                self.planned_lead = if mode == SPEECH && !performance.is_empty() { 0.0 } else { 0.05 };
                let offset = self.engine.schedule(&self.planned_gestures, self.planned_lead, true);
                self.engine.set_gate(0.0); // The score owns phrase breath, including its lead-in and release.
                self.speech_end = Some(offset + self.planned_end + 0.08);
                self.beat_end = None;
            } else if mode == WHISTLE {
                // A standalone CHIMERA van der Pol whistle: aeroacoustic source, no vocal folds.
                self.engine.stop_speech();
                self.engine.set_beat_playing(false);
                self.speech_end = None;
                self.beat_end = None;
            } else if mode == VOICE {
                // A sustained Chimera vowel, no score: the pitch/gain curves play it like an
                // instrument, but the sound is the engine's voice (tract + folds), not a flute.
                let vowel = if text.is_empty() { "a" } else { text };
                if let Some(artic) = vowel_artic(vowel, BodyMix::new(female, child)) {
                    self.engine.set_artic(&artic, None);
                }
                self.vowels = body_vowels(BodyMix::new(female, child));
                self.vowel_at = (0.0, 0.0);
                self.engine.stop_speech();
                self.engine.set_gate(1.0);
                self.speech_end = None;
                self.beat_end = None;
            } else {
                let steps = parse_beat(text);
                let beats = steps.len() as f64 / 4.0;
                self.engine.set_beat_steps(&steps);
                self.engine.set_beat_tempo(Some(bpm), Some(0.0));
                self.engine.set_beat_playing(true);
                self.engine.set_gate(1.0);
                self.beat_end = Some(self.engine.clock() + beats * 60.0 / bpm + 0.15);
                self.speech_end = None;
            }
            self.performance.clear();
            self.performance.push_str(performance);
            self.melody.clear();
            self.melody.push_str(melody);
            self.planned_pitch = pitch;
            self.expression = expression;
            self.text.clear();
            self.text.push_str(text);
            self.mode = mode;
            self.rate = rate;
            self.female = female;
            self.child = child;
            self.bpm = bpm;
            self.language = language;
            // Drop the pre-schedule buffer so the new plan starts at its first sample.
            self.pos = BLOCK;
        }
    }
}

impl Node for ChimeraNode {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        if self.mode == WHISTLE {
            let pitch = params[PITCH].clamp(200.0, 3_000.0);
            let drive = (params[PRESSURE] / 22.0).clamp(0.0, 1.0);
            let breath = params[BREATH].clamp(0.0, 1.0);
            let gain = params[GAIN].clamp(0.0, 2.0);
            let white = self.rng.next_f64() * 2.0 - 1.0;
            let dt = 1.0 / self.sample_rate.max(1.0);
            let noise_alpha = 1.0 - (-core::f64::consts::TAU * 2000.0 * dt).exp();
            self.whistle_noise += noise_alpha * (white - self.whistle_noise);
            let jitter_alpha = 1.0 - (-core::f64::consts::TAU * 20.0 * dt).exp();
            self.whistle_jitter += jitter_alpha * ((self.rng.next_f64()*2.0-1.0) - self.whistle_jitter);
            let jitter_norm = (3.0*(2.0-jitter_alpha)/jitter_alpha).sqrt();
            let cents = 2.0 * breath * self.whistle_jitter * jitter_norm;
            self.whistle.f = pitch * (cents / 1200.0).exp2();
            let edge = params.get(WHISTLE_EDGE).copied().unwrap_or(1.0).clamp(0.1, 2.0);
            let body = params.get(WHISTLE_BODY).copied().unwrap_or(1.0).clamp(0.0, 4.0);
            let tone = self.whistle.next_shaped(drive, self.whistle_noise * 500.0 * breath, 1.0, edge);
            let phase = self.whistle.acoustic_phase();
            // Weak even harmonic follows the ACTUAL oscillator, never a second clock.
            // Scale with jet amplitude: no persistent partial at zero pressure.
            let h2_ratio = body * 0.004 * (2000.0 / pitch).clamp(0.65, 2.0);
            let second = (2.0*phase).cos() * drive * 2.0 * h2_ratio;
            // Signed turbulent air: rectification would introduce an artificial DC term.
            // Finite air bandwidth: filter only the turbulent component, leaving
            // the oscillator and its phase-locked harmonics untouched.
            let air_alpha = 1.0 - (-core::f64::consts::TAU * 5000.0 * dt).exp();
            self.whistle_air[0] += air_alpha * ((white-self.whistle_noise) - self.whistle_air[0]);
            self.whistle_air[1] += air_alpha * (self.whistle_air[0] - self.whistle_air[1]);
            let air = self.whistle_air[1] * breath * drive * 0.06;
            // A stochastic flow envelope produces sidebands around the whistle,
            // rather than adding unrelated broadband hiss. This is a phenomenological
            // turbulence control, not a solved mouth/jet geometry. Time constants and
            // stationary variance remain comparable at different sample rates.
            let mut flow = 0.0;
            for (i, (cutoff, amount)) in [(40.0, 0.6), (450.0, 0.35)].iter().enumerate() {
                let alpha = 1.0 - (-core::f64::consts::TAU * cutoff * dt).exp();
                let random = self.rng.next_f64() * 2.0 - 1.0;
                self.whistle_flow[i][0] += alpha * (random - self.whistle_flow[i][0]);
                self.whistle_flow[i][1] += alpha * (self.whistle_flow[i][0] - self.whistle_flow[i][1]);
                // Two poles keep envelope texture near the tone instead of leaking
                // modulation into remote harmonics. Exact stationary variance for
                // cascaded identical one-poles fed by uniform [-1,1] noise.
                let q = 1.0 - alpha;
                let norm = (3.0 * (2.0-alpha).powi(3) / (alpha * (1.0 + q*q))).sqrt();
                flow += amount * self.whistle_flow[i][1] * norm;
            }
            let flow_gain = 1.0 + 0.65 * (breath * flow / 0.65).tanh();
            let sample = ((tone * 0.9 + second) * flow_gain + air) * gain;
            let sample = if sample.is_finite() {
                sample.clamp(-1.5, 1.5)
            } else {
                0.0
            };
            return NodeOut {
                l: sample,
                r: sample,
                value: 0.0,
            };
        }

        if self.pos >= BLOCK {
            // Gate the engine open only while a plan or a beat pass is running; otherwise a
            // finished phrase keeps phonating a schwa for the rest of the timeline.
            let clock = self.engine.clock();
            if self.speech_end.is_some_and(|end| clock > end) {
                self.engine.set_gate(0.0);
                self.speech_end = None;
            }
            if self.beat_end.is_some_and(|end| clock > end) {
                self.engine.set_beat_playing(false);
                self.engine.set_gate(0.0);
                self.beat_end = None;
            }
            let patch = SourcePatch {
                f0: Some(params[PITCH]),
                pressure: Some(params[PRESSURE]),
                breathiness: Some(params[BREATH]),
                tension: Some(params[TENSION]),
                ..SourcePatch::default()
            };
            self.engine.set_source(&patch);
            if self.mode == VOICE {
                let f1 = params.get(VOWEL_F1).copied().unwrap_or(0.0);
                let f2 = params.get(VOWEL_F2).copied().unwrap_or(0.0);
                self.steer_vowel(f1, f2);
            }
            self.engine.render(&mut self.buffer);
            self.pos = 0;
        }
        let gain = params[GAIN].clamp(0.0, 2.0);
        let sample = self.buffer[self.pos] as f64 * gain;
        self.pos += 1;
        let sample = if sample.is_finite() {
            sample.clamp(-1.5, 1.5)
        } else {
            0.0
        };
        NodeOut {
            l: sample,
            r: sample,
            value: 0.0,
        }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        if (init.sample_rate - self.sample_rate).abs() > 0.5 {
            self.sample_rate = init.sample_rate;
            self.engine = Engine::new(init.sample_rate);
            self.engine.set_gate(1.0);
            self.whistle = Whistle::new(init.sample_rate);
            self.whistle_noise = 0.0;
        self.whistle_air = [0.0; 2];
            self.body_set = false;
            self.mode = usize::MAX;
            self.language = usize::MAX;
        }
        self.plan(init);
    }

    fn reset(&mut self) {
        self.engine.stop_speech();
        self.engine.set_beat_playing(false);
        self.speech_end = None;
        self.beat_end = None;
        self.whistle = Whistle::new(self.sample_rate);
        self.whistle_noise = 0.0;
        self.whistle_air = [0.0; 2];
        self.whistle_jitter = 0.0;
        self.whistle_flow = [[0.0; 2]; 2];
        self.buffer = [0.0; BLOCK];
        self.pos = BLOCK;
        // Timeline restart cancels the score. Restore the prepared phrase without reparsing text.
        if self.mode == SPEECH || self.mode == SING {
            let offset = self.engine.schedule(&self.planned_gestures, self.planned_lead, true);
            self.engine.set_gate(0.0);
            self.speech_end = Some(offset + self.planned_end + 0.08);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    fn init<'a>(params: &'a [f64], texts: &'a [Option<String>]) -> NodeInit<'a> {
        NodeInit {
            id: "chimera1",
            seed: 1,
            sample_rate: 48_000.0,
            params,
            texts,
            curve: None,
            inputs: &[],
            duration: 3.0,
        }
    }

    fn ctx(frame: u64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: 48_000.0,
            dt: 1.0 / 48_000.0,
            t: frame as f64 / 48_000.0,
            seconds: frame as f64 / 48_000.0,
            duration: 3.0,
            mode: GraphMode::Timeline,
            live_frames: frame,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn run(init: &NodeInit<'_>, frames: usize) -> Vec<f64> {
        let mut node = (DEF.create)(init);
        node.configure(init);
        (0..frames)
            .map(|frame| node.process_sample(&ctx(frame as u64), (0.0, 0.0), init.params).l)
            .collect()
    }

    fn rms(samples: &[f64]) -> f64 {
        (samples.iter().map(|x| x * x).sum::<f64>() / samples.len() as f64).sqrt()
    }

    #[test]
    fn speech_plan_has_gestures() {
        let segments = tokenize_ipa("sa va");
        println!(
            "segments {:?}",
            segments.iter().map(|s| s.ipa.clone()).collect::<Vec<_>>()
        );
        let plan = plan_utterance(
            &segments,
            &PlanOptions {
                body: BodyMix::MALE,
                t0: 0.0,
                rate: Some(1.0),
                f0: Some(115.0),
                intonation: Some(Intonation::Statement),
                accent: None,
                open_end: None,
                sung: None,
            },
        );
        println!("gestures {} end {}", plan.gestures.len(), plan.end);
        assert!(!plan.gestures.is_empty());
    }

    #[test]
    fn speech_is_audible_and_finite() {
        let params = [0.0, 0.0, 120.0, 8.0, 1.0, 0.0, 0.0, 100.0, 1.0, 0.1, 0.5];
        let texts: [Option<String>; 11] = [
            None,
            Some("sa va".into()),
            None, None, None, None, None, None, None, None, None,
        ];
        let init = init(&params, &texts);
        let output = run(&init, 96_000);
        assert!(rms(&output) > 0.005, "rms {}", rms(&output));
        assert!(output.iter().all(|x| x.is_finite() && x.abs() <= 1.5));
    }

    #[test]
    fn english_text_is_spoken() {
        let params = [0.0, 0.0, 120.0, 8.0, 1.0, 0.0, 0.0, 100.0, 1.0, 0.1, 0.5, 0.0];
        let texts: [Option<String>; 12] = [
            None,
            Some("Hello. How are you?".into()),
            None, None, None, None, None, None, None, None, None, None,
        ];
        let init = init(&params, &texts);
        let output = run(&init, 144_000);
        let level = rms(&output);
        assert!(level > 0.005, "rms {level}");
        assert!(output.iter().all(|x| x.is_finite() && x.abs() <= 1.5));
    }

    #[test]
    fn whistle_mode_is_audible() {
        let params = [3.0, 0.0, 1_500.0, 10.0, 1.0, 0.0, 0.0, 100.0, 0.8, 0.1, 0.5, 0.0];
        let texts: [Option<String>; 12] = [
            None,
            Some("u".into()),
            None, None, None, None, None, None, None, None, None, None,
        ];
        let init = init(&params, &texts);
        let output = run(&init, 24_000);
        let level = rms(&output);
        assert!(level > 0.005, "rms {level}");
        assert!(output.iter().all(|x| x.is_finite() && x.abs() <= 1.5));
    }

    #[test]
    fn whistle_timbre_defaults_preserve_existing_graphs() {
        let old = [3.0, 0.0, 1500.0, 5.0, 1.0, 0.0, 0.0, 100.0, 0.8, 0.2, 0.5, 0.0, 0.0, 0.14, 0.0];
        let mut explicit = old.to_vec(); explicit.extend([1.0, 1.0]);
        let texts: [Option<String>; 17] = Default::default();
        assert_eq!(run(&init(&old, &texts), 24_000), run(&init(&explicit, &texts), 24_000));
    }

    #[test]
    fn whistle_body_and_edge_control_different_harmonics() {
        let mut params = [3.0, 0.0, 1000.0, 5.0, 1.0, 0.0, 0.0, 100.0, 0.8, 0.0, 0.5, 0.0, 0.0, 0.14, 0.0, 1.0, 1.0];
        let texts: [Option<String>; 17] = Default::default();
        let partial = |y: &[f64], h: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, x) in y[24_000..].iter().enumerate() {
                let phase = core::f64::consts::TAU * 1000.0 * h * i as f64 / 48_000.0;
                re += x * phase.cos(); im += x * phase.sin();
            }
            (re*re+im*im).sqrt()
        };
        let base = run(&init(&params, &texts), 72_000);
        params[WHISTLE_BODY] = 2.0;
        let body = run(&init(&params, &texts), 72_000);
        let even_change = partial(&body, 2.0) / partial(&base, 2.0);
        assert!((1.8..2.2).contains(&even_change), "body ratio {even_change}");
        params[WHISTLE_BODY] = 1.0; params[WHISTLE_EDGE] = 0.25;
        let soft = run(&init(&params, &texts), 72_000);
        let odd_change = (partial(&soft, 3.0)/partial(&soft, 1.0)) / (partial(&base, 3.0)/partial(&base, 1.0));
        assert!(odd_change < 0.4, "odd ratio {odd_change}");
        assert!((rms(&soft[24_000..])/rms(&base[24_000..])-1.0).abs()<0.02);
    }

    #[test]
    fn whistle_flow_has_no_sound_without_pressure() {
        let params = [3.0, 0.0, 1500.0, 0.0, 1.0, 0.0, 0.0, 100.0, 1.0, 1.0, 0.5, 0.0];
        let texts: [Option<String>; 12] = Default::default();
        let output = run(&init(&params, &texts), 24_000);
        assert!(output.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn whistle_full_breath_stays_bounded() {
        let params = [3.0, 0.0, 2800.0, 22.0, 1.0, 0.0, 0.0, 100.0, 0.2, 1.0, 0.5, 0.0];
        let texts: [Option<String>; 12] = Default::default();
        let output = run(&init(&params, &texts), 48_000);
        assert!(output.iter().all(|x| x.is_finite() && x.abs() < 1.0));
        assert!(rms(&output) > 0.01);
    }

    #[test]
    fn voice_mode_sings_a_sustained_vowel() {
        let params = [2.0, 0.0, 1_000.0, 10.0, 1.0, 0.0, 0.0, 100.0, 1.0, 0.05, 0.6];
        let texts: [Option<String>; 11] = [
            None,
            Some("u".into()),
            None, None, None, None, None, None, None, None, None,
        ];
        let init = init(&params, &texts);
        let output = run(&init, 24_000);
        let level = rms(&output);
        assert!(level > 0.005, "rms {level}");
        assert!(output.iter().all(|x| x.is_finite() && x.abs() <= 1.5));
    }

    #[test]
    fn voice_mode_follows_the_vowel_formant_inputs() {
        // steered to an /i/ (F1 300, F2 2200), then to an /a/ (F1 750, F2 1250): the render opens up
        let mut params = [2.0, 0.0, 130.0, 9.0, 1.0, 0.0, 0.0, 100.0, 1.0, 0.05, 0.5, 0.0, 0.0, 0.14, 0.0, 1.0, 1.0, 300.0, 2200.0];
        let texts: [Option<String>; 19] = Default::default();
        // energy of the 130 Hz harmonics between lo and hi Hz
        let band = |y: &[f64], lo: f64, hi: f64| {
            let mut e = 0.0;
            let mut h = 1.0;
            while 130.0 * h < hi {
                let f = 130.0 * h;
                if f >= lo {
                    let (mut re, mut im) = (0.0, 0.0);
                    for (i, x) in y.iter().enumerate() {
                        let ph = core::f64::consts::TAU * f * i as f64 / 48_000.0;
                        re += x * ph.cos();
                        im += x * ph.sin();
                    }
                    e += re * re + im * im;
                }
                h += 1.0;
            }
            e
        };
        let i = run(&init(&params, &texts), 48_000);
        params[VOWEL_F1] = 750.0;
        params[VOWEL_F2] = 1250.0;
        let a = run(&init(&params, &texts), 48_000);
        assert!(i.iter().chain(a.iter()).all(|x| x.is_finite() && x.abs() <= 1.5));
        let (ti, ta) = (&i[24_000..], &a[24_000..]);
        let ri = band(ti, 600.0, 900.0) / band(ti, 200.0, 450.0);
        let ra = band(ta, 600.0, 900.0) / band(ta, 200.0, 450.0);
        assert!(ra > 3.0 * ri, "open/close energy ratio /a/ {ra} vs /i/ {ri}");
    }

    #[test]
    fn beatbox_is_audible_and_finite() {
        let params = [1.0, 0.0, 120.0, 8.0, 1.0, 0.0, 0.0, 120.0, 1.0, 0.1, 0.5];
        let texts: [Option<String>; 11] = [
            None,
            Some("B t tss B".into()),
            None, None, None, None, None, None, None, None, None,
        ];
        let init = init(&params, &texts);
        let output = run(&init, 96_000);
        assert!(rms(&output) > 0.005, "rms {}", rms(&output));
        assert!(output.iter().all(|x| x.is_finite() && x.abs() <= 1.5));
    }

    #[test]
    fn female_body_is_audible() {
        let params = [0.0, 0.0, 200.0, 8.0, 1.0, 1.0, 0.0, 100.0, 1.0, 0.1, 0.5];
        let texts: [Option<String>; 11] = [
            None,
            Some("sa va".into()),
            None, None, None, None, None, None, None, None, None,
        ];
        let init = init(&params, &texts);
        let output = run(&init, 48_000);
        assert!(rms(&output) > 0.005);
        assert!(output.iter().all(|x| x.is_finite() && x.abs() <= 1.5));
    }

    #[test]
    fn sung_phrase_keeps_last_syllable_and_replays() {
        let params = [4.0, 0.0, 150.0, 8.0, 1.0, 0.0, 0.0, 120.0, 0.9, 0.08, 0.5, 0.0, 0.0];
        let mut texts: [Option<String>; 13] = Default::default();
        texts[TEXT] = Some("We can sing a song.".into());
        texts[MELODY] = Some("55:1 57:1 59:1 57:1 55:2".into());
        let init = init(&params, &texts);
        let mut node = (DEF.create)(&init);
        let render = |node: &mut Box<dyn Node>| -> Vec<f64> {
            (0..216_000).map(|i| node.process_sample(&ctx(i), (0.0, 0.0), &params).l).collect()
        };
        let first = render(&mut node);
        assert!(rms(&first[..9_600]) < 0.0001, "voice sounds before planned entry");
        assert!(first.iter().all(|x| x.is_finite() && x.abs() <= 1.5));
        assert!(rms(&first[120_000..144_000]) > 0.003, "last syllable missing");
        assert!(rms(&first[192_000..]) < 0.001, "phrase did not release");
        node.reset();
        let second = render(&mut node);
        assert!(rms(&second[120_000..144_000]) > 0.003, "replay lost phrase");
    }

    #[test]
    fn spoken_sentence_reaches_end_and_releases() {
        let params = [0.0, 0.0, 130.0, 8.0, 1.0, 0.0, 0.0, 100.0, 0.9, 0.08, 0.5, 0.0];
        let mut texts: [Option<String>; 12] = Default::default();
        texts[TEXT] = Some("We can sing a song.".into());
        let init = init(&params, &texts);
        let output = run(&init, 240_000);
        assert!(rms(&output[24_000..72_000]) > 0.003);
        assert!(rms(&output[216_000..]) < 0.001);
    }

    #[test]
    fn reset_stops_voice() {
        let params = [0.0, 0.0, 120.0, 8.0, 1.0, 0.0, 0.0, 100.0, 1.0, 0.1, 0.5];
        let texts: [Option<String>; 11] = [
            None,
            Some("sa va".into()),
            None, None, None, None, None, None, None, None, None,
        ];
        let init = init(&params, &texts);
        let mut node = (DEF.create)(&init);
        node.configure(&init);
        for frame in 0..24_000 {
            let _ = node.process_sample(&ctx(frame), (0.0, 0.0), &params);
        }
        node.reset();
        let first = node.process_sample(&ctx(0), (0.0, 0.0), &params);
        // The score is cancelled; the tract still rings out its last pressure.
        assert!(first.l.abs() < 0.25, "first sample {}", first.l);
    }
}
