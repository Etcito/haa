//! `pluck` source node: a plucked string, extended Karplus-Strong digital waveguide.
//! HAA-only node (no Sona counterpart), one string per node: a guitar chord is several nodes.
//!
//! Loop: integer delay line -> symmetric 3-tap loss filter (linear phase, exactly 1 sample) ->
//! optional stiffness allpass chain (inharmonicity) -> first-order allpass fine tuning -> loop
//! gain. The tuning solves the exact phase delay of every loop element at the fundamental, so the
//! pitch holds from bass E1 (41 Hz) to the top of the guitar neck.
//!
//! Excitation (on each pluck): one period of a blend of a soft triangular displacement and a noise
//! burst (`pickHardness`), comb-filtered at `pickPosition`, low-passed by `brightness` and
//! velocity, then injected into the loop. Re-plucking a ringing string chokes the old vibration.
//!
//! Output: `pickup` none (bridge signal, acoustic) or a magnetic pickup: a comb at
//! `pickupPosition` (one coil for `single`, two coils 3% of the scale apart for `humbucker`) and
//! the coil's resonant low-pass. `body` mixes in fixed acoustic-guitar body modes (air ~100 Hz,
//! top plate ~200/400 Hz, upper bands). Pitch slides and hammer-ons compose with `notes` + `glide`.
//!
//! Gate: a rising edge of `gate` above 0.01 plucks with velocity x gate level; when that gate
//! falls back the string is damped with `release` (a fretting finger lifting). `autoStrike`
//! plucks once on Play and on each loop, like `membrane`.

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
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Pluck::new(init.sample_rate, init.seed))
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

/// Loop coefficients for one (frequency, T60, damping, stiffness) setting.
#[derive(Clone, Copy)]
struct Tuning {
    key: [f64; 4],
    period: f64,
    delay: usize,
    loss_a: f64,
    loop_gain: f64,
    stiff_a: f64,
    stiff_on: bool,
    tune_a: f64,
}

fn tune(sample_rate: f64, frequency: f64, t60: f64, damping: f64, stiffness: f64) -> Tuning {
    let key = [frequency, t60, damping, stiffness];
    let f = frequency.clamp(MIN_HZ, MAX_HZ.min(sample_rate * 0.2));
    let period = sample_rate / f;
    let w0 = TAU * f / sample_rate;

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

    // Integer delay plus a fine-tuning allpass of 0.5..1.5 samples; the loss filter is 1 sample.
    let rest = period - 1.0 - stiff_delay;
    let delay = (rest - 0.5).floor().max(1.0);
    let target = rest - delay;
    let mut d = target.clamp(0.05, 2.5);
    let mut tune_a = (1.0 - d) / (1.0 + d);
    for _ in 0..3 {
        d = (d + target - allpass_delay(tune_a, w0)).clamp(0.05, 2.5);
        tune_a = (1.0 - d) / (1.0 + d);
    }

    // Loss filter g * (a z + (1 - 2a) + a z^-1): per-period gain at f0 from T60, and the T60 at a
    // high reference frequency shortened by `damping`.
    let t60 = t60.max(0.01);
    let g0 = 10f64.powf(-3.0 / (t60 * f));
    let ratio = 0.5 * 0.02f64.powf(damping.clamp(0.0, 1.0));
    let fh = (4_000.0f64).max(2.0 * f).min(0.45 * sample_rate);
    let mut loss_a = 0.0;
    let c0 = 1.0 - w0.cos();
    if fh > 1.01 * f {
        let gh = 10f64.powf(-3.0 / (t60 * ratio * f));
        let r = gh / g0;
        let ch = 1.0 - (TAU * fh / sample_rate).cos();
        let denominator = 2.0 * (ch - r * c0);
        if denominator > 1e-12 {
            loss_a = ((1.0 - r) / denominator).clamp(0.0, 0.25);
        }
    }
    let loop_gain = (g0 / (1.0 - 2.0 * loss_a * c0)).min(0.99995);
    Tuning {
        key,
        period,
        delay: delay as usize,
        loss_a,
        loop_gain,
        stiff_a,
        stiff_on,
        tune_a,
    }
}

