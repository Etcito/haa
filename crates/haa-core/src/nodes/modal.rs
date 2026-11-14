//! `modal` source node: a struck solid object, by modal synthesis. HAA-only node (no Sona counterpart).
//!
//! The object is a bank of up to 128 damped modes whose frequencies come from the vibration theory
//! of its `shape`, scaled so the lowest mode sits at `frequency`:
//! - `bar`: a free-free beam, ratios 1, 2.756, 5.404, 8.933... ((2n+1)/3)², plus a torsional
//!   series: a plank, a stick, a pipe, a rail.
//! - `plate`: a rectangular plate, f ∝ (m/a)² + (n/b)² (aspect drawn per voice): a panel, a sheet,
//!   a lid. Mode density is even in frequency, as in a real plate.
//! - `shell`: a thin cylindrical / cup shell, ring modes n(n²−1)/√(n²+1) and their axial family,
//!   every mode a slightly split doublet that beats: a bottle, a glass, a can, a bell.
//! - `box`: three plates of different panel sizes coupled at the edges: a crate, a cabinet.
//! - `block`: a thick solid, f ∝ √((l/a)² + (m/b)² + (n/c)²), mode density growing with f²:
//!   a stone, a brick, a lump of concrete.
//! - `sheet`: a thin, long plate (aspect ~2) that always glides a little in pitch when hit hard:
//!   a metal sheet, a roof panel, corrugated iron.
//! - `bottle`: a `shell` plus the air cavity (Helmholtz) far below the glass modes.
//!   `box` is hollow too: its air cavity rings at 0.62 × the lowest panel mode.
//!
//! `material` (wood, metal, glass, stone, plastic, ceramic, concrete, tile, snow) sets the damping
//! law: each mode's decay rate is a constant term (supports, air) plus π·η·f·(f/1 kHz)^slope
//! (internal friction, the material's loss factor η), so the upper modes die faster, as measured on
//! real materials. It also scales the pitch (the bending-wave speed: the same object rings higher in
//! steel than in wood, `frequency` is the wood value) and the contact time (a stiff surface strikes
//! shorter and brighter; snow is a soft, long contact with almost no ring). `decay` multiplies every
//! T60, `damping` scales the internal-friction part. Each mode's damping and gain is jittered per
//! voice. `shape` and `material` are hand-chosen models, never computed from a mesh; both can be
//! driven by a control (a preset's enum input), the rounded index picks the value.
//!
//! The strike is a Hertz contact: a force pulse sin^1.5(πt/τ) of unit impulse, so the contact time
//! τ sets the brightness (a pulse is flat up to about 1/τ) but not the level. τ is `contact`,
//! shortened by `force` (strike speed: τ ∝ v^-1/5, so ×2.4 from force 0 to 1). The modes are
//! weighted by the mode shapes at the strike `position`. `click` adds the sound radiated by the
//! contact itself (the striker's deceleration, the time derivative of the force): the tick.
//!
//! Nonlinear and stochastic parts, all scaled by `force` so a hard hit differs from a soft one in
//! more than level:
//! - `rattle`: loose parts (slats, a lid, nails) that knock while the body moves more than their
//!   play: micro-impacts at a rate that follows the body's vibration above a threshold, each
//!   ringing a small set of high, short modes of their own.
//! - `glide`: tension modulation; at large amplitude every mode is sharp by up to `glide` (a
//!   fraction) and settles as the vibration decays (a gong, a thin sheet, a can).
//! - `bounces`: the object chatters: after the first strike come up to 8 more, `bounceTime` apart
//!   at first, each gap and each strike scaled by `restitution`, with jitter.
//! - `vary`: every strike draws a new position, contact time, per-mode gain and a slight detune,
//!   and the random stream is not replayed by a reset, so two triggers never sound the same.
//!
//! An audio input is an extra force on the object (`input` gain): friction noise of a scrape or a
//! roll rings the same modes. `strike` off leaves only that input. The first strike comes after
//! `delay`. Output is mono (left = right): the runtime places the voice in 3D.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;
use core::f64::consts::{PI, TAU};

const SHAPE: usize = 0;
const MATERIAL: usize = 1;
const FREQUENCY: usize = 2;
const MODES: usize = 3;
const DECAY: usize = 4;
const DAMPING: usize = 5;
const CONTACT: usize = 6;
const FORCE: usize = 7;
const POSITION: usize = 8;
const VARY: usize = 9;
const CLICK: usize = 10;
const RATTLE: usize = 11;
const GLIDE: usize = 12;
const BOUNCES: usize = 13;
const BOUNCE_TIME: usize = 14;
const RESTITUTION: usize = 15;
const INPUT: usize = 16;
const STRIKE: usize = 17;
const DELAY: usize = 18;
const LEVEL: usize = 19;

