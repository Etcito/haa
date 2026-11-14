//! `pluck` source node: a plucked string, extended Karplus-Strong digital waveguide.
//! HAA-only node (no Sona counterpart). Gate mode: one string played by `frequency` + `gate`.
//! Score mode (`score` holds notes): the node is a set of `voices` strings that play the score
//! itself, chords and overlapping notes included, so a whole guitar or bass part is one node.
//!
//! Loop: integer delay line -> one-pole loss filter -> optional stiffness allpass chain
//! (inharmonicity) -> first-order allpass fine tuning -> loop gain. The loss filter is solved from
//! two decay times: `decay` (T60) at the fundamental and T60 x 0.5 x 0.02^`damping` at
//! `dampingFreq` (at least 1.5 x the fundamental), so the partials of a low string
//! die progressively faster with frequency, as on a real string (a 3-tap FIR runs out of slope on a
//! bass: its loss grows with the square of frequency, and a 55 Hz loop turns only 55 times per
//! second). The tuning solves the exact phase delay of every loop element at the fundamental, so
//! the pitch holds from bass E1 (41 Hz) to the top of the guitar neck.
//!
//! Excitation (on each pluck): one period of a blend of a soft triangular displacement and a noise
//! burst (`pickHardness`), comb-filtered at `pickPosition`, low-passed by `brightness` and
//! velocity, then injected into the loop. Re-plucking a ringing string chokes the old vibration.
//!
//! Two polarizations: the string vibrates parallel and perpendicular to the top. The second loop
//! (`polarization` of the excitation) is detuned by `detune` cents and loses less energy at the
//! bridge (longer T60, less high damping), so a note beats and decays in two stages: a fast
//! initial drop then a long tail, as a real string does.
//!
//! Playing: `decayTilt` shortens the sustain of high notes (T60 x (110 Hz / f)^tilt); `mute` is a
//! palm mute (much shorter, darker sustain and a duller attack); `attackBend` starts a hard pluck
//! sharp and lets it settle (tension modulation, up to 40 cents x velocity^2, ~80 ms); vibrato
//! (`vibratoRate`, `vibratoDepth` in cents) and pitch curves from `notes`/`glide` retune the loop
//! every 16 samples; `velocityBrightness` is how much velocity opens the attack tone; `pickNoise`
//! adds the pick's (or the finger's) click, high-passed noise outside the loop decaying in 3 ms for
//! a hard pick and up to 23 ms for a soft finger.
//!
//! Output: `pickup` none (bridge signal, acoustic) or a magnetic pickup: a comb at
//! `pickupPosition` (one coil for `single`, two coils 3% of the scale apart for `humbucker`) and
//! the coil's resonant low-pass. `body` mixes in acoustic-guitar body modes (air ~100 Hz, top plate
//! ~200/400 Hz, upper bands), scaled in frequency by 1 / `bodySize`. Pitch slides and hammer-ons
//! compose with `notes` + `glide`.
//!
//! Gate: a rising edge of `gate` above 0.01 plucks with velocity x gate level; when that gate
//! falls back the string is damped with `release` (a fretting finger lifting). `autoStrike`
//! plucks once on Play and on each loop, like `membrane`.
//!
//! Score: `[{"start","duration","midi","level"}]` (the `notes` node's format, overlaps allowed).
//! Each note plucks the free string nearest in pitch (or steals the oldest) with velocity x level
//! and is released at its end; `frequency`, `gate` and `autoStrike` are then unused. A string under
//! -120 dB with nothing to play is skipped. A note may carry its own timbre around the node's:
//! `brightness` is added to the node's `brightness` for that note (-1..1, default 0) and `decay`
//! multiplies the node's `decay` (0.05..20, default 1), so measured per-note tone and sustain ride
//! on the values a fit tunes for the whole node.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::biquad::{Biquad, BiquadKind};
use crate::dsp::dc_blocker::DcBlocker;
use crate::dsp::delay_line::DelayLine;
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;
use core::f64::consts::TAU;

const FREQUENCY: usize = 0;
const GATE: usize = 1;
const VELOCITY: usize = 2;
const DECAY: usize = 3;
const RELEASE: usize = 4;
const DAMPING: usize = 5;
const BRIGHTNESS: usize = 6;
const PICK_POSITION: usize = 7;
const PICK_HARDNESS: usize = 8;
const STIFFNESS: usize = 9;
const BODY: usize = 10;
const PICKUP: usize = 11;
const PICKUP_POSITION: usize = 12;
const GAIN: usize = 13;
const AUTO_STRIKE: usize = 14;
const POLARIZATION: usize = 15;
const DETUNE: usize = 16;
const MUTE: usize = 17;
const PICK_NOISE: usize = 18;
const ATTACK_BEND: usize = 19;
const VIBRATO_RATE: usize = 20;
const VIBRATO_DEPTH: usize = 21;
const DECAY_TILT: usize = 22;
const VELOCITY_BRIGHTNESS: usize = 23;
const BODY_SIZE: usize = 24;
const SCORE: usize = 25;
const VOICES: usize = 26;
const DAMPING_FREQ: usize = 27;
/// Strings of a score-mode node.
const MAX_VOICES: usize = 8;
const MAX_SCORE_NOTES: usize = 4096;
/// Idle detection: per-sample decay of the running level and the silent level.
const IDLE_DECAY: f64 = 0.9995;
const IDLE_LEVEL: f64 = 1e-6;

