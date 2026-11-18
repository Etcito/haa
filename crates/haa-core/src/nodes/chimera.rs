//! `chimera` source node: CHIMERA's speech and beatbox engines as one HAA node.
//!
//! This is the real thing, not an approximation: the node owns a `chimera::Engine` and renders
//! it into HAA's graph. `text` is IPA for speech (`sa va`, `bɔ̃ʒuʁ`) or a beatbox pattern
//! (`B t tss B`) depending on `mode`. In `lyrics` mode it is timed lyrics, one phrase per line
//! (`1.20 I have changed my name so often`): each line is planned once into the physical voice's
//! phoneme gestures (its articulatory targets from the phoneme tables), while the pitch, the level
//! and the speaking rate come from the node's inputs (a note sequence or an intonation curve, a
//! level curve, a rate curve that stretches the vowels).
//!
//! CHIMERA is MIT-licensed; its crate is a sibling checkout (see `Cargo.toml`).
//! Blocks: the engine renders 128 frames at a time into an internal buffer; `process_sample`
//! serves them one by one, so the node keeps HAA's per-sample contract.
//! `configure` (live swap, load time) is where text is planned and the body rebuilt; it may
//! allocate. `process_sample` and `reset` never allocate.

use ::chimera::anatomy::human::BodyMix;
use ::chimera::engine::Engine;
use ::chimera::control::score::{ControlKey, Gesture};
use ::chimera::phonetics::beatbox::parse_beat;
use ::chimera::phonetics::english::EnglishG2P;
use ::chimera::phonetics::ipa::tokenize_ipa;
use ::chimera::anatomy::human::{rest_tip, ArticKey, ArticPatch, NEUTRAL};
use ::chimera::phonetics::inventory::phoneme;
use ::chimera::phonetics::planner::{expand, plan_phrases, plan_utterance, vowel_artic, vowel_table, PhraseInput, PlanOptions};
use ::chimera::phonetics::{Intonation, Segment};
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
const VOWEL_F3: usize = 19;
const VOICING: usize = 20;
const FRICATION: usize = 21;
const FRIC_PLACE: usize = 22;
const CLOSURE: usize = 23;
const BRIGHTNESS: usize = 24;
const VIBRATO_DEPTH: usize = 25;
const VIBRATO_RATE: usize = 26;
const GLIDE: usize = 27;
const TRACT: usize = 28;
const PRESENCE: usize = 29;
const NASAL: usize = 30;