// The order of the sfx contract's SHAPES (block, plate, sheet, bar, box, shell, bottle): a preset's
// `shape` input (0..6) wires straight into the enum. `block` (0) is the default arm of the matches.
const SHAPE_PLATE: usize = 1;
const SHAPE_SHEET: usize = 2;
const SHAPE_BAR: usize = 3;
const SHAPE_BOX: usize = 4;
const SHAPE_SHELL: usize = 5;
const SHAPE_BOTTLE: usize = 6;
const SHAPES: usize = 7;
/// Air cavity of a hollow shape (Helmholtz), as a ratio of the lowest structural mode: a box, a bottle.
const CAVITY_BOX: f64 = 0.62;
const CAVITY_BOTTLE: f64 = 0.14;
/// Tension glide a thin sheet always has (added to `glide`).
const SHEET_GLIDE: f64 = 0.012;

const MAX_MODES: usize = 128;
const RATTLE_MODES: usize = 6;
const MAX_STRIKES: usize = 9;
/// Retuning period (samples) while `glide` moves the modes.
const RETUNE: u32 = 32;

/// Per material: loss factor η, its frequency slope, the T60 (s) of the frequency-independent losses,
/// the pitch factor (bending-wave speed, √(E/ρ), relative to wood: the same object is higher in steel)
/// and the contact softness (contact time factor: a stiffer surface gives a shorter, brighter strike).
const MATERIALS: [[f64; 5]; 9] = [
    [0.015, 0.4, 0.8, 1.0, 1.0],    // wood
    [0.0004, 0.3, 6.0, 1.6, 0.6],   // metal (steel)
    [0.0012, 0.25, 3.0, 1.5, 0.5],  // glass
    [0.012, 0.3, 0.4, 1.2, 0.6],    // stone
    [0.03, 0.2, 0.4, 0.6, 1.5],     // plastic
    [0.002, 0.3, 1.5, 1.4, 0.5],    // ceramic
    [0.02, 0.3, 0.25, 1.0, 0.7],    // concrete: massive and lossy, a short dull ring
    [0.004, 0.3, 0.6, 1.3, 0.55],   // tile (fired clay): brighter and longer than concrete
    [0.3, 0.0, 0.04, 0.3, 6.0],     // snow: almost no ring, a soft long contact
];

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
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number { min, max, default, step, scale },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

/// The `modal` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "modal",
    category: Category::Source,
    label_en: "Struck object (modal)",
    label_fr: "Objet frappé (modal)",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "shape",
            kind: ParamKind::Enum { values: &["block", "plate", "sheet", "bar", "box", "shell", "bottle"], default: 1 },
            unit: "",
            label_en: "Shape",
            label_fr: "Forme",
            modulatable: true,
            smooth: false,
        },
        ParamDef {
            key: "material",
            kind: ParamKind::Enum {
                values: &["wood", "metal", "glass", "stone", "plastic", "ceramic", "concrete", "tile", "snow"],
                default: 0,
            },
            unit: "",
            label_en: "Material",
            label_fr: "Matière",
            modulatable: true,
            smooth: false,
        },
        number("frequency", 20.0, 10_000.0, 300.0, 1.0, Scale::Log, "Hz", "Lowest mode", "Mode le plus grave"),
        ParamDef {
            key: "modes",
            kind: ParamKind::Number { min: 1.0, max: MAX_MODES as f64, default: 48.0, step: 1.0, scale: Scale::Lin },
            unit: "",
            label_en: "Mode count",
            label_fr: "Nombre de modes",
            modulatable: false,
            smooth: false,
        },
        number("decay", 0.05, 10.0, 1.0, 0.01, Scale::Log, "×", "Resonance time", "Durée de résonance"),
        number("damping", 0.0, 4.0, 1.0, 0.01, Scale::Lin, "×", "Internal friction", "Friction interne"),
        number("contact", 0.00002, 0.02, 0.0005, 0.00001, Scale::Log, "s", "Contact time", "Durée de contact"),
        number("force", 0.0, 1.0, 0.5, 0.01, Scale::Lin, "", "Strike force", "Force de frappe"),
        number("position", 0.0, 1.0, 0.3, 0.01, Scale::Lin, "", "Strike position", "Point de frappe"),
        number("vary", 0.0, 1.0, 0.5, 0.01, Scale::Lin, "", "Variation per strike", "Variation par frappe"),
        number("click", 0.0, 2.0, 0.3, 0.01, Scale::Lin, "", "Contact click", "Claquement du contact"),
        number("rattle", 0.0, 2.0, 0.0, 0.01, Scale::Lin, "", "Loose parts rattle", "Cliquetis des parties lâches"),
        number("glide", 0.0, 0.05, 0.0, 0.0005, Scale::Lin, "", "Pitch glide (tension)", "Glissement (tension)"),
        number("bounces", 0.0, 8.0, 0.0, 1.0, Scale::Lin, "", "Bounces", "Rebonds"),
        number("bounceTime", 0.002, 0.5, 0.03, 0.001, Scale::Log, "s", "First bounce gap", "Premier écart de rebond"),
        number("restitution", 0.0, 0.95, 0.5, 0.01, Scale::Lin, "", "Restitution", "Restitution"),
        number("input", 0.0, 4.0, 1.0, 0.01, Scale::Lin, "×", "Input force", "Force de l'entrée"),
        ParamDef {
            key: "strike",
            kind: ParamKind::Bool { default: true },
            unit: "",
            label_en: "Strike on start",
            label_fr: "Frappe au départ",
            modulatable: false,
            smooth: false,
        },
        number("delay", 0.0, 2.0, 0.006, 0.001, Scale::Lin, "s", "Start delay", "Délai initial"),
        number("level", 0.0, 4.0, 1.0, 0.01, Scale::Lin, "", "Level", "Niveau"),
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let mut m = Modal::new(init.sample_rate, init.seed);
    m.build(init.params);
    Box::new(m)
}

