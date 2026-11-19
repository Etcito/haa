//! `chimera` source node: CHIMERA's speech and beatbox engines as one HAA node.
//!
//! This is the real thing, not an approximation: the node owns a `chimera::Engine` and renders
//! it into HAA's graph. `text` is IPA for speech (`sa va`, `bɔ̃ʒuʁ`) or a beatbox pattern
//! (`B t tss B`) depending on `mode`. In `lyrics` mode it is timed lyrics, one phrase per line
//! (`1.20 I have changed my name so often`): each line is planned once into the physical voice's
//! phoneme gestures (its articulatory targets from the phoneme tables), while the pitch, the level
//! and the speaking rate come from the node's inputs (a note sequence or an intonation curve, a
//! level curve, a rate curve that stretches the vowels). In `articulate` mode nothing is planned: the articulator
//! inputs (jaw, tongue body, tongue tip, lips, ...) are the tract's targets, so measured articulator curves play it
//! directly, while voicing, frication and closure act as in `voice` mode.
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
use ::chimera::phonetics::ipa::{skipped_ipa, tokenize_ipa};
use ::chimera::anatomy::human::{jaw_carry, rest_tip, ArticKey, ArticPatch, NEUTRAL};
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
const LARYNX: usize = 31;
const JAW: usize = 32;
const TONGUE_HEIGHT: usize = 33;
const TONGUE_FRONT: usize = 34;
const LIP_ROUND: usize = 35;
const LIP_OPEN: usize = 36;
const EPILARYNX: usize = 37;
const OPEN_QUOTIENT: usize = 38;
const FRY: usize = 39;
const ROUGHNESS: usize = 40;
const WALLS: usize = 41;
const ASPIRATION: usize = 42;
const VELUM: usize = 43;
const CLOSURE_SHARPNESS: usize = 44;
const SKEW: usize = 45;
const TONGUE_ROOT: usize = 46;
const LIP_WIDTH: usize = 47;
const GLOTTAL_GAP: usize = 48;
const WALL_MASS: usize = 49;
const SINUS: usize = 50;
const FOSSA: usize = 51;
const TIP_UP: usize = 52;
const TIP_FRONT: usize = 53;