struct Pluck {
    sample_rate: f64,
    line: DelayLine,
    fir1: f64,
    fir2: f64,
    stiff: [Allpass; DISPERSION_STAGES],
    fine: Allpass,
    tuning: Tuning,
    excitation: Vec<f64>,
    exc_len: usize,
    exc_pos: usize,
    choke_left: usize,
    rng: Rng,
    started: bool,
    gate_prev: f64,
    held: bool,
    released: bool,
    pickup_kind: usize,
    coil: Biquad,
    body: [Biquad; 6],
    dc: DcBlocker,
}

impl Pluck {
    fn new(sample_rate: f64, seed: u32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let max_period = (sample_rate / MIN_HZ).ceil() as usize + 4;
        let mut body = [Biquad::new(), Biquad::new(), Biquad::new(), Biquad::new(), Biquad::new(), Biquad::new()];
        for (filter, (f, q, _)) in body.iter_mut().zip(BODY_MODES) {
            filter.set(BiquadKind::Bandpass, f, q, 0.0, sample_rate);
        }
        let mut node = Self {
            sample_rate,
            line: DelayLine::new(1.1 / MIN_HZ, sample_rate),
            fir1: 0.0,
            fir2: 0.0,
            stiff: [Allpass::default(); DISPERSION_STAGES],
            fine: Allpass::default(),
            tuning: tune(sample_rate, 110.0, 4.0, 0.35, 0.1),
            excitation: vec![0.0; 2 * max_period + 8],
            exc_len: 0,
            exc_pos: 0,
            choke_left: 0,
            rng: Rng::new(seed ^ 0x9e37_79b9),
            started: false,
            gate_prev: 0.0,
            held: false,
            released: false,
            pickup_kind: usize::MAX,
            coil: Biquad::new(),
            body,
            dc: DcBlocker::new(sample_rate),
        };
        node.set_pickup(0);
        node
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

    fn apply_tuning(&mut self, t: Tuning) {
        self.tuning = t;
        for stage in &mut self.stiff {
            stage.a = t.stiff_a;
        }
        self.fine.a = t.tune_a;
    }

    /// Builds the excitation for one pluck into the preallocated buffer (no allocation).
    fn excite(&mut self, amplitude: f64, p: &[f64]) {
        if self.choke_left == 0 && self.exc_pos >= self.exc_len {
            self.choke_left = self.tuning.delay;
        }
        let cap = self.excitation.len() / 2;
        let len = (self.tuning.period.round() as usize).clamp(2, cap);
        let hardness = p[PICK_HARDNESS].clamp(0.0, 1.0);
        let beta = p[PICK_POSITION].clamp(0.02, 0.5);
        let apex = beta.max(1.0 / len as f64);

        // Soft triangular displacement (zero mean) blended with a noise burst.
        let mut mean = 0.0;
        for i in 0..len {
            let x = (i as f64 + 0.5) / len as f64;
            let tri = if x < apex { x / apex } else { (1.0 - x) / (1.0 - apex) };
            let noise = self.rng.next_f64() * 2.0 - 1.0;
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

        // Two one-pole low-passes: brightness and velocity open the attack tone.
        let velocity = amplitude.clamp(0.0, 1.0);
        let cutoff = (150.0 * 2f64.powf(p[BRIGHTNESS].clamp(0.0, 1.0) * 7.0) * (0.55 + 0.45 * velocity))
            .min(0.45 * self.sample_rate);
        let pole = (-TAU * cutoff / self.sample_rate).exp();
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
        self.released = false;
    }

    fn clear(&mut self) {
        self.line.reset();
        self.fir1 = 0.0;
        self.fir2 = 0.0;
        for stage in &mut self.stiff {
            stage.reset();
        }
        self.fine.reset();
        self.exc_len = 0;
        self.exc_pos = 0;
        self.choke_left = 0;
        self.coil.reset();
        for filter in &mut self.body {
            filter.reset();
        }
        self.dc.reset();
    }
}

impl Node for Pluck {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, _input: (f64, f64), p: &[f64]) -> NodeOut {
        let gate = p[GATE];
        let rising = gate > GATE_ON && self.gate_prev <= GATE_ON;
        let auto = (!self.started || ctx.looped) && p[AUTO_STRIKE] > 0.5;
        self.started = true;
        self.gate_prev = gate;
        if self.held && gate <= GATE_ON {
            self.held = false;
            self.released = true;
        }

        let decay = p[DECAY].clamp(0.05, 30.0);
        let t60 = if self.released { decay.min(p[RELEASE].clamp(0.01, 5.0)) } else { decay };
        let key = [p[FREQUENCY], t60, p[DAMPING], p[STIFFNESS]];
        if key != self.tuning.key {
            let t = tune(self.sample_rate, key[0], key[1], key[2], key[3]);
            self.apply_tuning(t);
        }

        if rising || auto {
            let level = if rising { gate.min(1.0) } else { 1.0 };
            self.excite(p[VELOCITY].clamp(0.0, 1.0) * level, p);
            self.held = rising;
        }
        self.set_pickup(p[PICKUP].round() as usize);

        // One trip around the string loop.
        let t = self.tuning;
        let v = self.line.read(t.delay as f64);
        let mut y = t.loss_a * (v + self.fir2) + (1.0 - 2.0 * t.loss_a) * self.fir1;
        self.fir2 = self.fir1;
        self.fir1 = v;
        if t.stiff_on {
            for stage in &mut self.stiff {
                y = stage.process(y);
            }
        }
        y = self.fine.process(y) * t.loop_gain;
        if self.choke_left > 0 {
            y *= CHOKE;
            self.choke_left -= 1;
        }
        if self.exc_pos < self.exc_len {
            y += self.excitation[self.exc_pos];
            self.exc_pos += 1;
        }
        if !y.is_finite() {
            self.clear();
            y = 0.0;
        }
        self.line.write(y);

        // Pickup: displacement read at a point along the string, then the coil resonance.
        let pickup = match self.pickup_kind {
            1 => {
                let back = self.line.read(1.0 + p[PICKUP_POSITION].clamp(0.04, 0.5) * t.period);
                self.coil.process(0.8 * (y - back))
            }
            2 => {
                let beta = p[PICKUP_POSITION].clamp(0.04, 0.5);
                let a = self.line.read(1.0 + beta * t.period);
                let b = self.line.read(1.0 + (beta + HUMBUCKER_SPACING).min(0.5) * t.period);
                self.coil.process(0.5 * ((y - a) + (y - b)))
            }
            _ => y,
        };

        let body_mix = p[BODY].clamp(0.0, 1.0);
        let mut resonance = 0.0;
        for (filter, (_, _, weight)) in self.body.iter_mut().zip(BODY_MODES) {
            resonance += weight * filter.process(pickup);
        }
        let mixed = pickup * (1.0 - 0.6 * body_mix) + body_mix * BODY_GAIN * resonance;
        let out = self.dc.process(mixed) * p[GAIN].clamp(0.0, 2.0) * OUT_SCALE;
        let out = if out.is_finite() { out.clamp(-2.0, 2.0) } else { 0.0 };
        NodeOut { l: out, r: out, value: 0.0 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        if (init.sample_rate - self.sample_rate).abs() > 0.5 {
            *self = Pluck::new(init.sample_rate, init.seed);
        }
    }

    fn reset(&mut self) {
        self.clear();
        self.started = false;
        self.gate_prev = 0.0;
        self.held = false;
        self.released = false;
    }
}