/// A scheduled strike: start sample, amplitude, force.
#[derive(Clone, Copy, Default)]
struct Strike {
    at: u64,
    amp: f64,
    force: f64,
}

struct Modal {
    sample_rate: f64,
    seed: u32,
    rng: Rng,
    /// Geometry of this voice's object: shape draws (aspect ratios, doublet splits), fixed per seed.
    geometry: [f64; 6],
    /// Every shape's mode list (ratio, two mode-shape coordinates), computed once.
    tables: Vec<Vec<(f64, f64, f64)>>,
    /// Mode ratio (to the lowest), mode-shape coordinates (two numbers), per-voice gain and damping jitter.
    ratio: Vec<f64>,
    shape_a: Vec<f64>,
    shape_b: Vec<f64>,
    jitter_gain: Vec<f64>,
    jitter_damp: Vec<f64>,
    /// Oscillators.
    re: Vec<f64>,
    im: Vec<f64>,
    cr: Vec<f64>,
    ci: Vec<f64>,
    hz: Vec<f64>,
    radius: Vec<f64>,
    /// Excitation weight per mode for the current strike, and for the audio input.
    weight: Vec<f64>,
    base_weight: Vec<f64>,
    /// Audio input to force: 1 / (1 ms of samples).
    input_scale: f64,
    /// Rattle bank.
    r_re: [f64; RATTLE_MODES],
    r_im: [f64; RATTLE_MODES],
    r_cr: [f64; RATTLE_MODES],
    r_ci: [f64; RATTLE_MODES],
    r_gain: [f64; RATTLE_MODES],
    rattle_pulse: u32,
    rattle_amp: f64,
    /// Keys of the built bank.
    key: [f64; 6],
    built_shape: usize,
    built_count: usize,
    /// Strike schedule since the reset.
    strikes: [Strike; MAX_STRIKES],
    strike_count: usize,
    next_strike: usize,
    planned: bool,
    /// Current pulse: samples done, length, amplitude, previous force value (for the click).
    pulse_n: u32,
    pulse_len: u32,
    pulse_amp: f64,
    pulse_prev: f64,
    click_amp: f64,
    /// Body motion envelope (output level), its value at the last strike, the strike force.
    env: f64,
    env_peak: f64,
    strike_force: f64,
    detune: f64,
    frame: u64,
    retune_count: u32,
}

impl Modal {
    fn new(sample_rate: f64, seed: u32) -> Self {
        let mut rng = Rng::new(seed ^ 0x2545_f491);
        let geometry = [rng.next_f64(), rng.next_f64(), rng.next_f64(), rng.next_f64(), rng.next_f64(), rng.next_f64()];
        Modal {
            sample_rate: if sample_rate.is_finite() && sample_rate > 1.0 { sample_rate } else { 48_000.0 },
            seed,
            rng,
            geometry,
            tables: Vec::new(),
            ratio: Vec::with_capacity(MAX_MODES + 1),
            shape_a: Vec::with_capacity(MAX_MODES),
            shape_b: Vec::with_capacity(MAX_MODES),
            jitter_gain: Vec::with_capacity(MAX_MODES),
            jitter_damp: Vec::with_capacity(MAX_MODES),
            re: vec![0.0; MAX_MODES],
            im: vec![0.0; MAX_MODES],
            cr: vec![0.0; MAX_MODES],
            ci: vec![0.0; MAX_MODES],
            hz: vec![0.0; MAX_MODES],
            radius: vec![0.0; MAX_MODES],
            weight: vec![0.0; MAX_MODES],
            base_weight: vec![0.0; MAX_MODES],
            input_scale: 1.0 / (0.001 * if sample_rate.is_finite() && sample_rate > 1.0 { sample_rate } else { 48_000.0 }),
            r_re: [0.0; RATTLE_MODES],
            r_im: [0.0; RATTLE_MODES],
            r_cr: [0.0; RATTLE_MODES],
            r_ci: [0.0; RATTLE_MODES],
            r_gain: [0.0; RATTLE_MODES],
            rattle_pulse: 0,
            rattle_amp: 0.0,
            key: [f64::NAN; 6],
            built_shape: usize::MAX,
            built_count: 0,
            strikes: [Strike::default(); MAX_STRIKES],
            strike_count: 0,
            next_strike: 0,
            planned: false,
            pulse_n: 0,
            pulse_len: 0,
            pulse_amp: 0.0,
            pulse_prev: 0.0,
            click_amp: 0.0,
            env: 0.0,
            env_peak: 1e-9,
            strike_force: 0.5,
            detune: 1.0,
            frame: 0,
            retune_count: 0,
        }
    }