const SPEECH: usize = 0;
const VOICE: usize = 2;
const WHISTLE: usize = 3;
const SING: usize = 4;
const LYRICS: usize = 5;
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
                values: &["speech", "beatbox", "voice", "whistle", "sing", "lyrics"],
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
        // speech and sing plan at this rate (0.5-2); `lyrics` plays its lines at it, modulated by a rate curve: a
        // vowel (or a nasal, a lateral) is played this much faster (> 1) or slower (< 1), another consonant never slower
        // than its own length
        ParamDef {
            key: "rate",
            kind: ParamKind::Number {
                min: 0.1,
                max: 4.0,
                default: 1.0,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "×",
            label_en: "Rate",
            label_fr: "Débit",
            modulatable: true,
            smooth: true,
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
        // `voice` mode, what a sung or spoken line does besides its vowels. F3 (0 = ignored) joins F1/F2 in choosing
        // the vowel (rounding, r-colouring). Voicing 0 stops the folds (a whisper, /h/, the voiceless part of a
        // consonant); frication moves the tongue tip or the lips to a fricative constriction whose jet makes the
        // tract's own turbulence noise (place 0 /f/, 0.5 /ʃ/, 1 /s/); closure seals the tract at the alveolar
        // ridge, and its release under pressure is a plosive burst. Brightness and vibrato are the folds'.
        ParamDef {
            key: "vowelF3",
            kind: ParamKind::Number { min: 0.0, max: 4_000.0, default: 0.0, step: 1.0, scale: Scale::Lin },
            unit: "Hz", label_en: "Vowel F3", label_fr: "Voyelle F3",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "voicing",
            kind: ParamKind::Number { min: 0.0, max: 1.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Voicing (folds)", label_fr: "Voisement (cordes)",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "frication",
            kind: ParamKind::Number { min: 0.0, max: 1.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Frication (constriction)", label_fr: "Friction (constriction)",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "fricPlace",
            kind: ParamKind::Number { min: 0.0, max: 1.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Frication place (f, sh, s)", label_fr: "Lieu de friction (f, ch, s)",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "closure",
            kind: ParamKind::Number { min: 0.0, max: 1.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Closure (plosive)", label_fr: "Occlusion (plosive)",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "brightness",
            kind: ParamKind::Number { min: 0.0, max: 1.0, default: 0.25, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Brightness (fold closure)", label_fr: "Brillance (fermeture des cordes)",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "vibratoDepth",
            kind: ParamKind::Number { min: 0.0, max: 200.0, default: 0.0, step: 1.0, scale: Scale::Lin },
            unit: "cents", label_en: "Vibrato depth", label_fr: "Profondeur du vibrato",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "vibratoRate",
            kind: ParamKind::Number { min: 2.0, max: 10.0, default: 5.5, step: 0.1, scale: Scale::Lin },
            unit: "Hz", label_en: "Vibrato rate", label_fr: "Vitesse du vibrato",
            modulatable: true, smooth: true,
        },
        // `voice` mode: how fast the folds follow the pitch input (time constant). The default glides like a performer
        // between held notes; a measured pitch curve, which already holds its glides and vibrato, wants it short.
        ParamDef {
            key: "glide",
            kind: ParamKind::Number { min: 0.002, max: 0.3, default: 0.06, step: 0.001, scale: Scale::Log },
            unit: "s", label_en: "Pitch glide", label_fr: "Glissé de hauteur",
            modulatable: false, smooth: false,
        },
        // `voice` and `lyrics` modes, the timbre a singer adds to the body: the tract's length against the body's (> 1
        // longer: every formant lower by that ratio; < 1 shorter, higher), and the singer's formant, a peak of the
        // radiated sound around 3.3 kHz (scaled with the tract) whose gain is this many dB (0 = none).
        ParamDef {
            key: "tract",
            kind: ParamKind::Number { min: 0.7, max: 1.4, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "×", label_en: "Tract length", label_fr: "Longueur du conduit",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "presence",
            kind: ParamKind::Number { min: -12.0, max: 18.0, default: 0.0, step: 0.1, scale: Scale::Lin },
            unit: "dB", label_en: "Presence (singer's formant)", label_fr: "Présence (formant du chanteur)",
            modulatable: true, smooth: true,
        },
        // `voice` and `lyrics` modes: a velopharyngeal port (cm²) left open under the velum's own gestures, for a
        // singer who sings a little through the nose (0 = none; 0.1-0.3 a nasal voice, 0.5 a nasal vowel). The nose
        // adds a low pole (~250-300 Hz) and the sinuses' zero (~440 Hz) dents the band above it.
        ParamDef {
            key: "nasal",
            kind: ParamKind::Number { min: 0.0, max: 0.6, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "cm²", label_en: "Nasality", label_fr: "Nasalité",
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
        text_vowel: ArticPatch::default(),
        consonants: [ArticPatch::default(); 4],
        parts: Vec::new(),
        steer_at: [f64::NAN; 6],
        lyrics: Vec::new(),
        lyric_next: 0,
        lyric_now: None,
        last_seconds: 0.0,
        presence: Presence::default(),
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
    /// `voice` mode: the calibrated vowels of the current body, (ln F1, ln F2, ln F3, articulation).
    vowels: Vec<(f64, f64, f64, ArticPatch)>,
    /// `voice` mode: the articulation of the `text` vowel (used while no formant input steers).
    text_vowel: ArticPatch,
    /// `voice` mode: the calibrated targets of /f/, /ʃ/, /s/ and /t/ for the current body.
    consonants: [ArticPatch; 4],
    /// Blend weights scratch (capacity reserved at plan time: steering never allocates).
    parts: Vec<(f64, ArticPatch)>,
    /// (F1, F2, F3, frication, place, closure) the tract was last steered to.
    steer_at: [f64; 6],
    /// `lyrics` mode: the planned lines, the next one to schedule, the line playing (index, its score offset),
    /// and the patch time of the last block (a smaller one is a restart).
    lyrics: Vec<LyricLine>,
    lyric_next: usize,
    lyric_now: Option<(usize, f64)>,
    last_seconds: f64,
    /// `voice` and `lyrics` modes: the singer's formant on the rendered block.
    presence: Presence,
}

/// The singer's formant as a peaking filter (RBJ biquad, Q 1.4, centre 3.3 kHz / tract) on the voice's output:
/// coefficients per block, state across blocks. At 0 dB it passes the block untouched.
#[derive(Default)]
struct Presence {
    /// (gain dB, tract) the coefficients were made for
    at: (f64, f64),
    /// b0, b1, b2, a1, a2 (a0 = 1)
    c: [f64; 5],
    z: [f64; 2],
}

impl Presence {
    const HZ: f64 = 3300.0;
    const Q: f64 = 1.4;

    fn process(&mut self, block: &mut [f32], gain_db: f64, tract: f64, sample_rate: f64) {
        if gain_db.abs() < 0.01 {
            self.z = [0.0; 2];
            self.at = (0.0, 0.0);
            return;
        }
        if (gain_db - self.at.0).abs() > 1e-3 || (tract - self.at.1).abs() > 1e-4 {
            self.at = (gain_db, tract);
            let f = (Self::HZ / tract.max(0.1)).min(0.45 * sample_rate);
            let a = 10f64.powf(gain_db / 40.0);
            let w = core::f64::consts::TAU * f / sample_rate;
            let alpha = w.sin() / (2.0 * Self::Q);
            let a0 = 1.0 + alpha / a;
            self.c = [(1.0 + alpha * a) / a0, -2.0 * w.cos() / a0, (1.0 - alpha * a) / a0, -2.0 * w.cos() / a0, (1.0 - alpha / a) / a0];
        }
        let [b0, b1, b2, a1, a2] = self.c;
        let [mut z1, mut z2] = self.z;
        for s in block.iter_mut() {
            let x = *s as f64;
            let y = b0 * x + z1;
            z1 = b1 * x - a1 * y + z2;
            z2 = b2 * x - a2 * y;
            *s = y as f32;
        }
        self.z = if z1.is_finite() && z2.is_finite() { [z1, z2] } else { [0.0; 2] };
    }
}

/// `lyrics` mode: one line planned at rate 1, its score time 0 the line's start minus [`LYRIC_LEAD`].
pub struct LyricLine {
    /// patch time (s) of the line's first segment
    pub start: f64,
    pub gestures: Vec<Gesture>,
    /// (t0, t1, held) of each planned segment, on the line's score time: held for a vowel, a nasal or a lateral (the
    /// sounds a singer holds, which a rate under 1 stretches)
    pub spans: Vec<(f64, f64, bool)>,
    /// the IPA of each span
    pub phones: Vec<String>,
    /// each word said and its (t0, t1) on the line's score time
    pub words: Vec<(String, f64, f64)>,
}

/// A planned utterance's breath lead: its first segment starts this long after the plan's origin.
pub const LYRIC_LEAD: f64 = 0.06;
/// The pause before a line written without its start time (s).
const LYRIC_PAUSE: f64 = 0.3;

/// Plans `lyrics` text (see [`lyric_lines`]): per line, English G2P (or IPA words) into the physical voice's phoneme
/// gestures at rate 1, its pitch and vibrato gestures dropped (the pitch input sings the line), sorted by start.
pub fn plan_lyrics(text: &str, ipa: bool, female: f64, child: f64, pitch: f64) -> Vec<LyricLine> {
    ::chimera::init();
    let body = BodyMix::new(female.clamp(0.0, 1.0), child.clamp(0.0, 1.0));
    let mut lines = Vec::new();
    let mut next_start = LYRIC_PAUSE;
    for (start, words) in lyric_lines(text) {
        // the segments, and the words they say (each word's last segment marks its end)
        let (segments, said): (Vec<Segment>, Vec<String>) = if ipa {
            let mut segs = Vec::new();
            let mut said = Vec::new();
            for w in words.split_whitespace() {
                let mut ws = tokenize_ipa(w);
                if let Some(last) = ws.last_mut() {
                    last.word_end = Some(true);
                    segs.append(&mut ws);
                    said.push(w.to_string());
                }
            }
            (segs, said)
        } else {
            let phrases = english_g2p().to_phrases(words);
            let said = phrases.iter().flat_map(|p| p.words.iter().cloned()).collect();
            (phrases.into_iter().flat_map(|p| p.segments).collect(), said)
        };
        if segments.is_empty() {
            continue;
        }
        let mut plan = plan_utterance(&segments, &PlanOptions {
            body, t0: 0.0, rate: Some(1.0), f0: Some(pitch), intonation: Some(Intonation::Flat), accent: Some(0.0),
            ..Default::default()
        });
        plan.gestures.retain(|g| !g.targets.has(ControlKey::F0) && !g.targets.has(ControlKey::VIBRATO));
        // word spans: the planner splits a diphthong into two timed parts
        let mut words_t = Vec::new();
        let (mut k, mut w0, mut wi) = (0usize, f64::NAN, 0usize);
        for seg in &segments {
            let parts = expand(&seg.ipa).map_or(1, |p| p.len().max(1));
            let (Some(first), Some(last)) = (plan.segments.get(k), plan.segments.get(k + parts - 1)) else { break };
            if w0.is_nan() {
                w0 = first.t0;
            }
            k += parts;
            if seg.word_end == Some(true) {
                words_t.push((said.get(wi).cloned().unwrap_or_else(|| format!("w{}", wi + 1)), w0, last.t1));
                wi += 1;
                w0 = f64::NAN;
            }
        }
        let start = start.unwrap_or(next_start);
        next_start = start + plan.end - LYRIC_LEAD + LYRIC_PAUSE;
        let spans = plan.segments.iter().map(|s| (s.t0, s.t1, matches!(s.kind.as_str(), "vowel" | "nasal" | "lateral"))).collect();
        let phones = plan.segments.iter().map(|s| s.ipa.clone()).collect();
        lines.push(LyricLine { start, gestures: plan.gestures, spans, phones, words: words_t });
    }
    lines.sort_by(|a, b| a.start.total_cmp(&b.start));
    lines
}

/// How fast a line's score runs at `rate` when it stands at score time `tau`: `rate` in a held sound (a vowel, a nasal,
/// a lateral), never slower than 1 in another consonant, a pause or a breath (a held note stretches what a singer holds).
pub fn lyric_speed(line: &LyricLine, tau: f64, rate: f64) -> f64 {
    let r = if rate.is_finite() { rate.clamp(0.1, 4.0) } else { 1.0 };
    if lyric_held(line, tau) { r } else { r.max(1.0) }
}

/// Whether a line's score time `tau` is inside one of its held sounds (a vowel, a nasal, a lateral).
pub fn lyric_held(line: &LyricLine, tau: f64) -> bool {
    let i = line.spans.partition_point(|s| s.1 <= tau);
    matches!(line.spans.get(i), Some(&(t0, t1, true)) if tau >= t0 && tau < t1)
}

/// When each word of each line is said (patch seconds) if the node plays them at `rate_at(t)`: the score clock of
/// [`lyric_speed`] integrated every millisecond from the line's scheduling. Per line: (start, end, words).
pub fn lyrics_timing(lines: &[LyricLine], rate_at: impl Fn(f64) -> f64) -> Vec<(f64, f64, Vec<(String, f64, f64)>)> {
    let dt = 0.001;
    lines
        .iter()
        .map(|line| {
            let end_tau = line.spans.last().map_or(0.0, |s| s.1);
            let mut targets: Vec<f64> = line.words.iter().flat_map(|w| [w.1, w.2]).collect();
            targets.push(end_tau);
            let mut at = vec![f64::NAN; targets.len()];
            let (mut t, mut tau) = (line.start - LYRIC_LEAD, 0.0);
            let mut q = 0;
            while q < targets.len() && t < line.start + 120.0 {
                while q < targets.len() && tau >= targets[q] {
                    at[q] = t;
                    q += 1;
                }
                tau += lyric_speed(line, tau, rate_at(t)) * dt;
                t += dt;
            }
            let words = line.words.iter().enumerate().map(|(i, w)| (w.0.clone(), at[2 * i], at[2 * i + 1])).collect();
            (line.start, at[targets.len() - 1], words)
        })
        .collect()
}

/// When a lyrics line says each of its sounds, as [`lyrics_timing`] times its words: per line, (IPA, start, end) in
/// patch seconds (NaN where the plan never reaches it).
pub fn lyrics_phone_timing(lines: &[LyricLine], rate_at: impl Fn(f64) -> f64) -> Vec<Vec<(String, f64, f64)>> {
    let dt = 0.001;
    lines
        .iter()
        .map(|line| {
            // score times in order, and where each lands
            let mut order: Vec<(f64, usize)> = line.spans.iter().enumerate().flat_map(|(i, s)| [(s.0, 2 * i), (s.1, 2 * i + 1)]).collect();
            order.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut at = vec![f64::NAN; order.len()];
            let (mut t, mut tau) = (line.start - LYRIC_LEAD, 0.0);
            let mut q = 0;
            while q < order.len() && t < line.start + 120.0 {
                while q < order.len() && tau >= order[q].0 {
                    at[order[q].1] = t;
                    q += 1;
                }
                tau += lyric_speed(line, tau, rate_at(t)) * dt;
                t += dt;
            }
            line.phones.iter().enumerate().map(|(i, p)| (p.clone(), at[2 * i], at[2 * i + 1])).collect()
        })
        .collect()
}

/// `lyrics` text as (start, words) per line: `<seconds> words`, or words alone (the line follows the previous one).
fn lyric_lines(text: &str) -> Vec<(Option<f64>, &str)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (head, rest) = l.split_once(char::is_whitespace).unwrap_or((l, ""));
            match head.parse::<f64>() {
                Ok(t) if t.is_finite() && (0.0..=3_600.0).contains(&t) => (Some(t), rest.trim()),
                _ => (None, l),
            }
        })
        .filter(|(_, words)| !words.is_empty())
        .collect()
}

/// The calibrated vowels of a body mix: formants blended by the body weights, articulation from `vowel_artic`.
fn body_vowels(body: BodyMix) -> Vec<(f64, f64, f64, ArticPatch)> {
    let table = vowel_table();
    let Some(male) = table.get("male") else { return Vec::new() };
    let mut out = Vec::new();
    for ipa in male.keys() {
        let (mut f1, mut f2, mut f3, mut wsum) = (0.0, 0.0, 0.0, 0.0);
        for (b, w) in body.weights() {
            if w <= 0.0 {
                continue;
            }
            let Some(v) = table.get(b.as_str()).and_then(|t| t.get(ipa)).or_else(|| male.get(ipa)) else { continue };
            if v.formants.len() < 3 {
                continue;
            }
            f1 += w * v.formants[0];
            f2 += w * v.formants[1];
            f3 += w * v.formants[2];
            wsum += w;
        }
        if let (true, Some(artic)) = (wsum > 0.0, vowel_artic(ipa, body)) {
            out.push(((f1 / wsum).ln(), (f2 / wsum).ln(), (f3 / wsum).ln(), artic));
        }
    }
    out
}

/// `{ ...base, ...over }`: a patch with every key of `base` that `over` leaves out.
fn over(base: &ArticPatch, over: &ArticPatch) -> ArticPatch {
    let mut out = *base;
    out.merge(over);
    out
}

impl ChimeraNode {
    /// Steer the tract. The vowel: Shepard blend (inverse distance, power 4) of every calibrated vowel of the body by
    /// distance in ln formants (F2 counted at 0.6 of F1, F3 at 0.4 when given), so the blend moves continuously as the
    /// target moves (no jump when the nearest vowels change) and reaches the corners the vowels span. Frication and
    /// closure then pull the articulation toward a consonant target (/f/-/ʃ/-/s/ by place, /t/ for a closure) built
    /// on that vowel, so only the consonant's own articulators move. Never allocates.
    fn steer(&mut self, f1: f64, f2: f64, f3: f64, fric: f64, place: f64, closure: f64) {
        if self.vowels.is_empty() {
            return;
        }
        let key = [f1, f2, f3, fric, place, closure];
        let close = |a: f64, b: f64, rel: bool| if rel { (a.max(1.0) / b.max(1.0)).ln().abs() < 0.01 } else { (a - b).abs() < 0.01 };
        if (0..6).all(|i| close(key[i], self.steer_at[i], i < 3)) {
            return;
        }
        self.steer_at = key;
        let vowel = if f1 > 0.0 && f2 > 0.0 {
            let (l1, l2, l3) = (f1.ln(), f2.ln(), if f3 > 0.0 { f3.ln() } else { 0.0 });
            self.parts.clear();
            for (v1, v2, v3, artic) in &self.vowels {
                let mut d2 = (l1 - v1).powi(2) + (0.6 * (l2 - v2)).powi(2);
                if f3 > 0.0 {
                    d2 += (0.4 * (l3 - v3)).powi(2);
                }
                self.parts.push((1.0 / (d2.sqrt() + 0.02).powi(4), *artic));
            }
            ArticPatch::blend(&self.parts)
        } else {
            self.text_vowel
        };
        let amount = fric.max(closure).clamp(0.0, 1.0);
        if amount < 1e-3 {
            // a pure vowel: the tongue tip rests behind the incisors, as before frication existed
            self.engine.set_artic(&vowel, Some(true));
            return;
        }
        // every key the consonants move gets a vowel value: the neutral posture where the vowel says nothing, the
        // tip where it rests for this jaw and tongue body
        let mut base = ArticPatch::default();
        for k in ArticKey::ALL {
            base.set(k, NEUTRAL.get(k));
        }
        base.merge(&vowel);
        if vowel.get(ArticKey::TtX).is_none() {
            let g = |k: ArticKey| base.get(k).unwrap_or(NEUTRAL.get(k));
            let tip = rest_tip(g(ArticKey::Jaw), g(ArticKey::TbX), g(ArticKey::TbY));
            base.set(ArticKey::TtX, tip.x);
            base.set(ArticKey::TtY, tip.y);
        }
        let p = place.clamp(0.0, 1.0);
        let [f, sh, s, t] = self.consonants;
        let fricative = if p < 0.5 {
            ArticPatch::blend(&[(1.0 - 2.0 * p, over(&base, &f)), (2.0 * p, over(&base, &sh))])
        } else {
            ArticPatch::blend(&[(2.0 - 2.0 * p, over(&base, &sh)), (2.0 * p - 1.0, over(&base, &s))])
        };
        let c = closure.clamp(0.0, 1.0);
        let consonant = ArticPatch::blend(&[(1.0 - c, fricative), (c, over(&base, &t))]);
        self.engine.set_artic(&ArticPatch::blend(&[(1.0 - amount, base), (amount, consonant)]), Some(false));
    }
}

impl ChimeraNode {
    /// `lyrics` mode, once per block: schedule the lines the patch time `now` reached, and play the score at the speed
    /// [`lyric_speed`] gives `rate` there.
    fn play_lyrics(&mut self, now: f64, rate: f64) {
        if now + 1e-9 < self.last_seconds {
            // the timeline restarted or looped
            self.engine.stop_speech();
            self.lyric_next = 0;
            self.lyric_now = None;
        }
        self.last_seconds = now;
        while self.lyric_next < self.lyrics.len() && now >= self.lyrics[self.lyric_next].start - LYRIC_LEAD {
            let off = self.engine.schedule(&self.lyrics[self.lyric_next].gestures, 0.0, false);
            self.lyric_now = Some((self.lyric_next, off));
            self.lyric_next += 1;
        }
        let eff = match self.lyric_now {
            Some((i, off)) => lyric_speed(&self.lyrics[i], self.engine.score_time() - off, rate),
            None => 1.0,
        };
        self.engine.set_score_rate(eff);
    }

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
            } else if mode == LYRICS {
                // Each line becomes one breath group of the physical voice's phoneme gestures (G2P or IPA), timed at
                // rate 1; its pitch gestures are dropped (the pitch input sings it) and it is scheduled when the
                // patch reaches the line's start.
                self.engine.set_beat_playing(false);
                self.engine.stop_speech();
                let lines = plan_lyrics(text, language == IPA, female, child, pitch);
                self.lyrics = lines;
                // a live edit plays from the next line on
                self.lyric_next = self.lyrics.partition_point(|l| l.start - LYRIC_LEAD < self.last_seconds);
                self.lyric_now = None;
                self.engine.set_gate(0.0); // each line's plan owns its breath
                self.speech_end = None;
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
                let body = BodyMix::new(female, child);
                if let Some(artic) = vowel_artic(vowel, body) {
                    self.engine.set_artic(&artic, None);
                    self.text_vowel = artic;
                }
                self.vowels = body_vowels(body);
                self.consonants = ["f", "ʃ", "s", "t"].map(|c| phoneme(c, body).map(|p| p.targets).unwrap_or_default());
                self.parts = Vec::with_capacity(self.vowels.len());
                self.steer_at = [f64::NAN; 6];
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
        ctx: &SampleCtx<'_>,
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
            let mut patch = SourcePatch {
                f0: Some(params[PITCH]),
                pressure: Some(params[PRESSURE]),
                breathiness: Some(params[BREATH]),
                tension: Some(params[TENSION]),
                ..SourcePatch::default()
            };
            let at = |i: usize, d: f64| params.get(i).copied().unwrap_or(d);
            let voicing = at(VOICING, 1.0).clamp(0.0, 1.0);
            if self.mode == LYRICS {
                self.play_lyrics(ctx.seconds, at(RATE, 1.0));
            }
            if self.mode == VOICE || self.mode == LYRICS {
                // under ~1/3 voicing the folds part (voiceless abduction: aspiration); above, they vibrate with an
                // amplitude that follows the voicing, so a voiced sound fades into a voiceless one (voice mode; in
                // lyrics mode the planned consonants part the folds)
                if self.mode == VOICE {
                    patch.voiced = Some(voicing >= 0.35);
                }
                patch.brightness = Some(at(BRIGHTNESS, 0.25).clamp(0.0, 1.0));
                patch.vibrato_depth = Some(at(VIBRATO_DEPTH, 0.0).clamp(0.0, 200.0));
                patch.vibrato_rate = Some(at(VIBRATO_RATE, 5.5).clamp(2.0, 10.0));
                patch.glide = Some(at(GLIDE, 0.06).clamp(0.002, 0.3));
                self.engine.voice_mut().airway.length_scale = at(TRACT, 1.0).clamp(0.7, 1.4);
                self.engine.voice_mut().airway.nasal_leak = at(NASAL, 0.0).clamp(0.0, 0.6);
            }
            self.engine.set_source(&patch);
            if self.mode == VOICE {
                self.engine.voice_mut().glottis.params.amplitude *= voicing;
                self.steer(
                    at(VOWEL_F1, 0.0),
                    at(VOWEL_F2, 0.0),
                    at(VOWEL_F3, 0.0),
                    at(FRICATION, 0.0),
                    at(FRIC_PLACE, 1.0),
                    at(CLOSURE, 0.0),
                );
            }
            self.engine.render(&mut self.buffer);
            if self.mode == VOICE || self.mode == LYRICS {
                let tract = at(TRACT, 1.0).clamp(0.7, 1.4);
                self.presence.process(&mut self.buffer, at(PRESENCE, 0.0).clamp(-12.0, 18.0), tract, self.sample_rate);
            }
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
        self.lyric_next = 0;
        self.lyric_now = None;
        self.last_seconds = 0.0;
        self.presence = Presence::default();
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
    fn tract_length_lowers_the_formants_and_presence_lifts_the_singers_formant_band() {
        // a held /a/ at 130 Hz: a 25% longer tract moves its F2 down; +12 dB of presence lifts 2.5-4.5 kHz
        let mut params = [0.0; 30];
        params[..19].copy_from_slice(&[2.0, 0.0, 130.0, 9.0, 1.0, 0.0, 0.0, 100.0, 1.0, 0.05, 0.5, 0.0, 0.0, 0.14, 0.0, 1.0, 1.0, 750.0, 1250.0]);
        params[VOICING] = 1.0;
        params[FRIC_PLACE] = 1.0;
        params[BRIGHTNESS] = 0.25;
        params[VIBRATO_RATE] = 5.5;
        params[GLIDE] = 0.06;
        params[TRACT] = 1.0;
        let texts: [Option<String>; 30] = Default::default();
        // energy of each 130 Hz harmonic under 6 kHz, on the second half
        let harmonics = |y: &[f64]| -> Vec<(f64, f64)> {
            let y = &y[24_000..];
            (1..46)
                .map(|h| {
                    let f = 130.0 * h as f64;
                    let (mut re, mut im) = (0.0, 0.0);
                    for (i, x) in y.iter().enumerate() {
                        let ph = core::f64::consts::TAU * f * i as f64 / 48_000.0;
                        re += x * ph.cos();
                        im += x * ph.sin();
                    }
                    (f, re * re + im * im)
                })
                .collect()
        };
        // the /a/'s second formant: its loudest harmonic between 900 and 1800 Hz
        let f2 = |hs: &[(f64, f64)]| hs.iter().filter(|(f, _)| (900.0..1800.0).contains(f)).fold((0.0, 0.0), |m, h| if h.1 > m.1 { *h } else { m }).0;
        let share = |hs: &[(f64, f64)]| {
            let band: f64 = hs.iter().filter(|(f, _)| (2500.0..4500.0).contains(f)).map(|(_, e)| e).sum();
            10.0 * (band / hs.iter().map(|(_, e)| e).sum::<f64>()).log10()
        };
        let base = run(&init(&params, &texts), 48_000);
        params[TRACT] = 1.25;
        let long = run(&init(&params, &texts), 48_000);
        params[TRACT] = 1.0;
        params[PRESENCE] = 12.0;
        let present = run(&init(&params, &texts), 48_000);
        assert!(base.iter().chain(&long).chain(&present).all(|x| x.is_finite() && x.abs() <= 1.5));
        let (hb, hl, hp) = (harmonics(&base), harmonics(&long), harmonics(&present));
        assert!(f2(&hl) < 0.95 * f2(&hb), "F2: long tract {} vs {}", f2(&hl), f2(&hb));
        assert!(share(&hp) > share(&hb) + 6.0, "2.5-4.5 kHz share: presence {} vs {}", share(&hp), share(&hb));
    }

    #[test]
    fn nasality_moves_a_held_schwa_from_its_f1_band_to_the_noses_low_pole() {
        // a held /ə/ at 130 Hz, oral then with 0.3 cm² of velopharyngeal port: the 400-650 Hz harmonics (F1's band)
        // lose their share to the 200-330 Hz ones (the nose's pole)
        let mut params = [0.0; 31];
        params[..17].copy_from_slice(&[2.0, 0.0, 130.0, 9.0, 1.0, 0.0, 0.0, 100.0, 1.0, 0.05, 0.5, 0.0, 0.0, 0.14, 0.0, 1.0, 1.0]);
        params[VOICING] = 1.0;
        params[FRIC_PLACE] = 1.0;
        params[BRIGHTNESS] = 0.25;
        params[VIBRATO_RATE] = 5.5;
        params[GLIDE] = 0.06;
        params[TRACT] = 1.0;
        let mut texts: [Option<String>; 31] = Default::default();
        texts[TEXT] = Some("ə".into());
        let share = |y: &[f64], lo: f64, hi: f64| -> f64 {
            let y = &y[24_000..];
            let e: Vec<(f64, f64)> = (1..16)
                .map(|h| {
                    let f = 130.0 * h as f64;
                    let (mut re, mut im) = (0.0, 0.0);
                    for (i, x) in y.iter().enumerate() {
                        let ph = core::f64::consts::TAU * f * i as f64 / 48_000.0;
                        re += x * ph.cos();
                        im += x * ph.sin();
                    }
                    (f, re * re + im * im)
                })
                .collect();
            let band: f64 = e.iter().filter(|(f, _)| (lo..hi).contains(f)).map(|(_, p)| p).sum();
            10.0 * (band / e.iter().map(|(_, p)| p).sum::<f64>()).log10()
        };
        let oral = run(&init(&params, &texts), 48_000);
        params[NASAL] = 0.3;
        let nasal = run(&init(&params, &texts), 48_000);
        assert!(oral.iter().chain(&nasal).all(|x| x.is_finite() && x.abs() <= 1.5));
        let (f1_oral, f1_nasal) = (share(&oral, 400.0, 650.0), share(&nasal, 400.0, 650.0));
        let (low_oral, low_nasal) = (share(&oral, 200.0, 330.0), share(&nasal, 200.0, 330.0));
        assert!(f1_nasal < f1_oral - 2.0, "400-650 Hz share: nasal {f1_nasal:.1} dB vs oral {f1_oral:.1} dB");
        assert!(low_nasal > low_oral + 2.0, "200-330 Hz share: nasal {low_nasal:.1} dB vs oral {low_oral:.1} dB");
    }

    #[test]
    fn voice_mode_frication_turns_the_vowel_into_a_voiceless_hiss() {
        // the same /a/ steering, then voiceless with the tongue tip at /s/: the tract's own jet noise, high and audible
        let mut params = [
            2.0, 0.0, 130.0, 10.0, 1.0, 0.0, 0.0, 100.0, 1.0, 0.1, 0.5, 0.0, 0.0, 0.14, 0.0, 1.0, 1.0, 750.0, 1250.0,
            0.0, 1.0, 0.0, 1.0, 0.0, 0.25, 0.0, 5.5,
        ];
        let texts: [Option<String>; 27] = Default::default();
        // share of the signal's power left after two first differences (+12 dB/octave): high when the energy sits high
        let high = |y: &[f64]| {
            let d2: Vec<f64> = y.windows(3).map(|w| w[2] - 2.0 * w[1] + w[0]).collect();
            rms(&d2) / rms(y)
        };
        let vowel = run(&init(&params, &texts), 48_000);
        params[VOICING] = 0.0;
        params[FRICATION] = 1.0;
        let hiss = run(&init(&params, &texts), 48_000);
        assert!(vowel.iter().chain(hiss.iter()).all(|x| x.is_finite() && x.abs() <= 1.5));
        let (v, s) = (&vowel[24_000..], &hiss[24_000..]);
        assert!(rms(s) > 0.002, "hiss rms {}", rms(s));
        assert!(high(s) > 4.0 * high(v), "high share: hiss {} vowel {}", high(s), high(v));
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
    fn lyrics_line_starts_on_time_and_its_rate_stretches_the_vowels() {
        let mut params = [
            5.0, 0.0, 120.0, 8.0, 1.0, 0.0, 0.0, 100.0, 1.0, 0.1, 0.5, 0.0, 0.0, 0.14, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0,
            0.0, 0.25, 0.0, 5.5, 0.06,
        ];
        let mut texts: [Option<String>; 28] = Default::default();
        texts[TEXT] = Some("0.5 so often".into());
        // the start of the last 10 ms window still sounding
        let end = |out: &[f64]| (0..out.len() / 480).rev().find(|&w| rms(&out[w * 480..(w + 1) * 480]) > 0.002).map_or(0.0, |w| w as f64 * 0.01);
        let normal = run(&init(&params, &texts), 144_000);
        assert!(normal.iter().all(|x| x.is_finite()));
        assert!(rms(&normal[..19_000]) < 0.0005, "voice sounds before the line starts");
        assert!(rms(&normal[24_000..48_000]) > 0.003, "line not sung");
        params[RATE] = 0.4;
        let slow = run(&init(&params, &texts), 144_000);
        let (a, b) = (end(&normal), end(&slow));
        assert!(b > a + 0.15, "rate 0.4 ends at {b} s, rate 1 at {a} s");
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