const SPEECH: usize = 0;
const VOICE: usize = 2;
const WHISTLE: usize = 3;
const SING: usize = 4;
const LYRICS: usize = 5;
const ARTICULATE: usize = 6;
const ENGLISH: usize = 0;
const IPA: usize = 1;
const FRENCH: usize = 2;
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
                values: &["speech", "beatbox", "voice", "whistle", "sing", "lyrics", "articulate"],
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
                values: &["english", "ipa", "french"],
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
        // `voice` and `lyrics` modes, the singer's own habit over every sound the plan (or the formant steering) asks
        // for, added to its articulator targets (0 = none, every existing patch unchanged): the larynx raised (> 0)
        // or lowered (< 0, a longer, darker tract), the jaw more open (> 0: F1 up, carrying the tongue) or closed, the tongue body higher
        // (> 0: F1 down) or lower and further forward (> 0: F2 up) or back, the lips more rounded (> 0: every formant
        // down, F2 most) and wider open, the epilarynx narrowed (a ring around 3 kHz, "twang").
        // `articulate` mode has no plan and no formant steering: its targets are the neutral posture plus these inputs
        // (and tipUp, tipFront), so curves on them place the tract absolutely, articulator by articulator.
        ParamDef {
            key: "larynx",
            kind: ParamKind::Number { min: -1.6, max: 1.6, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "cm", label_en: "Larynx height", label_fr: "Hauteur du larynx",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "jaw",
            kind: ParamKind::Number { min: -1.5, max: 1.5, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "cm", label_en: "Jaw opening", label_fr: "Ouverture de la mâchoire",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "tongueHeight",
            kind: ParamKind::Number { min: -2.0, max: 2.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "cm", label_en: "Tongue height", label_fr: "Hauteur de la langue",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "tongueFront",
            kind: ParamKind::Number { min: -2.0, max: 2.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "cm", label_en: "Tongue front", label_fr: "Avancée de la langue",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "lipRound",
            kind: ParamKind::Number { min: -1.0, max: 1.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "cm", label_en: "Lip rounding", label_fr: "Arrondi des lèvres",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "lipOpen",
            kind: ParamKind::Number { min: -1.5, max: 1.5, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "cm", label_en: "Lip opening", label_fr: "Ouverture des lèvres",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "epilarynx",
            kind: ParamKind::Number { min: 0.0, max: 1.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Epilarynx narrowing (twang)", label_fr: "Resserrement de l'épilarynx (twang)",
            modulatable: true, smooth: true,
        },
        // `voice` and `lyrics` modes, the folds' pulse apart from tension: the open quotient moved from what the
        // tension gives (< 0 a longer closed phase: upper harmonics up against H1; > 0 an H1-heavy pulse), vocal fry
        // (a slow, double-pulsed creak) and roughness (jitter, shimmer, period doubling above 0.5; 0.1 is the model's
        // healthy voice). Then the tract's soft walls' damping (1 = as built; < 1 narrower, stronger formants) and the
        // breath noise of the glottal jet (1 = as built, 0 none).
        ParamDef {
            key: "openQuotient",
            kind: ParamKind::Number { min: -0.4, max: 0.4, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Open quotient shift", label_fr: "Décalage du quotient d'ouverture",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "fry",
            kind: ParamKind::Number { min: 0.0, max: 1.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Vocal fry", label_fr: "Voix craquée (fry)",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "roughness",
            kind: ParamKind::Number { min: 0.0, max: 1.0, default: 0.1, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Roughness", label_fr: "Raucité",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "walls",
            kind: ParamKind::Number { min: 0.1, max: 6.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "×", label_en: "Wall damping", label_fr: "Amortissement des parois",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "aspiration",
            kind: ParamKind::Number { min: 0.0, max: 8.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "×", label_en: "Aspiration noise", label_fr: "Bruit d'aspiration",
            modulatable: true, smooth: true,
        },
        // `voice` and `lyrics` modes: the soft palate lowered over every sound (cm² of velopharyngeal port added to
        // the plan's: the nose coupled to the vowels, a low pole and a zero near F1), how abruptly the folds close (the
        // source's tilt) and the opening phase's share of the pulse apart from brightness, the tongue root advanced
        // (> 0, a wider pharynx: F1 down) or retracted, the lips spread (> 0) wider. 0 = none, as before.
        ParamDef {
            key: "velum",
            kind: ParamKind::Number { min: 0.0, max: 1.4, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "cm²", label_en: "Soft palate lowered", label_fr: "Voile du palais abaissé",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "closureSharpness",
            kind: ParamKind::Number { min: -1.0, max: 1.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Glottal closure sharpness", label_fr: "Netteté de la fermeture glottique",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "skew",
            kind: ParamKind::Number { min: -0.3, max: 0.3, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Pulse skew", label_fr: "Asymétrie de l'impulsion",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "tongueRoot",
            kind: ParamKind::Number { min: -2.0, max: 2.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Tongue root advance", label_fr: "Avancée de la racine de la langue",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "lipSpread",
            kind: ParamKind::Number { min: -1.0, max: 1.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "", label_en: "Lip spreading", label_fr: "Étirement des lèvres",
            modulatable: true, smooth: true,
        },
        // `voice` and `lyrics` modes: the posterior gap between the arytenoids through the closed phase, added to the
        // one breath gives (air leaking all cycle long: the tract damped, H1 lifted, no extra noise). 0 = as before.
        ParamDef {
            key: "glottalGap",
            kind: ParamKind::Number { min: 0.0, max: 0.08, default: 0.0, step: 0.001, scale: Scale::Lin },
            unit: "cm²", label_en: "Glottal gap", label_fr: "Fente glottique",
            modulatable: true, smooth: true,
        },
        // `voice` and `lyrics` modes, the body's side chambers and walls (1 = as built): the soft walls' mass, the
        // maxillary sinuses' opening to the nose (0 sealed: their pole and zero near the nasal resonances), the
        // piriform fossae's size (the dip above 4 kHz, the upper formants' place).
        ParamDef {
            key: "wallMass",
            kind: ParamKind::Number { min: 0.3, max: 3.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "×", label_en: "Wall mass", label_fr: "Masse des parois",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "sinus",
            kind: ParamKind::Number { min: 0.0, max: 4.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "×", label_en: "Sinus opening", label_fr: "Ouverture des sinus",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "fossa",
            kind: ParamKind::Number { min: 0.2, max: 3.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "×", label_en: "Piriform fossae size", label_fr: "Taille des sinus piriformes",
            modulatable: true, smooth: true,
        },
        // `voice`, `lyrics` and `articulate` modes: the tongue tip raised (> 0, toward the alveolar ridge) or lowered
        // from where it rests behind the lower incisors (it follows the jaw and the tongue body), and moved forward
        // (> 0) or back, added to its targets like the articulator inputs above (0 = none, as before).
        ParamDef {
            key: "tipUp",
            kind: ParamKind::Number { min: -2.0, max: 4.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "cm", label_en: "Tongue tip up", label_fr: "Pointe de la langue relevée",
            modulatable: true, smooth: true,
        },
        ParamDef {
            key: "tipFront",
            kind: ParamKind::Number { min: -2.0, max: 2.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "cm", label_en: "Tongue tip front", label_fr: "Pointe de la langue avancée",
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

/// The language a `lyrics` text is written in: English words (CMUdict + letter-to-sound), IPA words, or French words
/// (the French spelling rules of `chimera::phonetics::french`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LyricLanguage {
    #[default]
    English,
    Ipa,
    French,
}

impl LyricLanguage {
    /// From the node's `language` value or a request's name ("english", "ipa", "french"); anything else is English.
    pub fn from_name(name: &str) -> LyricLanguage {
        match name {
            "ipa" => LyricLanguage::Ipa,
            "french" => LyricLanguage::French,
            _ => LyricLanguage::English,
        }
    }

    fn from_index(i: usize) -> LyricLanguage {
        match i {
            IPA => LyricLanguage::Ipa,
            FRENCH => LyricLanguage::French,
            _ => LyricLanguage::English,
        }
    }
}

/// Plans `lyrics` text (see [`lyric_lines`]): per line, English or French G2P (or IPA words) into the physical voice's phoneme
/// gestures at rate 1, its pitch and vibrato gestures dropped (the pitch input sings the line), sorted by start.
pub fn plan_lyrics(text: &str, language: LyricLanguage, female: f64, child: f64, pitch: f64) -> Vec<LyricLine> {
    ::chimera::init();
    let body = BodyMix::new(female.clamp(0.0, 1.0), child.clamp(0.0, 1.0));
    let mut lines = Vec::new();
    let mut next_start = LYRIC_PAUSE;
    for (start, words) in lyric_lines(text) {
        // the segments, and the words they say (each word's last segment marks its end)
        let (segments, said): (Vec<Segment>, Vec<String>) = if language == LyricLanguage::French {
            ::chimera::phonetics::french::to_segments(words)
        } else if language == LyricLanguage::Ipa {
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

/// Words as the `lyrics` mode's G2P writes them (English, or French with [`LyricLanguage::French`]): each phone (IPA
/// before the planner splits a diphthong) with the index of its word, and the words.
pub fn lyric_g2p(words: &str, language: LyricLanguage) -> (Vec<(String, usize)>, Vec<String>) {
    ::chimera::init();
    let mut phones = Vec::new();
    let mut said = Vec::new();
    let mut wi = 0;
    if language == LyricLanguage::French {
        let (segs, said) = ::chimera::phonetics::french::to_segments(words);
        for seg in segs {
            // a nasal vowel keeps its tilde: the recogniser hears ɑ̃ ɔ̃ ɛ̃ œ̃ as phones of their own
            let ipa = if seg.nasalized == Some(true) { format!("{}\u{303}", seg.ipa) } else { seg.ipa.clone() };
            phones.push((ipa, wi));
            if seg.word_end == Some(true) {
                wi += 1;
            }
        }
        return (phones, said);
    }
    for p in english_g2p().to_phrases(words) {
        said.extend(p.words.iter().cloned());
        for seg in p.segments {
            if seg.ipa != "_" {
                phones.push((seg.ipa.clone(), wi));
            }
            if seg.word_end == Some(true) {
                wi += 1;
            }
        }
    }
    (phones, said)
}

/// Whether the voice says a piece of IPA as written: it is read as at least one sound, and every symbol has one.
pub fn ipa_sayable(ipa: &str) -> bool {
    tokenize_ipa(ipa).iter().any(|s| s.ipa != "_") && skipped_ipa(ipa).is_empty()
}

/// IPA (as written in a `lyrics` text) as the sounds the planner times: tokenized, diphthongs split.
fn planned_sounds(ipa: &str) -> Vec<String> {
    tokenize_ipa(ipa)
        .into_iter()
        .filter(|s| s.ipa != "_")
        .flat_map(|s| match expand(&s.ipa) {
            Some(parts) => parts.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            None => vec![s.ipa],
        })
        .collect()
}

/// A `lyrics` line's start and rate input fitted to when its sounds are heard ([`lyric_rate_fit`]).
pub struct LyricRateFit {
    /// patch seconds of the line's start (its text time)
    pub start: f64,
    /// [seconds, rate] points over the line
    pub rate: Vec<[f64; 2]>,
    /// where each planned sound should start (patch s), in plan order
    pub sounds: Vec<f64>,
}

/// The rate input that makes a `lyrics` line say its sounds when the audio does.
///
/// `targets`: the line's phones in order as written in its text (a diphthong one phone) with the patch second each
/// starts; `end`: when the last one ends. Each planned sound is matched with a target (edit alignment of the sounds;
/// a diphthong's parts share its time by their planned lengths; a sound with no target goes between its neighbours
/// at the planned pace). Each sound then gets the rate that fills its time ([`lyric_speed`]): a held sound (vowel,
/// nasal, lateral) stretches or shrinks; another sound is never slower than planned, so the time it cannot fill goes
/// to the held sound before it (the consonant starts later, the next sound on time), or, before the line's first held
/// sound, delays the line's start. A rate stays within 0.1-4.
///
/// Returns the line's start (patch s, to 0.01 s as the text writes it), the rate input over the line as [seconds,
/// rate] steps, at most `max_points` points (the steps closest in rate merged first), and where each planned sound
/// should start.
pub fn lyric_rate_fit(line: &LyricLine, targets: &[(String, f64)], end: f64, max_points: usize) -> LyricRateFit {
    let n = line.spans.len();
    if n == 0 || targets.is_empty() {
        return LyricRateFit { start: line.start, rate: Vec::new(), sounds: Vec::new() };
    }
    let d: Vec<f64> = (0..n).map(|k| (if k + 1 < n { line.spans[k + 1].0 } else { line.spans[k].1 }) - line.spans[k].0).map(|x| x.max(1e-3)).collect();
    let held: Vec<bool> = line.spans.iter().map(|s| s.2).collect();
    // the targets' sounds: (sound, target index)
    let parts: Vec<(String, usize)> = targets.iter().enumerate().flat_map(|(i, (ipa, _))| planned_sounds(ipa).into_iter().map(move |p| (p, i))).collect();
    // edit alignment, planned sounds (rows) against the targets' sounds (columns)
    let (m, w) = (parts.len(), parts.len() + 1);
    let mut c = vec![0usize; (n + 1) * w];
    for i in 0..=n {
        c[i * w] = i;
    }
    for j in 0..=m {
        c[j] = j;
    }
    let cost = |i: usize, j: usize| usize::from(line.phones[i] != parts[j].0);
    for i in 1..=n {
        for j in 1..=m {
            c[i * w + j] = (c[(i - 1) * w + j - 1] + cost(i - 1, j - 1)).min(c[(i - 1) * w + j] + 1).min(c[i * w + j - 1] + 1);
        }
    }
    let mut target_of = vec![usize::MAX; n];
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if c[i * w + j] == c[(i - 1) * w + j - 1] + cost(i - 1, j - 1) {
            target_of[i - 1] = parts[j - 1].1;
            i -= 1;
            j -= 1;
        } else if c[i * w + j] == c[(i - 1) * w + j] + 1 {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    // each matched sound's start: its target's start, the later parts of one target by their planned lengths
    let mut start = vec![f64::NAN; n];
    let mut k = 0;
    while k < n {
        let ti = target_of[k];
        if ti == usize::MAX {
            k += 1;
            continue;
        }
        let mut e = k + 1;
        while e < n && target_of[e] == ti {
            e += 1;
        }
        let t0 = targets[ti].1;
        let t1 = targets.get(ti + 1).map_or(end, |t| t.1).max(t0);
        let total: f64 = d[k..e].iter().sum();
        let mut acc = 0.0;
        for q in k..e {
            start[q] = t0 + (t1 - t0) * acc / total;
            acc += d[q];
        }
        k = e;
    }
    // the rest between their neighbours at the planned pace
    let known: Vec<usize> = (0..n).filter(|&k| start[k].is_finite()).collect();
    if known.is_empty() {
        return LyricRateFit { start: line.start, rate: Vec::new(), sounds: Vec::new() };
    }
    let score = |k: usize| line.spans[k].0;
    for k in 0..n {
        if start[k].is_finite() {
            continue;
        }
        let a = known.iter().rev().find(|&&q| q < k).copied();
        let b = known.iter().find(|&&q| q > k).copied();
        start[k] = match (a, b) {
            (Some(a), Some(b)) => start[a] + (start[b] - start[a]) * (score(k) - score(a)) / (score(b) - score(a)).max(1e-9),
            (Some(a), None) => start[a] + score(k) - score(a),
            (None, Some(b)) => start[b] - (score(b) - score(k)),
            (None, None) => line.start,
        };
    }
    // in order, each at least as long as a rate of 4 plays it
    for k in 1..n {
        start[k] = start[k].max(start[k - 1] + d[k - 1] / 4.0);
    }
    let sounds = start.clone();
    // a run of other sounds (a rate cannot stretch them) with more time than planned plays at its planned pace,
    // placed where its sounds and the held sound after it start closest to the audio (the median of their wanted
    // shifts): the held sound before it ends there, the one after starts there
    let mut a = 0;
    while a < n {
        if held[a] {
            a += 1;
            continue;
        }
        let mut b = a;
        while b < n && !held[b] {
            b += 1;
        }
        let span: f64 = d[a..b].iter().sum();
        if (if b < n { start[b] } else { end }) - start[a] > span {
            let mut shifts = Vec::new();
            let mut off = 0.0;
            for j in a..b {
                shifts.push(start[j] - off);
                off += d[j];
            }
            if b < n {
                shifts.push(start[b] - span);
            }
            shifts.sort_by(f64::total_cmp);
            let h = shifts.len();
            let mut c = if h % 2 == 1 { shifts[h / 2] } else { 0.5 * (shifts[h / 2 - 1] + shifts[h / 2]) };
            // the held sounds on each side keep a quarter of their planned length
            if a > 0 {
                c = c.max(start[a - 1] + d[a - 1] / 4.0);
            }
            if b < n {
                c = c.min(if b + 1 < n { start[b + 1] } else { end } - d[b] / 4.0 - span);
            }
            let mut off = 0.0;
            for j in a..b {
                start[j] = c + off;
                off += d[j];
            }
            if b < n {
                start[b] = c + span;
            }
        }
        a = b;
    }
    // each sound's time (never shorter than a rate of 4 allows), then the time a consonant still cannot fill moved
    // back (a line's last sound simply ends early)
    let mut dur: Vec<f64> = (0..n).map(|k| if k + 1 < n { start[k + 1] - start[k] } else { end - start[k] }).collect();
    let mut first = start[0];
    for k in 0..n {
        dur[k] = dur[k].max(d[k] / 4.0);
        if !held[k] && dur[k] > d[k] {
            let excess = dur[k] - d[k];
            dur[k] = d[k];
            if k + 1 == n {
                continue;
            }
            // the nearest held sound before it (only consonants between)
            match (0..k).rev().find(|&q| held[q]) {
                Some(q) => dur[q] += excess,
                None => first += excess,
            }
        }
    }
    // the text writes the start to 0.01 s: the first held sound takes up what the rounding moved
    let line_start = (first * 100.0).floor() / 100.0;
    if let Some(q) = (0..n).find(|&q| held[q]) {
        if (0..q).all(|r| !held[r]) {
            dur[q] += first - line_start;
        }
    }
    // steps: (start, length, rate, planned length)
    let mut steps: Vec<(f64, f64, f64, f64)> = Vec::new();
    let mut t = line_start;
    let mut prev_r = 1.0;
    for k in 0..n {
        let need = d[k] / dur[k];
        let r = if held[k] {
            need.clamp(0.1, 4.0)
        } else if need > 1.0 + 1e-9 {
            need.min(4.0)
        } else if prev_r <= 1.0 {
            // a consonant at its planned pace: any rate up to 1 plays it so; the previous one saves a step
            prev_r
        } else {
            1.0
        };
        match steps.last_mut() {
            Some(s) if (s.2 - r).abs() < 1e-6 => {
                s.1 += dur[k];
                s.3 += d[k];
            }
            _ => steps.push((t, dur[k], r, d[k])),
        }
        t += dur[k];
        prev_r = r;
    }
    // at most max_points: merge the two neighbouring steps closest in rate
    let cap = max_points.max(4);
    while 2 * steps.len() + 2 > cap && steps.len() > 1 {
        let q = (0..steps.len() - 1).min_by(|&a, &b| (steps[a].2.ln() - steps[a + 1].2.ln()).abs().total_cmp(&(steps[b].2.ln() - steps[b + 1].2.ln()).abs())).unwrap_or(0);
        let (a, b) = (steps[q], steps[q + 1]);
        steps[q] = (a.0, a.1 + b.1, ((a.3 + b.3) / (a.1 + b.1)).clamp(0.1, 4.0), a.3 + b.3);
        steps.remove(q + 1);
    }
    // the breath lead at the planned pace, then each step held to just before the next
    let mut pts = vec![[line_start - LYRIC_LEAD, 1.0], [line_start - 0.002, 1.0]];
    for (s0, len, r, _) in &steps {
        let eps = (len * 0.25).min(0.002);
        pts.push([*s0, *r]);
        pts.push([s0 + len - eps, *r]);
    }
    LyricRateFit { start: line_start, rate: pts, sounds }
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
                        else if language == FRENCH { ::chimera::phonetics::french::to_segments(word).0 }
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
                } else if language == FRENCH {
                    let segments = ::chimera::phonetics::french::to_segments(text).0;
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
                let lines = plan_lyrics(text, LyricLanguage::from_index(language), female, child, pitch);
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
            } else if mode == VOICE || mode == ARTICULATE {
                // A sustained Chimera vowel, no score: the pitch/gain curves play it like an
                // instrument, but the sound is the engine's voice (tract + folds), not a flute.
                // `articulate`: the neutral posture, which the articulator inputs move (no text vowel).
                let vowel = if text.is_empty() { "a" } else { text };
                let body = BodyMix::new(female, child);
                if mode == ARTICULATE {
                    let mut neutral = ArticPatch::default();
                    for k in ArticKey::ALL {
                        neutral.set(k, NEUTRAL.get(k));
                    }
                    self.engine.set_artic(&neutral, Some(true));
                    self.text_vowel = neutral;
                } else if let Some(artic) = vowel_artic(vowel, body) {
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
            let freeform = self.mode == VOICE || self.mode == ARTICULATE;
            if freeform || self.mode == LYRICS {
                // under ~1/3 voicing the folds part (voiceless abduction: aspiration); above, they vibrate with an
                // amplitude that follows the voicing, so a voiced sound fades into a voiceless one (voice and
                // articulate modes; in lyrics mode the planned consonants part the folds)
                if freeform {
                    patch.voiced = Some(voicing >= 0.35);
                }
                patch.brightness = Some(at(BRIGHTNESS, 0.25).clamp(0.0, 1.0));
                patch.vibrato_depth = Some(at(VIBRATO_DEPTH, 0.0).clamp(0.0, 200.0));
                patch.vibrato_rate = Some(at(VIBRATO_RATE, 5.5).clamp(2.0, 10.0));
                patch.glide = Some(at(GLIDE, 0.06).clamp(0.002, 0.3));
                self.engine.voice_mut().airway.length_scale = at(TRACT, 1.0).clamp(0.7, 1.4);
                self.engine.voice_mut().airway.nasal_leak = at(NASAL, 0.0).clamp(0.0, 0.6);
                patch.oq_offset = Some(at(OPEN_QUOTIENT, 0.0).clamp(-0.4, 0.4));
                patch.fry = Some(at(FRY, 0.0).clamp(0.0, 1.0));
                patch.roughness = Some(at(ROUGHNESS, 0.1).clamp(0.0, 1.0));
                patch.sharpness_offset = Some(at(CLOSURE_SHARPNESS, 0.0).clamp(-1.0, 1.0));
                patch.skew_offset = Some(at(SKEW, 0.0).clamp(-0.3, 0.3));
                patch.gap_offset = Some(at(GLOTTAL_GAP, 0.0).clamp(0.0, 0.08));
                let voice = self.engine.voice_mut();
                voice.airway.wall_loss = at(WALLS, 1.0).clamp(0.1, 6.0);
                voice.airway.aspiration = at(ASPIRATION, 1.0).clamp(0.0, 8.0);
                voice.airway.wall_mass = at(WALL_MASS, 1.0).clamp(0.3, 3.0);
                voice.airway.sinus_coupling = at(SINUS, 1.0).clamp(0.0, 4.0);
                voice.airway.fossa_area = at(FOSSA, 1.0).clamp(0.2, 3.0);
                let off = &mut voice.artic.offset;
                off[ArticKey::Larynx.index()] = at(LARYNX, 0.0).clamp(-1.6, 1.6);
                // the jaw carries the tongue body (the calibrated tongue targets are absolute: a jaw alone barely
                // changes a vowel)
                let jaw = at(JAW, 0.0).clamp(-1.5, 1.5);
                let carry = jaw_carry(jaw);
                off[ArticKey::Jaw.index()] = jaw;
                off[ArticKey::TbY.index()] = at(TONGUE_HEIGHT, 0.0).clamp(-2.0, 2.0) + carry.y;
                off[ArticKey::TbX.index()] = at(TONGUE_FRONT, 0.0).clamp(-2.0, 2.0) + carry.x;
                off[ArticKey::LipProt.index()] = at(LIP_ROUND, 0.0).clamp(-1.0, 1.0);
                off[ArticKey::LipAp.index()] = at(LIP_OPEN, 0.0).clamp(-1.5, 1.5);
                off[ArticKey::Epilarynx.index()] = at(EPILARYNX, 0.0).clamp(0.0, 1.0);
                off[ArticKey::Velum.index()] = at(VELUM, 0.0).clamp(0.0, 1.4);
                off[ArticKey::Root.index()] = at(TONGUE_ROOT, 0.0).clamp(-2.0, 2.0);
                off[ArticKey::LipWidth.index()] = at(LIP_WIDTH, 0.0).clamp(-1.0, 1.0);
                off[ArticKey::TtY.index()] = at(TIP_UP, 0.0).clamp(-2.0, 4.0);
                off[ArticKey::TtX.index()] = at(TIP_FRONT, 0.0).clamp(-2.0, 2.0);
            }
            self.engine.set_source(&patch);
            if freeform {
                self.engine.voice_mut().glottis.params.amplitude *= voicing;
                // articulate: no formant target, the neutral posture under the articulator inputs
                let formants = self.mode == VOICE;
                let f = |i: usize| if formants { at(i, 0.0) } else { 0.0 };
                self.steer(
                    f(VOWEL_F1),
                    f(VOWEL_F2),
                    f(VOWEL_F3),
                    at(FRICATION, 0.0),
                    at(FRIC_PLACE, 1.0),
                    at(CLOSURE, 0.0),
                );
            }
            self.engine.render(&mut self.buffer);
            if freeform || self.mode == LYRICS {
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
    fn voice_habit_inputs_change_nothing_at_their_defaults_and_move_f1_and_h1_h2() {
        // a held /e/ at 130 Hz in voice mode: the new inputs at their defaults render the same samples as a patch
        // written before them; a closer jaw (carrying the tongue) moves F1's band (400-650 Hz) down under 330 Hz; a
        // shorter open phase (open quotient -0.25) lifts H2 against H1
        let mut params = [0.0; 52];
        params[..17].copy_from_slice(&[2.0, 0.0, 130.0, 9.0, 1.0, 0.0, 0.0, 100.0, 1.0, 0.05, 0.5, 0.0, 0.0, 0.14, 0.0, 1.0, 1.0]);
        params[VOICING] = 1.0;
        params[FRIC_PLACE] = 1.0;
        params[BRIGHTNESS] = 0.25;
        params[VIBRATO_RATE] = 5.5;
        params[GLIDE] = 0.06;
        params[TRACT] = 1.0;
        let mut texts: Vec<Option<String>> = vec![None; 52];
        texts[TEXT] = Some("e".into());
        let harmonics = |y: &[f64]| -> Vec<f64> {
            let y = &y[24_000..];
            (1..16)
                .map(|h| {
                    let f = 130.0 * h as f64;
                    let (mut re, mut im) = (0.0, 0.0);
                    for (i, x) in y.iter().enumerate() {
                        let ph = core::f64::consts::TAU * f * i as f64 / 48_000.0;
                        re += x * ph.cos();
                        im += x * ph.sin();
                    }
                    re * re + im * im
                })
                .collect()
        };
        let share = |e: &[f64], lo: f64, hi: f64| -> f64 {
            let band: f64 = e.iter().enumerate().filter(|(h, _)| (lo..hi).contains(&(130.0 * (*h + 1) as f64))).map(|(_, p)| p).sum();
            10.0 * (band / e.iter().sum::<f64>()).log10()
        };
        let old = run(&init(&params[..31], &texts[..31]), 48_000);
        params[ROUGHNESS] = 0.1;
        params[WALLS] = 1.0;
        params[ASPIRATION] = 1.0;
        params[WALL_MASS] = 1.0;
        params[SINUS] = 1.0;
        params[FOSSA] = 1.0;
        let base = run(&init(&params, &texts), 48_000);
        assert_eq!(old, base, "the new inputs at their defaults changed the render");
        params[JAW] = -0.8;
        let high = run(&init(&params, &texts), 48_000);
        params[JAW] = 0.0;
        params[OPEN_QUOTIENT] = -0.25;
        let pressed = run(&init(&params, &texts), 48_000);
        assert!(high.iter().chain(&pressed).all(|x| x.is_finite() && x.abs() <= 1.5));
        let (eb, eh, ep) = (harmonics(&base), harmonics(&high), harmonics(&pressed));
        let (f1_base, f1_high) = (share(&eb, 400.0, 650.0), share(&eh, 400.0, 650.0));
        let (low_base, low_high) = (share(&eb, 200.0, 330.0), share(&eh, 200.0, 330.0));
        assert!(f1_high < f1_base - 2.0 && low_high > low_base + 2.0, "jaw closed: 400-650 Hz {f1_base:.1} -> {f1_high:.1} dB, 200-330 Hz {low_base:.1} -> {low_high:.1} dB");
        let h21 = |e: &[f64]| 10.0 * (e[1] / e[0]).log10();
        assert!(h21(&ep) > h21(&eb) + 2.0, "open quotient -0.25: H2-H1 {:.1} -> {:.1} dB", h21(&eb), h21(&ep));
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

    #[test]
    fn french_lyrics_plan_the_french_phones() {
        // "Je ne vendrai pas mon âme." through the French G2P: nasal vowels, the uvular r, silent finals
        let (phones, said) = lyric_g2p("Je ne vendrai pas mon âme", LyricLanguage::French);
        assert_eq!(said, vec!["je", "ne", "vendrai", "pas", "mon", "âme"]);
        let ipa: Vec<&str> = phones.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(ipa, vec!["ʒ", "ə", "n", "ə", "v", "ɑ̃", "d", "ʁ", "ɛ", "p", "a", "m", "ɔ̃", "a", "m"]);
        let lines = plan_lyrics("0.5 Je ne vendrai pas mon âme", LyricLanguage::French, 0.0, 0.0, 120.0);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].phones.iter().any(|p| p == "ʁ"), "{:?}", lines[0].phones);
        assert_eq!(LyricLanguage::from_name("french"), LyricLanguage::French);
    }

    #[test]
    fn lyric_rate_fit_lands_each_sound_when_the_audio_says_it() {
        // "aɪ hæv" heard later and slower than planned: aɪ at 1.0, h at 1.3, æ at 1.4, v at 1.7, ending at 1.9
        let lines = plan_lyrics("0.5 aɪ hæv", LyricLanguage::Ipa, 0.0, 0.0, 120.0);
        let targets = [("aɪ", 1.0), ("h", 1.3), ("æ", 1.4), ("v", 1.7)].map(|(p, t)| (p.to_string(), t));
        let fit = lyric_rate_fit(&lines[0], &targets, 1.9, 120);
        assert!((fit.start - 1.0).abs() < 1e-9, "start {}", fit.start);
        assert!(fit.rate.iter().all(|p| (0.1..=4.0).contains(&p[1])) && fit.rate.windows(2).all(|w| w[1][0] >= w[0][0]));
        // played by the node's clock: the consonant run h (planned shorter than its time) sits between its own time and
        // the vowel after it, every sound within 15 ms of the audio
        let rate_at = |t: f64| {
            let i = fit.rate.partition_point(|p| p[0] <= t);
            if i == 0 || i == fit.rate.len() {
                return fit.rate[i.min(fit.rate.len() - 1)][1];
            }
            let (a, b) = (fit.rate[i - 1], fit.rate[i]);
            a[1] + (b[1] - a[1]) * (t - a[0]) / (b[0] - a[0]).max(1e-12)
        };
        let mut moved = plan_lyrics("0.5 aɪ hæv", LyricLanguage::Ipa, 0.0, 0.0, 120.0);
        moved[0].start = fit.start;
        let timed = lyrics_phone_timing(&moved, rate_at);
        let starts: Vec<(&str, f64)> = timed[0].iter().map(|(p, a, _)| (p.as_str(), *a)).collect();
        assert_eq!(starts.iter().map(|s| s.0).collect::<Vec<_>>(), vec!["a", "ɪ", "h", "æ", "v"]);
        for (k, want) in [(0, 1.0), (2, 1.3), (3, 1.4), (4, 1.7)] {
            assert!((starts[k].1 - want).abs() < 0.015, "{} at {:.3}, heard at {want}", starts[k].0, starts[k].1);
        }
        // the G2P side: words and phones as the lyrics mode writes them
        let (phones, said) = lyric_g2p("I have", LyricLanguage::English);
        assert_eq!(said, vec!["i", "have"]);
        assert_eq!(phones.iter().map(|(p, w)| (p.as_str(), *w)).collect::<Vec<_>>(), vec![("aɪ", 0), ("h", 1), ("ə", 1), ("v", 1)]);
        assert!(ipa_sayable("ɑː") && !ipa_sayable("5") && !ipa_sayable(""));
    }

    #[test]
    fn articulate_mode_follows_the_tongue_tip_and_tip_defaults_preserve_lyrics() {
        let defaults: Vec<f64> = DEF.params[..TIP_UP]
            .iter()
            .map(|p| match p.kind {
                crate::catalog::ParamKind::Number { default, .. } => default,
                crate::catalog::ParamKind::Enum { default, .. } => default as f64,
                crate::catalog::ParamKind::Bool { default } => f64::from(u8::from(default)),
                crate::catalog::ParamKind::Text { .. } => 0.0,
            })
            .collect();
        // a lyrics patch written before tipUp / tipFront existed plays the same as one that sets them to 0
        let mut old = defaults.clone();
        old[MODE] = LYRICS as f64;
        let mut texts: Vec<Option<String>> = vec![None; TIP_FRONT + 1];
        texts[TEXT] = Some("0.1 i have changed".into());
        let mut explicit = old.clone();
        explicit.extend([0.0, 0.0]);
        let lyrics = run(&init(&old, &texts[..TIP_UP]), 48_000);
        assert!(rms(&lyrics) > 0.005);
        assert_eq!(lyrics, run(&init(&explicit, &texts), 48_000));
        // articulate mode holds the neutral posture plus its inputs: raising the tip to the ridge changes the sound
        let mut held = defaults;
        held[MODE] = ARTICULATE as f64;
        held[PITCH] = 130.0;
        held[PRESSURE] = 9.0;
        held.extend([0.0, 0.0]);
        let neutral = run(&init(&held, &texts), 24_000);
        held[TIP_UP] = 4.0;
        let tip = run(&init(&held, &texts), 24_000);
        assert!(neutral.iter().chain(tip.iter()).all(|x| x.is_finite() && x.abs() <= 1.5));
        assert!(rms(&neutral[12_000..]) > 0.005, "neutral rms {}", rms(&neutral[12_000..]));
        let diff: Vec<f64> = neutral.iter().zip(&tip).map(|(a, b)| a - b).collect();
        assert!(rms(&diff[12_000..]) > 0.1 * rms(&neutral[12_000..]), "tip up changed the sound by {}", rms(&diff[12_000..]));
    }
}