    fn u(&mut self) -> f64 {
        self.rng.next_f64()
    }

    /// Mode ratios and shape coordinates of `shape`, `count` modes (sorted, lowest = 1).
    fn shape_modes(&self, shape: usize, count: usize) -> Vec<(f64, f64, f64)> {
        let g = self.geometry;
        let mut list: Vec<(f64, f64, f64)> = Vec::new();
        match shape {
            SHAPE_BAR => {
                let first = [1.0, 2.756, 5.404, 8.933, 13.344];
                for n in 0..count {
                    let r = if n < first.len() { first[n] } else { ((2.0 * n as f64 + 3.0) / 3.0).powi(2) };
                    list.push((r, n as f64 + 1.5, 0.0));
                }
                // Torsional series of a bar of finite width: harmonic, from 1.6..3 × the first bending mode.
                let t0 = 1.6 + 1.4 * g[0];
                for m in 1..=count {
                    list.push((t0 * m as f64, m as f64, 1.0));
                }
            }
            SHAPE_PLATE | SHAPE_BOX | SHAPE_SHEET => {
                let panels: Vec<(f64, f64)> = if shape == SHAPE_PLATE {
                    vec![(1.0, 1.25 + 0.5 * g[0])]
                } else if shape == SHAPE_SHEET {
                    vec![(1.0, 1.8 + 0.6 * g[0])]
                } else {
                    let (a, b, c) = (1.0, 0.75 + 0.15 * g[0], 0.55 + 0.15 * g[1]);
                    vec![(a, b), (b, c), (a, c)]
                };
                let side = (count as f64).sqrt().ceil() as usize + 3;
                for (pi, &(a, b)) in panels.iter().enumerate() {
                    for m in 1..=side {
                        for n in 1..=side {
                            let r = (m as f64 / a).powi(2) + (n as f64 / b).powi(2);
                            list.push((r, m as f64 + 10.0 * pi as f64, n as f64));
                        }
                    }
                }
            }
            SHAPE_SHELL | SHAPE_BOTTLE => {
                let base = |n: f64| n * (n * n - 1.0) / (n * n + 1.0).sqrt();
                let b2 = base(2.0);
                let axial = 1.3 + 0.3 * g[0];
                for n in 2..(count + 2) {
                    let r = base(n as f64) / b2;
                    for (family, f) in [(0.0, 1.0), (1.0, axial + 0.04 * n as f64)] {
                        let split = 1.0 + 0.002 + 0.008 * ((g[1] * 7.0 + n as f64 * 0.37 + family).fract());
                        list.push((r * f, n as f64, family * 2.0));
                        list.push((r * f * split, n as f64, family * 2.0 + 1.0));
                    }
                }
            }
            _ => {
                let (a, b, c) = (1.0, 0.8 + 0.15 * g[0], 0.6 + 0.2 * g[1]);
                let side = ((count as f64).cbrt().ceil() as usize + 3).max(4);
                for l in 0..=side {
                    for m in 0..=side {
                        for n in 0..=side {
                            if l + m + n == 0 {
                                continue;
                            }
                            let r = ((l as f64 / a).powi(2) + (m as f64 / b).powi(2) + (n as f64 / c).powi(2)).sqrt();
                            list.push((r, (l * 31 + m * 7 + n) as f64, (l + 2 * m + 3 * n) as f64));
                        }
                    }
                }
            }
        }
        list.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(core::cmp::Ordering::Equal));
        list.dedup_by(|x, y| (x.0 - y.0).abs() < 1e-9 && x.1 == y.1 && x.2 == y.2);
        list.truncate(count);
        let lowest = list.first().map_or(1.0, |m| m.0);
        list.iter_mut().for_each(|m| m.0 /= lowest);
        // A hollow shape's air cavity (a = -1): below the lowest structural mode, which stays `frequency`.
        let cavity = match shape {
            SHAPE_BOX => Some(CAVITY_BOX),
            SHAPE_BOTTLE => Some(CAVITY_BOTTLE),
            _ => None,
        };
        if let Some(ratio) = cavity {
            list.insert(0, (ratio, -1.0, -1.0));
        }
        list
    }

    /// Selects the mode list (shape or count changed). Every shape's list is computed once at creation,
    /// so a shape chosen by a control while playing allocates nothing.
    fn build(&mut self, p: &[f64]) {
        let shape = (p[SHAPE].round().max(0.0) as usize).min(SHAPES - 1);
        let count = (p[MODES].round() as usize).clamp(1, MAX_MODES);
        if shape == self.built_shape && count == self.built_count {
            return;
        }
        if self.tables.is_empty() {
            self.tables = (0..SHAPES).map(|s| self.shape_modes(s, MAX_MODES)).collect();
        }
        self.built_shape = shape;
        self.built_count = count;
        let mut jr = Rng::new(self.seed ^ 0x68e3_1da4);
        self.ratio.clear();
        self.shape_a.clear();
        self.shape_b.clear();
        self.jitter_gain.clear();
        self.jitter_damp.clear();
        // The lowest `count` entries (a hollow shape's cavity first, then the structural modes).
        for &(r, a, b) in self.tables[shape].iter().take(count) {
            self.ratio.push(r);
            self.shape_a.push(a);
            self.shape_b.push(b);
            self.jitter_gain.push(0.35 + 0.65 * jr.next_f64());
            self.jitter_damp.push(0.75 + 0.5 * jr.next_f64());
        }
        self.key = [f64::NAN; 6];
        self.base_weight.iter_mut().for_each(|w| *w = 0.0);
    }

    /// Mode-shape amplitude of mode `k` at strike position `x` (0..1).
    fn shape_gain(&self, k: usize, x: f64) -> f64 {
        let (a, b) = (self.shape_a[k], self.shape_b[k]);
        if a < 0.0 {
            return 1.2; // the air cavity: driven by the whole wall, wherever it is struck
        }
        let y = (x * 1.618 + 0.31).fract();
        match self.built_shape {
            SHAPE_BAR => {
                if b > 0.5 {
                    (a * PI * (x - 0.5)).cos().abs() * 0.6 // torsion: strongest off the centre line
                } else {
                    (a * PI * x).sin().abs() * 0.8 + 0.2 * (a * PI * x).cos().abs()
                }
            }
            SHAPE_PLATE | SHAPE_BOX | SHAPE_SHEET => {
                let m = a % 10.0;
                (m * PI * (0.15 + 0.7 * x)).sin().abs() * (b * PI * (0.15 + 0.7 * y)).sin().abs()
            }
            SHAPE_SHELL | SHAPE_BOTTLE => {
                // Doublet: the two members of a pair are cos(nθ) and sin(nθ) of the strike angle.
                let theta = x * TAU;
                let v = if (b as usize) % 2 == 0 { (a * theta).cos() } else { (a * theta).sin() };
                0.25 + 0.75 * v.abs()
            }
            _ => 0.3 + 0.7 * ((a * 0.7 + b * 1.3) * (x + 0.2)).sin().abs(),
        }
    }

    /// Coefficients of every mode from the current params (frequency, decay, damping, material).
    fn tune(&mut self, p: &[f64], glide_factor: f64) {
        let sr = self.sample_rate;
        let material = (p[MATERIAL].round().max(0.0) as usize).min(MATERIALS.len() - 1);
        let [eta, slope, flat, pitch, _] = MATERIALS[material];
        let f0 = p[FREQUENCY].clamp(20.0, 10_000.0) * pitch * self.detune;
        let decay = p[DECAY].clamp(0.05, 10.0);
        let damping = p[DAMPING].clamp(0.0, 4.0);
        let nyquist = 0.45 * sr;
        for k in 0..self.ratio.len() {
            let f = f0 * self.ratio[k] * glide_factor;
            self.hz[k] = f;
            if f >= nyquist.min(20_000.0) {
                self.radius[k] = 0.0;
                self.cr[k] = 0.0;
                self.ci[k] = 0.0;
                continue;
            }
            let sigma = (6.907_755 / flat + PI * eta * damping * f * (f / 1000.0).powf(slope)) * self.jitter_damp[k] / decay;
            let r = (-sigma / sr).exp();
            let w = TAU * f / sr;
            self.radius[k] = r;
            self.cr[k] = r * w.cos();
            self.ci[k] = r * w.sin();
        }
    }

    /// Plans the strikes of one trigger (first strike plus bounces).
    fn plan(&mut self, p: &[f64]) {
        let sr = self.sample_rate;
        let vary = p[VARY].clamp(0.0, 1.0);
        let bounces = (p[BOUNCES].round().max(0.0) as usize).min(MAX_STRIKES - 1);
        let e = p[RESTITUTION].clamp(0.0, 0.95);
        let force = p[FORCE].clamp(0.0, 1.0);
        let mut t = p[DELAY].clamp(0.0, 2.0);
        let mut gap = p[BOUNCE_TIME].clamp(0.002, 0.5);
        // Each take strikes a little harder or softer (±30 % at full variation): layers of a preset change balance.
        let mut amp = 1.0 + 0.6 * vary * (self.u() - 0.5);
        self.strike_count = 0;
        for i in 0..=bounces {
            if i > 0 {
                t += gap * (1.0 + 0.6 * vary * (self.u() - 0.5));
                gap *= e.max(0.05);
                amp *= e * (1.0 + 0.5 * vary * (self.u() - 0.5));
            }
            self.strikes[i] = Strike { at: (t * sr) as u64, amp, force: force * amp.sqrt() };
            self.strike_count += 1;
        }
        self.next_strike = 0;
        // Another object of the same kind each take: up to ±3 % in pitch at full variation (half a semitone).
        self.detune = 1.0 + 0.06 * vary * (self.u() - 0.5);
        self.planned = true;
    }

    /// Starts strike `s`: new weights, a Hertz pulse.
    fn start_strike(&mut self, s: Strike, p: &[f64]) {
        let sr = self.sample_rate;
        let vary = p[VARY].clamp(0.0, 1.0);
        let x = (p[POSITION].clamp(0.0, 1.0) + 0.8 * vary * (self.u() - 0.5)).rem_euclid(1.0);
        let mut sum = 0.0;
        for k in 0..self.ratio.len() {
            let jitter = 1.0 + vary * 0.8 * (self.u() - 0.5);
            let w = self.shape_gain(k, x) * self.jitter_gain[k] * jitter.max(0.05);
            self.weight[k] = w;
            sum += w * w;
        }
        let norm = 1.0 / sum.max(1e-12).sqrt();
        for w in self.weight.iter_mut() {
            *w *= norm;
        }
        // Hertz: contact time ∝ v^(-1/5); force 0..1 spans about 85× in speed, so 2.4× in τ.
        let material = (p[MATERIAL].round().max(0.0) as usize).min(MATERIALS.len() - 1);
        let contact = p[CONTACT].clamp(0.00002, 0.02) * MATERIALS[material][4] * 2.4f64.powf(0.5 - s.force) * 2f64.powf(vary * (self.u() - 0.5));
        self.pulse_len = ((contact * sr).round() as u32).max(1);
        self.pulse_n = 0;
        self.pulse_amp = s.amp;
        self.pulse_prev = 0.0;
        self.click_amp = s.amp * (1.0 + 0.6 * vary * (self.u() - 0.5));
        self.strike_force = s.force;
        self.env_peak = 0.0;
    }

    fn rattle_setup(&mut self, f0: f64) {
        let sr = self.sample_rate;
        let mut jr = Rng::new(self.seed ^ 0x1b87_3593);
        for k in 0..RATTLE_MODES {
            let f = (f0 * (3.0 + 7.0 * jr.next_f64())).clamp(400.0, 0.4 * sr);
            let t60 = 0.015 + 0.04 * jr.next_f64();
            let r = (-6.907_755 / (t60 * sr)).exp();
            let w = TAU * f / sr;
            self.r_cr[k] = r * w.cos();
            self.r_ci[k] = r * w.sin();
            self.r_gain[k] = 0.4 + 0.6 * jr.next_f64();
        }
    }
}