const MIN_HZ: f64 = 20.0;
const MAX_HZ: f64 = 4_000.0;
/// Gate level above which a rising edge plucks.
const GATE_ON: f64 = 0.01;
/// Gain applied once to the old vibration when a ringing string is plucked again.
const CHOKE: f64 = 0.3;
/// Number of stiffness allpass sections.
const DISPERSION_STAGES: usize = 4;
/// Humbucker coil spacing as a fraction of the scale (about 18 mm on a 650 mm scale).
const HUMBUCKER_SPACING: f64 = 0.03;
/// Output calibration: a full-velocity default pluck peaks near 0.5.
const OUT_SCALE: f64 = 0.55;
/// Acoustic body modes: frequency (Hz), Q, weight.
const BODY_MODES: [(f64, f64, f64); 6] = [
    (98.0, 16.0, 1.0),
    (196.0, 14.0, 1.1),
    (385.0, 10.0, 0.7),
    (590.0, 8.0, 0.5),
    (1_250.0, 3.5, 0.45),
    (2_900.0, 2.0, 0.35),
];
const BODY_GAIN: f64 = 1.6;
/// Samples between two retunings of a moving pitch (vibrato, attack bend, pitch curves).
const RETUNE_EVERY: u32 = 16;
/// The perpendicular polarization: T60 and high-damping factors.
const POL_T60: f64 = 1.8;
const POL_DAMPING: f64 = 0.8;
/// Attack bend: largest offset (cents at full velocity) and its time constant (s).
const BEND_CENTS: f64 = 40.0;
const BEND_TAU: f64 = 0.08;
/// Pick click: decay time constant (s) of a hard pick and the extra of a soft finger, high-pass
/// corner (Hz); the burst lasts five time constants.
const CLICK_TAU: f64 = 0.003;
const CLICK_SOFT_TAU: f64 = 0.02;
const CLICK_HZ: f64 = 1_000.0;

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
    scale: Scale,
    unit: &'static str,
    label_en: &'static str,
    label_fr: &'static str,
    smooth: bool,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number { min, max, default, step, scale },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth,
    }
}