impl Node for Modal {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, input: (f64, f64), p: &[f64]) -> NodeOut {
        self.build(p);
        if !self.planned {
            self.plan(p);
        }
        let key = [p[FREQUENCY], p[DECAY], p[DAMPING], p[MATERIAL], self.detune, p[SHAPE]];
        let glide = p[GLIDE].clamp(0.0, 0.05) + if self.built_shape == SHAPE_SHEET { SHEET_GLIDE } else { 0.0 };
        let moved = !key.iter().zip(self.key.iter()).all(|(a, b)| (a - b).abs() <= 1e-9 * a.abs().max(1e-9));
        self.retune_count += 1;
        if moved || (glide > 0.0 && self.retune_count >= RETUNE) {
            self.retune_count = 0;
            if moved {
                self.key = key;
                let f0 = p[FREQUENCY].clamp(20.0, 10_000.0);
                self.rattle_setup(f0);
            }
            let rel = if self.env_peak > 0.0 { (self.env / self.env_peak).min(1.0) } else { 0.0 };
            self.tune(p, 1.0 + glide * self.strike_force * rel * rel);
            if self.base_weight.iter().all(|w| *w == 0.0) {
                let mut sum = 0.0;
                for k in 0..self.ratio.len() {
                    let w = self.shape_gain(k, p[POSITION].clamp(0.0, 1.0)) * self.jitter_gain[k];
                    self.base_weight[k] = w;
                    sum += w * w;
                }
                let norm = 1.0 / sum.max(1e-12).sqrt();
                self.base_weight.iter_mut().for_each(|w| *w *= norm);
            }
        }
        // Strikes.
        if p[STRIKE] >= 0.5 && self.next_strike < self.strike_count && self.frame >= self.strikes[self.next_strike].at {
            let s = self.strikes[self.next_strike];
            self.next_strike += 1;
            self.start_strike(s, p);
        }
        self.frame += 1;
        // Force pulse sin^1.5(πn/N), unit area; its derivative is the click.
        let mut force = 0.0;
        let mut click = 0.0;
        if self.pulse_n < self.pulse_len {
            let len = self.pulse_len as f64;
            let ph = PI * (self.pulse_n as f64 + 0.5) / len;
            let v = ph.sin().powf(1.5);
            // ∫ sin^1.5 over [0, π] ≈ 1.748, so the area of v over len samples is ≈ 0.5564·len.
            force = self.pulse_amp * v / (0.5564 * len);
            click = self.click_amp * (v - self.pulse_prev) * len.min(40.0) / 3.0;
            self.pulse_prev = v;
            self.pulse_n += 1;
        } else if self.pulse_prev > 0.0 {
            click = -self.click_amp * self.pulse_prev * self.pulse_len.min(40) as f64 / 3.0;
            self.pulse_prev = 0.0;
        }
        let ext = 0.5 * (input.0 + input.1);
        // An input held at 1 for 1 ms is as strong as one strike.
        let ext = if ext.is_finite() { ext * p[INPUT].clamp(0.0, 4.0) * self.input_scale } else { 0.0 };
        let mut body = 0.0;
        let n = self.ratio.len();
        for k in 0..n {
            if self.radius[k] == 0.0 {
                continue;
            }
            let x = force * self.weight[k] + ext * self.base_weight[k];
            let re = self.cr[k] * self.re[k] - self.ci[k] * self.im[k] + x;
            let im = self.ci[k] * self.re[k] + self.cr[k] * self.im[k];
            self.re[k] = re;
            self.im[k] = im;
            body += im;
        }
        body *= 0.5;
        // Body motion envelope: instant attack, 4 ms release.
        let a = body.abs();
        self.env = if a > self.env { a } else { self.env * 0.99943 + a * 0.00057 };
        if self.pulse_n < self.pulse_len || self.env > self.env_peak {
            self.env_peak = self.env_peak.max(self.env);
        }
        // Rattle: loose parts knock while the body moves more than their play (absolute: force × level).
        let rattle = p[RATTLE].clamp(0.0, 2.0);
        let mut rat = 0.0;
        if rattle > 0.0 {
            let rel = if self.env_peak > 0.0 { self.env / self.env_peak } else { 0.0 };
            let drive = (self.strike_force * rel - 0.2).max(0.0);
            let rate = rattle * 900.0 * drive;
            if rate > 0.0 && self.u() < rate / self.sample_rate {
                self.rattle_pulse = 3;
                self.rattle_amp = (0.3 + 0.7 * self.u()) * drive.min(1.0);
            }
            let x = if self.rattle_pulse > 0 {
                self.rattle_pulse -= 1;
                self.rattle_amp / 3.0
            } else {
                0.0
            };
            for k in 0..RATTLE_MODES {
                let re = self.r_cr[k] * self.r_re[k] - self.r_ci[k] * self.r_im[k] + x;
                let im = self.r_ci[k] * self.r_re[k] + self.r_cr[k] * self.r_im[k];
                self.r_re[k] = re;
                self.r_im[k] = im;
                rat += im * self.r_gain[k];
            }
            rat *= rattle * 0.5;
        }
        let out = (body + click * p[CLICK].clamp(0.0, 2.0) * 0.05 + rat) * p[LEVEL].clamp(0.0, 4.0);
        if !out.is_finite() {
            self.reset();
            return NodeOut { l: 0.0, r: 0.0, value: 0.0 };
        }
        NodeOut { l: out, r: out, value: 0.0 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        if (init.sample_rate - self.sample_rate).abs() > 0.5 || init.seed != self.seed {
            *self = Modal::new(init.sample_rate, init.seed);
        }
        self.build(init.params);
        self.key = [f64::NAN; 6];
        self.base_weight.iter_mut().for_each(|w| *w = 0.0);
    }

    /// Silences the object and re-plans the strikes; the random stream carries on (a new draw per trigger).
    fn reset(&mut self) {
        self.re.iter_mut().for_each(|v| *v = 0.0);
        self.im.iter_mut().for_each(|v| *v = 0.0);
        self.r_re = [0.0; RATTLE_MODES];
        self.r_im = [0.0; RATTLE_MODES];
        self.weight.iter_mut().for_each(|v| *v = 0.0);
        self.pulse_n = 0;
        self.pulse_len = 0;
        self.pulse_prev = 0.0;
        self.rattle_pulse = 0;
        self.env = 0.0;
        self.env_peak = 0.0;
        self.frame = 0;
        self.planned = false;
        self.key = [f64::NAN; 6];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 44_100.0;

    /// shape, material, frequency, modes, decay, damping, contact, force, position, vary, click, rattle, glide,
    /// bounces, bounceTime, restitution, input, strike, delay, level
    fn params(shape: f64, material: f64, contact: f64, vary: f64) -> [f64; 20] {
        [shape, material, 400.0, 48.0, 1.0, 1.0, contact, 0.5, 0.3, vary, 0.0, 0.0, 0.0, 0.0, 0.03, 0.5, 1.0, 1.0, 0.0, 1.0]
    }

    fn render(node: &mut dyn Node, p: &[f64], frames: usize) -> Vec<f64> {
        let c = SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        };
        (0..frames).map(|_| node.process_sample(&c, (0.0, 0.0), p).l).collect()
    }