/// The `pluck` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "pluck",
    category: Category::Source,
    label_en: "Plucked string",
    label_fr: "Corde pincée",
    audio_in: false,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        number("frequency", MIN_HZ, MAX_HZ, 110.0, 0.01, Scale::Log, "Hz", "Frequency", "Fréquence", true),
        number("gate", 0.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Pluck gate (rising edge)", "Attaque (front montant)", false),
        number("velocity", 0.0, 1.0, 0.8, 0.01, Scale::Lin, "", "Velocity", "Vélocité", false),
        number("decay", 0.05, 30.0, 4.0, 0.01, Scale::Log, "s", "Sustain (T60)", "Tenue (T60)", true),
        number("release", 0.01, 5.0, 0.15, 0.01, Scale::Log, "s", "Release (string damped)", "Relâchement (corde étouffée)", true),
        number("damping", 0.0, 1.0, 0.35, 0.01, Scale::Lin, "", "High damping", "Amortissement des aigus", true),
        number("brightness", 0.0, 1.0, 0.6, 0.01, Scale::Lin, "", "Brightness", "Brillance", true),
        number("pickPosition", 0.02, 0.5, 0.15, 0.01, Scale::Lin, "", "Pick position (from bridge)", "Position du médiator (depuis le chevalet)", true),
        number("pickHardness", 0.0, 1.0, 0.5, 0.01, Scale::Lin, "", "Pick hardness", "Dureté du médiator", true),
        number("stiffness", 0.0, 1.0, 0.1, 0.01, Scale::Lin, "", "Stiffness (inharmonicity)", "Raideur (inharmonicité)", true),
        number("body", 0.0, 1.0, 0.5, 0.01, Scale::Lin, "", "Body resonance", "Résonance de caisse", true),
        ParamDef {
            key: "pickup",
            kind: ParamKind::Enum { values: &["none", "single", "humbucker"], default: 0 },
            unit: "",
            label_en: "Pickup",
            label_fr: "Micro",
            modulatable: false,
            smooth: false,
        },
        number("pickupPosition", 0.04, 0.5, 0.2, 0.01, Scale::Lin, "", "Pickup position (from bridge)", "Position du micro (depuis le chevalet)", true),
        number("gain", 0.0, 2.0, 0.8, 0.01, Scale::Lin, "×", "Level", "Niveau", true),
        ParamDef {
            key: "autoStrike",
            kind: ParamKind::Bool { default: true },
            unit: "",
            label_en: "Pluck on Play / loop",
            label_fr: "Attaque au Play / boucle",
            modulatable: false,
            smooth: false,
        },
        number("polarization", 0.0, 1.0, 0.25, 0.01, Scale::Lin, "", "Second polarization", "Seconde polarisation", true),
        number("detune", 0.0, 20.0, 1.5, 0.1, Scale::Lin, "cents", "Polarization detune", "Désaccord des polarisations", true),
        number("mute", 0.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Palm mute", "Étouffé (paume)", true),
        number("pickNoise", 0.0, 1.0, 0.1, 0.01, Scale::Lin, "", "Pick click", "Clic du médiator", true),
        number("attackBend", 0.0, 1.0, 0.1, 0.01, Scale::Lin, "", "Attack pitch bend", "Glissé d'attaque", true),
        number("vibratoRate", 0.5, 12.0, 5.5, 0.01, Scale::Log, "Hz", "Vibrato rate", "Vitesse du vibrato", true),
        number("vibratoDepth", 0.0, 100.0, 0.0, 0.1, Scale::Lin, "cents", "Vibrato depth", "Profondeur du vibrato", true),
        number("decayTilt", 0.0, 1.0, 0.3, 0.01, Scale::Lin, "", "Shorter sustain up the neck", "Tenue plus courte dans l'aigu", true),
        number("velocityBrightness", 0.0, 1.0, 0.45, 0.01, Scale::Lin, "", "Velocity to brightness", "Vélocité vers brillance", true),
        number("bodySize", 0.5, 2.0, 1.0, 0.01, Scale::Log, "×", "Body size", "Taille de caisse", true),
        ParamDef {
            key: "score",
            kind: ParamKind::Text { default: "" },
            unit: "",
            label_en: "Notes (score mode)",
            label_fr: "Notes (mode partition)",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "voices",
            kind: ParamKind::Number { min: 1.0, max: 8.0, default: 6.0, step: 1.0, scale: Scale::Lin },
            unit: "",
            label_en: "Strings (score mode)",
            label_fr: "Cordes (mode partition)",
            modulatable: false,
            smooth: false,
        },
        number("dampingFreq", 150.0, 10_000.0, 4_000.0, 1.0, Scale::Log, "Hz", "Damping reference", "Référence d'amortissement", true),
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let score = parse_score(init.texts.get(SCORE).and_then(|s| s.as_deref()).unwrap_or(""));
    Box::new(Pluck::new(init.sample_rate, init.seed, score))
}

/// First-order allpass `(a + z^-1) / (1 + a z^-1)`.
#[derive(Clone, Copy, Default)]
struct Allpass {
    a: f64,
    x1: f64,
    y1: f64,
}

impl Allpass {
    fn process(&mut self, x: f64) -> f64 {
        let y = self.a * x + self.x1 - self.a * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
    fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }
}

/// Exact phase delay in samples of `Allpass { a }` at `w` radians per sample.
fn allpass_delay(a: f64, w: f64) -> f64 {
    let (s, c) = w.sin_cos();
    let num = (-s).atan2(a + c);
    let den = (-a * s).atan2(1.0 + a * c);
    -(num - den) / w
}

/// Phase delay in samples of the one-pole low-pass `(1 - b) / (1 - b z^-1)` at `w` rad/sample.
fn onepole_delay(b: f64, w: f64) -> f64 {
    (b * w.sin()).atan2(1.0 - b * w.cos()) / w
}

/// Magnitude of the same one-pole low-pass at `w`.
fn onepole_gain(b: f64, w: f64) -> f64 {
    (1.0 - b) / (1.0 - 2.0 * b * w.cos() + b * b).sqrt()
}

/// Loop coefficients for one (frequency, T60, damping, stiffness, damping reference) setting.
#[derive(Clone, Copy)]
struct Tuning {
    key: [f64; 5],
    period: f64,
    delay: usize,
    loss_b: f64,
    loop_gain: f64,
    stiff_a: f64,
    stiff_on: bool,
    tune_a: f64,
}

fn tune(sample_rate: f64, key: [f64; 5]) -> Tuning {
    let [frequency, t60, damping, stiffness, damping_freq] = key;
    let f = frequency.clamp(MIN_HZ, MAX_HZ.min(sample_rate * 0.2));
    let period = sample_rate / f;
    let w0 = TAU * f / sample_rate;

    // Loss: per-period gain at f0 from T60, and at the reference frequency from the T60 shortened
    // by `damping`; the pole is found by bisection (the gain ratio falls as the pole grows), and
    // its delay at f0 may take at most 30% of the period.
    let t60 = t60.max(0.01);
    let g0 = 10f64.powf(-3.0 / (t60 * f));
    let ratio = 0.5 * 0.02f64.powf(damping.clamp(0.0, 1.0));
    let fh = damping_freq.clamp(150.0, 10_000.0).max(1.5 * f).min(0.45 * sample_rate);
    let wh = TAU * fh / sample_rate;
    let mut loss_b = 0.0;
    if fh > 1.01 * f {
        let r = 10f64.powf(-3.0 / (t60 * ratio * f)) / g0;
        let (mut lo, mut hi) = (0.0f64, 0.995f64);
        while onepole_delay(hi, w0) > 0.3 * period && hi > 1e-3 {
            hi *= 0.9;
        }
        if onepole_gain(hi, wh) / onepole_gain(hi, w0) >= r {
            loss_b = hi;
        } else {
            for _ in 0..40 {
                let mid = 0.5 * (lo + hi);
                if onepole_gain(mid, wh) / onepole_gain(mid, w0) > r {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            loss_b = 0.5 * (lo + hi);
        }
    }
    let loss_delay = if loss_b > 0.0 { onepole_delay(loss_b, w0) } else { 0.0 };
    let loop_gain = (g0 / onepole_gain(loss_b, w0)).min(0.99995);

    // Stiffness: negative allpass coefficients delay low frequencies more than high ones, so the
    // upper partials run sharp. The chain may use at most 40% of the period.
    let mut stiff_a = -0.9 * stiffness.clamp(0.0, 1.0);
    let mut stiff_on = stiffness > 1e-3 && period > 24.0;
    let mut stiff_delay = 0.0;
    if stiff_on {
        let mut tries = 0;
        loop {
            stiff_delay = DISPERSION_STAGES as f64 * allpass_delay(stiff_a, w0);
            if stiff_delay <= 0.4 * period || tries >= 40 {
                break;
            }
            stiff_a *= 0.8;
            tries += 1;
        }
        if stiff_delay > 0.4 * period {
            stiff_on = false;
            stiff_delay = 0.0;
        }
    }

    // Integer delay plus a fine-tuning allpass of 0.5..1.5 samples.
    let rest = period - loss_delay - stiff_delay;
    let delay = (rest - 0.5).floor().max(1.0);
    let target = rest - delay;
    let mut d = target.clamp(0.05, 2.5);
    let mut tune_a = (1.0 - d) / (1.0 + d);
    for _ in 0..3 {
        d = (d + target - allpass_delay(tune_a, w0)).clamp(0.05, 2.5);
        tune_a = (1.0 - d) / (1.0 + d);
    }
    Tuning {
        key,
        period,
        delay: delay as usize,
        loss_b,
        loop_gain,
        stiff_a,
        stiff_on,
        tune_a,
    }
}

/// One polarization of the string: delay line, loss filter, stiffness chain and fine tuning.
struct StringLoop {
    line: DelayLine,
    lp: f64,
    stiff: [Allpass; DISPERSION_STAGES],
    fine: Allpass,
    tuning: Tuning,
}

impl StringLoop {
    fn new(sample_rate: f64) -> Self {
        let tuning = tune(sample_rate, [110.0, 4.0, 0.35, 0.1, 4_000.0]);
        let mut s = StringLoop {
            line: DelayLine::new(1.1 / MIN_HZ, sample_rate),
            lp: 0.0,
            stiff: [Allpass::default(); DISPERSION_STAGES],
            fine: Allpass::default(),
            tuning,
        };
        s.apply(tuning);
        s
    }

    fn apply(&mut self, t: Tuning) {
        self.tuning = t;
        for stage in &mut self.stiff {
            stage.a = t.stiff_a;
        }
        self.fine.a = t.tune_a;
    }

    fn retune(&mut self, sample_rate: f64, key: [f64; 5]) {
        if key != self.tuning.key {
            let t = tune(sample_rate, key);
            self.apply(t);
        }
    }

    /// One trip around the loop with `input` injected; `choke` scales the old vibration.
    fn tick(&mut self, input: f64, choke: f64) -> f64 {
        let t = self.tuning;
        let v = self.line.read(t.delay as f64);
        self.lp = (1.0 - t.loss_b) * v + t.loss_b * self.lp;
        let mut y = self.lp;
        if t.stiff_on {
            for stage in &mut self.stiff {
                y = stage.process(y);
            }
        }
        y = self.fine.process(y) * t.loop_gain * choke + input;
        if !y.is_finite() {
            self.reset();
            y = 0.0;
        }
        self.line.write(y);
        y
    }

    /// The magnetic pickup's view of this polarization (before the coil filter).
    fn pickup(&self, kind: usize, y: f64, position: f64) -> f64 {
        let period = self.tuning.period;
        match kind {
            1 => 0.8 * (y - self.line.read(1.0 + position * period)),
            2 => {
                let a = self.line.read(1.0 + position * period);
                let b = self.line.read(1.0 + (position + HUMBUCKER_SPACING).min(0.5) * period);
                0.5 * ((y - a) + (y - b))
            }
            _ => y,
        }
    }

    fn reset(&mut self) {
        self.line.reset();
        self.lp = 0.0;
        for stage in &mut self.stiff {
            stage.reset();
        }
        self.fine.reset();
    }
}

/// One sounding string: two polarizations, its pluck excitation, click and playing state.
struct StringVoice {
    strings: [StringLoop; 2],
    excitation: Vec<f64>,
    exc_len: usize,
    exc_pos: usize,
    /// Share of the excitation sent to the perpendicular polarization, fixed at the pluck.
    exc_split: f64,
    choke_left: usize,
    held: bool,
    released: bool,
    /// Seconds since the pluck (attack bend), the bend's start offset and the retune counter.
    since_pluck: f64,
    bend_amount: f64,
    retune_count: u32,
    /// Pick click: samples left, envelope, level and high-pass state (x1, y1).
    click_left: usize,
    click_env: f64,
    click_level: f64,
    click_decay: f64,
    click_hp: (f64, f64),
    /// Score mode: the note's pitch (Hz, MIDI), its start and end (s).
    hz: f64,
    midi: f64,
    began: f64,
    until: f64,
    /// Running output level; a voice under -120 dB with nothing to play is skipped.
    level: f64,
    active: bool,
    /// The raw inputs of the last retune.
    inputs: [f64; 10],
    /// Score mode: the playing note's brightness offset and decay factor.
    bright_offset: f64,
    decay_scale: f64,
}

impl StringVoice {
    fn new(sample_rate: f64) -> Self {
        let max_period = (sample_rate / MIN_HZ).ceil() as usize + 4;
        StringVoice {
            strings: [StringLoop::new(sample_rate), StringLoop::new(sample_rate)],
            excitation: vec![0.0; 2 * max_period + 8],
            exc_len: 0,
            exc_pos: 0,
            exc_split: 0.0,
            choke_left: 0,
            held: false,
            released: false,
            since_pluck: 1e9,
            bend_amount: 0.0,
            retune_count: 0,
            click_left: 0,
            click_env: 0.0,
            click_level: 0.0,
            click_decay: 0.0,
            click_hp: (0.0, 0.0),
            hz: 110.0,
            midi: 45.0,
            began: 0.0,
            until: 0.0,
            level: 0.0,
            active: true,
            inputs: [f64::NAN; 10],
            bright_offset: 0.0,
            decay_scale: 1.0,
        }
    }

    /// Retunes both polarizations for pitch `hz` (+ `vibrato` cents) and the playing params
    /// (each loop recomputes its coefficients only when its key changed).
    fn retune(&mut self, sample_rate: f64, p: &[f64], hz: f64, vibrato: f64) {
        let bend = self.bend_amount * (-self.since_pluck / BEND_TAU).exp();
        // The raw inputs: unchanged (the common case, every sample of a held note) means no work.
        let inputs = [
            hz,
            bend + vibrato,
            p[DECAY] * self.decay_scale,
            p[DECAY_TILT],
            p[MUTE],
            if self.released { p[RELEASE] } else { -1.0 },
            p[DAMPING],
            p[STIFFNESS],
            p[DETUNE],
            p[DAMPING_FREQ],
        ];
        if inputs == self.inputs {
            return;
        }
        self.inputs = inputs;
        let mut f = hz.clamp(MIN_HZ, MAX_HZ);
        let cents = bend + vibrato;
        if cents.abs() > 0.05 {
            f *= 2f64.powf(cents / 1200.0);
        }
        let mute = p[MUTE].clamp(0.0, 1.0);
        let tilt = p[DECAY_TILT].clamp(0.0, 1.0);
        let mut t60 = (p[DECAY] * self.decay_scale).clamp(0.05, 30.0) * (110.0 / f).powf(tilt) * (1.0 - 0.96 * mute);
        if self.released {
            t60 = t60.min(p[RELEASE].clamp(0.01, 5.0));
        }
        let t60 = t60.clamp(0.02, 40.0);
        let damping = p[DAMPING].clamp(0.0, 1.0);
        let damping = damping + (1.0 - damping) * 0.7 * mute;
        let stiffness = p[STIFFNESS];
        let detune = 2f64.powf(p[DETUNE].clamp(0.0, 20.0) / 1200.0);
        let reference = p[DAMPING_FREQ];
        self.strings[0].retune(sample_rate, [f, t60, damping, stiffness, reference]);
        let t60b = if self.released { t60 } else { t60 * POL_T60 };
        self.strings[1].retune(sample_rate, [f * detune, t60b, damping * POL_DAMPING, stiffness, reference]);
    }

    /// Builds the excitation for one pluck into the preallocated buffer (no allocation).
    fn excite(&mut self, sample_rate: f64, amplitude: f64, p: &[f64], rng: &mut Rng) {
        if self.choke_left == 0 && self.exc_pos >= self.exc_len {
            self.choke_left = self.strings[0].tuning.delay;
        }
        let cap = self.excitation.len() / 2;
        let len = (self.strings[0].tuning.period.round() as usize).clamp(2, cap);
        let hardness = p[PICK_HARDNESS].clamp(0.0, 1.0);
        let beta = p[PICK_POSITION].clamp(0.02, 0.5);
        let apex = beta.max(1.0 / len as f64);

        // Soft triangular displacement (zero mean) blended with a noise burst.
        let mut mean = 0.0;
        for i in 0..len {
            let x = (i as f64 + 0.5) / len as f64;
            let tri = if x < apex { x / apex } else { (1.0 - x) / (1.0 - apex) };
            let noise = rng.next_f64() * 2.0 - 1.0;
            let value = (1.0 - hardness) * tri + hardness * noise;
            self.excitation[i] = value;
            mean += value;
        }
        mean /= len as f64;
        for value in &mut self.excitation[..len] {
            *value -= mean;
        }

        // Pick position comb: e[i] - e[i - m], extending the burst by m samples.
        let m = ((beta * len as f64).round() as usize).clamp(1, len);
        let total = len + m;
        for i in (0..total).rev() {
            let now = if i < len { self.excitation[i] } else { 0.0 };
            let back = if i >= m { self.excitation[i - m] } else { 0.0 };
            self.excitation[i] = now - back;
        }

        // Two one-pole low-passes: brightness and velocity open the attack tone, a mute closes it.
        let velocity = amplitude.clamp(0.0, 1.0);
        let vb = p[VELOCITY_BRIGHTNESS].clamp(0.0, 1.0);
        let mute = p[MUTE].clamp(0.0, 1.0);
        let opening = (1.0 - vb + vb * velocity) * (1.0 - 0.6 * mute);
        let brightness = (p[BRIGHTNESS] + self.bright_offset).clamp(0.0, 1.0);
        let cutoff = (150.0 * 2f64.powf(brightness * 7.0) * opening).clamp(40.0, 0.45 * sample_rate);
        let pole = (-TAU * cutoff / sample_rate).exp();
        let (mut s1, mut s2, mut peak) = (0.0f64, 0.0f64, 0.0f64);
        for value in &mut self.excitation[..total] {
            s1 = (1.0 - pole) * *value + pole * s1;
            s2 = (1.0 - pole) * s1 + pole * s2;
            *value = s2;
            peak = peak.max(s2.abs());
        }
        let scale = if peak > 1e-12 { amplitude / peak } else { 0.0 };
        for value in &mut self.excitation[..total] {
            *value *= scale;
        }
        self.exc_len = total;
        self.exc_pos = 0;
        self.exc_split = p[POLARIZATION].clamp(0.0, 1.0);
        self.released = false;
        self.since_pluck = 0.0;
        self.bend_amount = BEND_CENTS * p[ATTACK_BEND].clamp(0.0, 1.0) * velocity * velocity;
        self.retune_count = 0;
        self.active = true;
        let click = p[PICK_NOISE].clamp(0.0, 1.0);
        if click > 0.0 && amplitude > 0.0 {
            let tau = CLICK_TAU + CLICK_SOFT_TAU * (1.0 - hardness);
            self.click_left = (5.0 * tau * sample_rate) as usize;
            self.click_decay = (-1.0 / (tau * sample_rate)).exp();
            self.click_env = 1.0;
            self.click_level = click * velocity * (0.3 + 0.7 * hardness);
        }
    }

    /// Lets the string go: the sustain falls to `release`.
    fn release(&mut self) {
        if self.held {
            self.held = false;
            self.released = true;
            self.retune_count = 0;
        }
    }

    /// One sample of this string: the pickup's raw view (before the coil) plus the click.
    fn tick(&mut self, sample_rate: f64, p: &[f64], vibrato: f64, kind: usize, rng: &mut Rng) -> f64 {
        if !self.active {
            return 0.0;
        }
        let dt = 1.0 / sample_rate;
        self.since_pluck += dt;
        // A moving pitch (vibrato, attack bend) retunes every RETUNE_EVERY samples; static params
        // are compared every sample and retune only on a change.
        let moving = vibrato != 0.0 || self.since_pluck < 6.0 * BEND_TAU;
        if !moving || self.retune_count == 0 {
            self.retune(sample_rate, p, self.hz, vibrato);
        }
        self.retune_count = (self.retune_count + 1) % RETUNE_EVERY;

        let input = if self.exc_pos < self.exc_len {
            let e = self.excitation[self.exc_pos];
            self.exc_pos += 1;
            e
        } else {
            0.0
        };
        let choke = if self.choke_left > 0 {
            self.choke_left -= 1;
            CHOKE
        } else {
            1.0
        };
        let split = self.exc_split;
        let ya = self.strings[0].tick(input * (1.0 - split), choke);
        let yb = self.strings[1].tick(input * split, choke);
        let position = p[PICKUP_POSITION].clamp(0.04, 0.5);
        let mut out = self.strings[0].pickup(kind, ya, position) + self.strings[1].pickup(kind, yb, position);

        // Pick click: decaying high-passed noise, outside the string loop.
        if self.click_left > 0 {
            self.click_left -= 1;
            let noise = rng.next_f64() * 2.0 - 1.0;
            let a = (-TAU * CLICK_HZ / sample_rate).exp();
            let (x1, y1) = self.click_hp;
            let hp = a * (y1 + noise - x1);
            self.click_hp = (noise, hp);
            out += 0.5 * self.click_level * self.click_env * hp;
            self.click_env *= self.click_decay;
        }

        self.level = (ya.abs() + yb.abs()).max(self.level * IDLE_DECAY);
        if self.level < IDLE_LEVEL && self.exc_pos >= self.exc_len && self.click_left == 0 {
            self.clear();
        }
        out
    }

    fn clear(&mut self) {
        for string in &mut self.strings {
            string.reset();
        }
        self.exc_len = 0;
        self.exc_pos = 0;
        self.choke_left = 0;
        self.click_left = 0;
        self.click_hp = (0.0, 0.0);
        self.level = 0.0;
        self.active = false;
    }
}

/// One note of the `score` param (the `notes` node's format; `level` scales the velocity).
#[derive(Clone, Copy, Debug, serde::Deserialize)]
struct ScoreNote {
    start: f64,
    duration: f64,
    midi: f64,
    #[serde(default = "full_level")]
    level: f64,
    /// Added to the node's brightness for this note.
    #[serde(default)]
    brightness: f64,
    /// Multiplies the node's decay (T60) for this note.
    #[serde(default = "full_level")]
    decay: f64,
}

fn full_level() -> f64 {
    1.0
}

/// Parses a score: notes may overlap (each takes a free string); invalid notes are dropped.
fn parse_score(text: &str) -> Vec<ScoreNote> {
    let Ok(mut notes) = serde_json::from_str::<Vec<ScoreNote>>(text) else {
        return Vec::new();
    };
    notes.retain(|n| {
        n.start.is_finite()
            && n.duration.is_finite()
            && n.start >= 0.0
            && n.duration >= 0.005
            && (0.0..=127.0).contains(&n.midi)
            && (0.0..=1.0).contains(&n.level)
            && n.brightness.is_finite()
            && n.decay.is_finite()
    });
    for n in &mut notes {
        n.brightness = n.brightness.clamp(-1.0, 1.0);
        n.decay = n.decay.clamp(0.05, 20.0);
    }
    notes.truncate(MAX_SCORE_NOTES);
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    notes
}

struct Pluck {
    sample_rate: f64,
    voices: Vec<StringVoice>,
    score: Vec<ScoreNote>,
    next_note: usize,
    last_t: f64,
    rng: Rng,
    started: bool,
    gate_prev: f64,
    pickup_kind: usize,
    coil: Biquad,
    body: [Biquad; 6],
    body_size: f64,
    dc: DcBlocker,
    vibrato_phase: f64,
}

impl Pluck {
    fn new(sample_rate: f64, seed: u32, score: Vec<ScoreNote>) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let count = if score.is_empty() { 1 } else { MAX_VOICES };
        let mut node = Self {
            sample_rate,
            voices: (0..count).map(|_| StringVoice::new(sample_rate)).collect(),
            score,
            next_note: 0,
            last_t: -1.0,
            rng: Rng::new(seed ^ 0x9e37_79b9),
            started: false,
            gate_prev: 0.0,
            pickup_kind: usize::MAX,
            coil: Biquad::new(),
            body: [Biquad::new(), Biquad::new(), Biquad::new(), Biquad::new(), Biquad::new(), Biquad::new()],
            body_size: 0.0,
            dc: DcBlocker::new(sample_rate),
            vibrato_phase: 0.0,
        };
        node.set_body_size(1.0);
        node.set_pickup(0);
        node
    }

    fn set_body_size(&mut self, size: f64) {
        let size = size.clamp(0.5, 2.0);
        if (size - self.body_size).abs() < 1e-3 {
            return;
        }
        self.body_size = size;
        let top = 0.45 * self.sample_rate;
        for (filter, (f, q, _)) in self.body.iter_mut().zip(BODY_MODES) {
            filter.set(BiquadKind::Bandpass, (f / size).min(top), q, 0.0, self.sample_rate);
        }
    }

    fn set_pickup(&mut self, kind: usize) {
        if kind == self.pickup_kind {
            return;
        }
        self.pickup_kind = kind;
        match kind {
            1 => self.coil.set(BiquadKind::Lowpass, 5_200.0, 1.8, 0.0, self.sample_rate),
            2 => self.coil.set(BiquadKind::Lowpass, 3_100.0, 1.25, 0.0, self.sample_rate),
            _ => self.coil = Biquad::new(),
        }
        self.coil.reset();
    }

    /// Gate mode: the single string follows `frequency`, `gate` and `autoStrike`.
    fn play_gate(&mut self, ctx: &SampleCtx<'_>, p: &[f64]) {
        let gate = p[GATE];
        let rising = gate > GATE_ON && self.gate_prev <= GATE_ON;
        let auto = (!self.started || ctx.looped) && p[AUTO_STRIKE] > 0.5;
        self.gate_prev = gate;
        let sr = self.sample_rate;
        let voice = &mut self.voices[0];
        voice.hz = p[FREQUENCY];
        if voice.held && gate <= GATE_ON {
            voice.release();
        }
        if rising || auto {
            let level = if rising { gate.min(1.0) } else { 1.0 };
            voice.retune(sr, p, voice.hz, 0.0);
            voice.excite(sr, p[VELOCITY].clamp(0.0, 1.0) * level, p, &mut self.rng);
            voice.retune(sr, p, voice.hz, 0.0);
            voice.held = rising;
        }
    }

    /// Score mode: each note takes a free string (the nearest in pitch), or steals the oldest.
    fn play_score(&mut self, ctx: &SampleCtx<'_>, p: &[f64]) {
        let t = ctx.t * ctx.duration;
        let count = (p[VOICES].round() as usize).clamp(1, self.voices.len());
        if t < self.last_t - 1e-9 || ctx.looped {
            self.next_note = self.score.partition_point(|n| n.start < t);
            for voice in &mut self.voices {
                voice.release();
            }
        }
        self.last_t = t;
        for voice in &mut self.voices {
            if voice.held && t >= voice.until {
                voice.release();
            }
        }
        let sr = self.sample_rate;
        while self.next_note < self.score.len() && self.score[self.next_note].start <= t {
            let note = self.score[self.next_note];
            self.next_note += 1;
            let free = (0..count)
                .filter(|&i| !self.voices[i].held)
                .min_by(|&a, &b| {
                    let cost = |v: &StringVoice| (v.midi - note.midi).abs() + if v.active { 24.0 * v.level.min(1.0) } else { 0.0 };
                    cost(&self.voices[a]).total_cmp(&cost(&self.voices[b]))
                });
            let pick = free.unwrap_or_else(|| {
                (0..count).min_by(|&a, &b| self.voices[a].began.total_cmp(&self.voices[b].began)).unwrap_or(0)
            });
            let voice = &mut self.voices[pick];
            voice.hz = 440.0 * 2f64.powf((note.midi - 69.0) / 12.0);
            voice.midi = note.midi;
            voice.began = note.start;
            voice.until = note.start + note.duration;
            voice.released = false;
            voice.since_pluck = 0.0;
            voice.bright_offset = note.brightness;
            voice.decay_scale = note.decay;
            voice.retune(sr, p, voice.hz, 0.0);
            voice.excite(sr, p[VELOCITY].clamp(0.0, 1.0) * note.level, p, &mut self.rng);
            voice.retune(sr, p, voice.hz, 0.0);
            voice.held = true;
        }
    }
}

impl Node for Pluck {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, _input: (f64, f64), p: &[f64]) -> NodeOut {
        if self.score.is_empty() {
            self.play_gate(ctx, p);
        } else {
            self.play_score(ctx, p);
        }
        self.started = true;
        self.set_pickup(p[PICKUP].round() as usize);
        self.set_body_size(p[BODY_SIZE]);

        let sr = self.sample_rate;
        self.vibrato_phase = (self.vibrato_phase + p[VIBRATO_RATE].clamp(0.5, 12.0) / sr).fract();
        let vibrato = p[VIBRATO_DEPTH].clamp(0.0, 100.0) * (TAU * self.vibrato_phase).sin();
        let kind = self.pickup_kind;
        let mut raw = 0.0;
        for voice in &mut self.voices {
            raw += voice.tick(sr, p, vibrato, kind, &mut self.rng);
        }
        let pickup = if kind == 1 || kind == 2 { self.coil.process(raw) } else { raw };

        let body_mix = p[BODY].clamp(0.0, 1.0);
        let mut resonance = 0.0;
        for (filter, (_, _, weight)) in self.body.iter_mut().zip(BODY_MODES) {
            resonance += weight * filter.process(pickup);
        }
        let mixed = pickup * (1.0 - 0.6 * body_mix) + body_mix * BODY_GAIN * resonance;
        let out = self.dc.process(mixed) * p[GAIN].clamp(0.0, 2.0) * OUT_SCALE;
        let out = if out.is_finite() {
            out.clamp(-2.0, 2.0)
        } else {
            self.reset();
            0.0
        };
        NodeOut { l: out, r: out, value: 0.0 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        let score = parse_score(init.texts.get(SCORE).and_then(|s| s.as_deref()).unwrap_or(""));
        let voices = if score.is_empty() { 1 } else { MAX_VOICES };
        if (init.sample_rate - self.sample_rate).abs() > 0.5 || voices != self.voices.len() {
            *self = Pluck::new(init.sample_rate, init.seed, score);
        } else {
            self.score = score;
            self.next_note = 0;
            self.last_t = -1.0;
        }
    }

    fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.clear();
            voice.active = true;
            voice.held = false;
            voice.released = false;
            voice.since_pluck = 1e9;
            voice.retune_count = 0;
        }
        self.coil.reset();
        for filter in &mut self.body {
            filter.reset();
        }
        self.dc.reset();
        self.started = false;
        self.gate_prev = 0.0;
        self.vibrato_phase = 0.0;
        self.next_note = 0;
        self.last_t = -1.0;
    }
}