    fn make(p: &[f64]) -> Box<dyn Node> {
        (DEF.create)(&NodeInit { id: "modal1", seed: 11, sample_rate: SR, params: p, texts: &[], curve: None, inputs: &[], duration: 1.0 })
    }

    /// Energy share above 4 kHz, by a DFT of the first 50 ms.
    fn high_share(x: &[f64]) -> f64 {
        let n = (0.05 * SR) as usize;
        let (mut hi, mut all) = (0.0, 0.0);
        for k in 1..n / 2 {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, v) in x[..n].iter().enumerate() {
                let a = TAU * (k * i) as f64 / n as f64;
                re += v * a.cos();
                im += v * a.sin();
            }
            let e = re * re + im * im;
            all += e;
            if k as f64 * SR / n as f64 > 4_000.0 {
                hi += e;
            }
        }
        hi / all
    }

    #[test]
    fn a_strike_rings_at_the_lowest_mode_and_a_shorter_contact_is_brighter() {
        let soft = params(1.0, 0.0, 0.004, 0.0);
        let out = render(make(&soft).as_mut(), &soft, (0.5 * SR) as usize);
        // The tail rings at the 400 Hz lowest plate mode more than between the modes below it.
        let tail = &out[(0.2 * SR) as usize..];
        let level = |hz: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, v) in tail.iter().enumerate() {
                let a = TAU * hz * i as f64 / SR;
                re += v * a.cos();
                im += v * a.sin();
            }
            (re * re + im * im).sqrt()
        };
        assert!(level(400.0) > 10.0 * level(250.0), "400 Hz {} vs 250 Hz {}", level(400.0), level(250.0));
        // Hertz contact: 40 µs instead of 4 ms puts far more energy above 4 kHz.
        let hard = params(1.0, 0.0, 0.00004, 0.0);
        let bright = render(make(&hard).as_mut(), &hard, (0.1 * SR) as usize);
        assert!(high_share(&bright) > 10.0 * high_share(&out), "{} vs {}", high_share(&bright), high_share(&out));
        assert!(out.iter().chain(&bright).all(|v| v.is_finite()));
    }

    #[test]
    fn every_restart_strikes_again_and_varies() {
        let p = params(3.0, 0.0, 0.0005, 0.8);
        let mut node = make(&p);
        let first = render(node.as_mut(), &p, 4410);
        node.reset();
        let second = render(node.as_mut(), &p, 4410);
        let peak = |x: &[f64]| x.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(peak(&first) > 1e-3 && peak(&second) > 1e-3);
        assert!(first.iter().zip(&second).any(|(a, b)| (a - b).abs() > 1e-4 * peak(&first)));
        // With no variation, a restart replays the same strike exactly.
        let still = params(3.0, 0.0, 0.0005, 0.0);
        let mut node = make(&still);
        let a = render(node.as_mut(), &still, 4410);
        node.reset();
        let b = render(node.as_mut(), &still, 4410);
        let diff = a.iter().zip(&b).fold(0.0f64, |m, (x, y)| m.max((x - y).abs()));
        assert!(diff < 1e-3 * peak(&a), "diff {diff}");
    }
}
