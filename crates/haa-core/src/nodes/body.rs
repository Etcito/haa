//! `body` effect node: the radiating body of an acoustic instrument (guitar, violin, ukulele, a
//! wooden box), as a modal resonator bank driven by the strings' bridge signal.
//! HAA-only node (no Sona counterpart).
//!
//! Two layers, both linear and time-invariant, so one `body` after a whole string section is the
//! same as one body per string (commuted synthesis):
//! - Low modes: the few strong, separately audible resonances (the Helmholtz air mode, the top
//!   plate's first modes). `modes` lists them as JSON `[[hz, t60_s, gain_db], ...]`; empty uses an
//!   acoustic-guitar set. A mode rings for its T60 and builds up over the same time, so a pluck's
//!   low end swells and thumps after the attack instead of starting at full level.
//! - Wood: a dense cloud of short modes (`woodModes` count, seeded) spread over `woodWidth` octaves
//!   around `woodFreq`, each with a random gain and a T60 near `woodDecay`. It gives every attack
//!   the irregular, noisy formant of a wooden plate: a pluck's upper partials and its finger noise
//!   come out as a dense band instead of a clean harmonic comb.
//!
//! `size` divides every frequency (a bigger body is lower), `decay` multiplies every T60, `low`
//! and `wood` scale the two layers, `direct` is how much of the input passes unchanged (the
//! strings heard directly), `gain` scales the output. Each mode is a two-pole band-pass with unity
//! peak gain, so a mode's `gain_db` is its level at resonance relative to the input. Stereo: each
//! channel has its own state.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;
use core::f64::consts::TAU;

const MODES: usize = 0;
const SIZE: usize = 1;
const DECAY: usize = 2;
const LOW: usize = 3;
const WOOD: usize = 4;
const WOOD_FREQ: usize = 5;
const WOOD_WIDTH: usize = 6;
const WOOD_DECAY: usize = 7;
const DIRECT: usize = 8;
const GAIN: usize = 9;
const WOOD_MODES: usize = 10;

/// Most low modes a `modes` list may hold, and the most wood modes.
const MAX_LOW: usize = 32;
const MAX_WOOD: usize = 96;
/// Acoustic guitar (steel-string, dreadnought-like): air mode, top plate (1,1) pair, (2,1), back,
/// cross-dipole and upper plate modes. Frequency (Hz), T60 (s), gain (dB).
const GUITAR_MODES: [[f64; 3]; 8] = [
    [98.0, 0.35, 0.0],
    [196.0, 0.22, -2.0],
    [225.0, 0.18, -4.0],
    [390.0, 0.12, -6.0],
    [470.0, 0.1, -8.0],
    [580.0, 0.09, -8.0],
    [790.0, 0.07, -10.0],
    [1_050.0, 0.05, -12.0],
];
/// The wood cloud's level calibration: at `wood` 1, broadband noise in comes out of the cloud at
/// about the input's level in its band.
const WOOD_SCALE: f64 = 2.5;

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

/// The `body` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "body",
    category: Category::Effect,
    label_en: "Instrument body",
    label_fr: "Caisse de résonance",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "modes",
            kind: ParamKind::Text { default: "" },
            unit: "",
            label_en: "Low modes [[Hz, T60 s, dB], ...]",
            label_fr: "Modes graves [[Hz, T60 s, dB], ...]",
            modulatable: false,
            smooth: false,
        },
        number("size", 0.5, 2.0, 1.0, 0.01, Scale::Log, "×", "Body size", "Taille de caisse"),
        number("decay", 0.1, 4.0, 1.0, 0.01, Scale::Log, "×", "Resonance time", "Durée de résonance"),
        number("low", 0.0, 4.0, 1.0, 0.01, Scale::Lin, "×", "Low modes", "Modes graves"),
        number("wood", 0.0, 4.0, 0.5, 0.01, Scale::Lin, "×", "Wood (dense upper modes)", "Bois (modes aigus denses)"),
        number("woodFreq", 300.0, 10_000.0, 2_500.0, 1.0, Scale::Log, "Hz", "Wood centre", "Centre du bois"),
        number("woodWidth", 0.5, 6.0, 2.5, 0.1, Scale::Lin, "oct", "Wood width", "Largeur du bois"),
        number("woodDecay", 0.003, 0.5, 0.03, 0.001, Scale::Log, "s", "Wood T60", "T60 du bois"),
        number("direct", 0.0, 1.0, 0.5, 0.01, Scale::Lin, "", "Direct strings", "Cordes directes"),
        number("gain", 0.0, 4.0, 1.0, 0.01, Scale::Lin, "×", "Level", "Niveau"),
        ParamDef {
            key: "woodModes",
            kind: ParamKind::Number { min: 8.0, max: MAX_WOOD as f64, default: 48.0, step: 1.0, scale: Scale::Lin },
            unit: "",
            label_en: "Wood mode count",
            label_fr: "Nombre de modes du bois",
            modulatable: false,
            smooth: false,
        },
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let mut body = Body::new(init.sample_rate, init.seed);
    body.configure(init);
    Box::new(body)
}

/// Parses `[[hz, t60, db], ...]`; an empty or invalid text gives the guitar set, bad rows are dropped.
fn parse_modes(text: &str) -> Vec<[f64; 3]> {
    let parsed: Vec<[f64; 3]> = serde_json::from_str::<Vec<Vec<f64>>>(text)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.len() >= 2 && m.iter().all(|v| v.is_finite()) && m[0] > 10.0 && m[1] > 0.0)
        .map(|m| [m[0].min(20_000.0), m[1].clamp(0.002, 10.0), m.get(2).copied().unwrap_or(0.0).clamp(-60.0, 24.0)])
        .take(MAX_LOW)
        .collect();
    if parsed.is_empty() { GUITAR_MODES.to_vec() } else { parsed }
}

/// One two-pole band-pass mode `g (1 - z^-2) / (1 - a1 z^-1 + a2 z^-2)`, unity peak gain, stereo state.
#[derive(Clone, Copy, Default)]
struct Mode {
    g: f64,
    a1: f64,
    a2: f64,
    weight: f64,
    x: [[f64; 2]; 2],
    y: [[f64; 2]; 2],
}

impl Mode {
    fn set(&mut self, hz: f64, t60: f64, weight: f64, sample_rate: f64) {
        let nyquist = 0.45 * sample_rate;
        if hz >= nyquist {
            self.weight = 0.0;
            return;
        }
        let r = (-6.907_755 / (t60.max(0.001) * sample_rate)).exp();
        self.a1 = 2.0 * r * (TAU * hz / sample_rate).cos();
        self.a2 = r * r;
        self.g = 0.5 * (1.0 - r * r);
        self.weight = weight;
    }

    #[inline]
    fn tick(&mut self, c: usize, input: f64) -> f64 {
        let y = self.g * (input - self.x[c][1]) + self.a1 * self.y[c][0] - self.a2 * self.y[c][1];
        self.x[c][1] = self.x[c][0];
        self.x[c][0] = input;
        self.y[c][1] = self.y[c][0];
        self.y[c][0] = y;
        y * self.weight
    }

    fn clear(&mut self) {
        self.x = [[0.0; 2]; 2];
        self.y = [[0.0; 2]; 2];
    }
}

struct Body {
    sample_rate: f64,
    seed: u32,
    table: Vec<[f64; 3]>,
    low: Vec<Mode>,
    wood: Vec<Mode>,
    /// The wood cloud's fixed random draws: position jitter, gain, T60 factor (one per mode).
    draws: Vec<[f64; 3]>,
    /// The params the coefficients were computed from.
    key: [f64; 6],
}

impl Body {
    fn new(sample_rate: f64, seed: u32) -> Self {
        let mut rng = Rng::new(seed ^ 0x5bd1_e995);
        let draws = (0..MAX_WOOD).map(|_| [rng.next_f64() - 0.5, 0.25 + 0.75 * rng.next_f64(), 0.6 + 0.8 * rng.next_f64()]).collect();
        Body {
            sample_rate: sample_rate.max(1.0),
            seed,
            table: GUITAR_MODES.to_vec(),
            low: Vec::with_capacity(MAX_LOW),
            wood: Vec::with_capacity(MAX_WOOD),
            draws,
            key: [f64::NAN; 6],
        }
    }

    /// Recomputes every mode when size, decay or a wood param moved (smoothed params move slowly).
    fn update(&mut self, p: &[f64]) {
        let key = [p[SIZE], p[DECAY], p[WOOD_FREQ], p[WOOD_WIDTH], p[WOOD_DECAY], p[WOOD_MODES]];
        if key.iter().zip(self.key.iter()).all(|(a, b)| (a - b).abs() <= 1e-6 * a.abs().max(1e-9)) {
            return;
        }
        self.key = key;
        let sr = self.sample_rate;
        let size = p[SIZE].clamp(0.5, 2.0);
        let decay = p[DECAY].clamp(0.1, 4.0);
        self.low.resize(self.table.len(), Mode::default());
        for (mode, [hz, t60, db]) in self.low.iter_mut().zip(self.table.iter()) {
            mode.set(hz / size, t60 * decay, 10f64.powf(db / 20.0), sr);
        }
        let count = (p[WOOD_MODES].round() as usize).clamp(8, MAX_WOOD);
        let centre = p[WOOD_FREQ].clamp(300.0, 10_000.0) / size;
        let width = p[WOOD_WIDTH].clamp(0.5, 6.0);
        let t60 = p[WOOD_DECAY].clamp(0.003, 0.5) * decay;
        self.wood.resize(count, Mode::default());
        // Even log spacing, each mode jittered within its slot; a raised-cosine weight over the band.
        // The cloud's power is normalised by its count and decay so `wood` keeps its level.
        let norm = WOOD_SCALE * (0.03 / t60).sqrt() * (48.0 / count as f64).sqrt();
        for (i, mode) in self.wood.iter_mut().enumerate() {
            let [jitter, gain, tf] = self.draws[i];
            let u = (i as f64 + 0.5 + 0.9 * jitter) / count as f64;
            let hz = centre * 2f64.powf(width * (u - 0.5));
            let window = 0.5 + 0.5 * (TAU * (u - 0.5)).cos();
            mode.set(hz, t60 * tf, norm * gain * window, sr);
        }
    }
}

impl Node for Body {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, input: (f64, f64), p: &[f64]) -> NodeOut {
        self.update(p);
        let low = p[LOW].clamp(0.0, 4.0);
        let wood = p[WOOD].clamp(0.0, 4.0);
        let direct = p[DIRECT].clamp(0.0, 1.0);
        let gain = p[GAIN].clamp(0.0, 4.0);
        let mut out = [0.0f64; 2];
        for (c, x) in [input.0, input.1].into_iter().enumerate() {
            let x = if x.is_finite() { x } else { 0.0 };
            let mut l = 0.0;
            for mode in &mut self.low {
                l += mode.tick(c, x);
            }
            let mut w = 0.0;
            if wood > 0.0 {
                for mode in &mut self.wood {
                    w += mode.tick(c, x);
                }
            }
            out[c] = (direct * x + low * l + wood * w) * gain;
        }
        if !(out[0].is_finite() && out[1].is_finite()) {
            self.reset();
            out = [0.0; 2];
        }
        NodeOut { l: out[0], r: out[1], value: 0.5 * (out[0] + out[1]) }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        if (init.sample_rate - self.sample_rate).abs() > 0.5 || init.seed != self.seed {
            *self = Body::new(init.sample_rate, init.seed);
        }
        self.table = parse_modes(init.texts.get(MODES).and_then(|s| s.as_deref()).unwrap_or(""));
        self.low.clear();
        self.key = [f64::NAN; 6];
    }

    fn reset(&mut self) {
        for mode in self.low.iter_mut().chain(self.wood.iter_mut()) {
            mode.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const RATE: f64 = 44_100.0;

    fn params(wood: f64, direct: f64) -> [f64; 11] {
        [0.0, 1.0, 1.0, 1.0, wood, 2_500.0, 2.5, 0.03, direct, 1.0, 48.0]
    }

    fn render(texts: &[Option<String>], p: &[f64], frames: usize, input: impl Fn(usize) -> f64) -> Vec<f64> {
        let mut node = (DEF.create)(&NodeInit {
            id: "body1",
            seed: 7,
            sample_rate: RATE,
            params: p,
            texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        });
        let ctx = SampleCtx {
            sample_rate: RATE,
            dt: 1.0 / RATE,
            t: 0.0,
            seconds: 0.0,
            duration: 1.0,
            mode: GraphMode::Timeline,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        };
        (0..frames).map(|i| { let x = input(i); node.process_sample(&ctx, (x, x), p).l }).collect()
    }

    /// Level (dB) of `x` at `hz`, by a single DFT bin over the whole buffer.
    fn level_at(x: &[f64], hz: f64) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in x.iter().enumerate() {
            let a = TAU * hz * i as f64 / RATE;
            re += v * a.cos();
            im += v * a.sin();
        }
        20.0 * ((re * re + im * im).sqrt() / x.len() as f64 + 1e-15).log10()
    }

    #[test]
    fn a_listed_mode_rings_at_its_frequency_for_its_t60() {
        let texts = [Some("[[62, 0.5, 0]]".to_string())];
        let p = params(0.0, 0.0);
        let out = render(&texts, &p, (RATE * 1.2) as usize, |i| if i == 0 { 1.0 } else { 0.0 });
        // The ring is at 62 Hz, not at a neighbouring frequency.
        let tail = &out[(0.1 * RATE) as usize..];
        assert!(level_at(tail, 62.0) > level_at(tail, 90.0) + 15.0);
        // Its envelope falls about 60 dB per T60 (12 dB per 0.1 s).
        let rms = |a: f64, b: f64| {
            let s = &out[(a * RATE) as usize..(b * RATE) as usize];
            (s.iter().map(|v| v * v).sum::<f64>() / s.len() as f64).sqrt()
        };
        let drop = 20.0 * (rms(0.1, 0.2) / rms(0.3, 0.4)).log10();
        assert!((drop - 24.0).abs() < 3.0, "drop over 0.2 s {drop} dB");
    }

    #[test]
    fn wood_turns_noise_into_a_band_around_its_centre() {
        let mut rng = Rng::new(3);
        let noise: Vec<f64> = (0..(RATE as usize)).map(|_| rng.next_f64() * 2.0 - 1.0).collect();
        let texts = [Some("[[100, 0.1, -60]]".to_string())];
        let p = params(1.0, 0.0);
        let out = render(&texts, &p, noise.len(), |i| noise[i]);
        let band = |lo: f64, hi: f64| -> f64 {
            let steps = 40;
            (0..steps).map(|k| 10f64.powf(level_at(&out, lo + (hi - lo) * k as f64 / steps as f64) / 10.0)).sum::<f64>()
        };
        let centre = band(2_000.0, 3_200.0);
        let below = band(150.0, 300.0);
        let above = band(12_000.0, 15_000.0);
        assert!(centre > 30.0 * below && centre > 30.0 * above, "centre {centre} below {below} above {above}");
    }

    #[test]
    fn direct_alone_passes_the_input_and_outputs_stay_finite() {
        let texts = [Some("[[62, 0.5, 0], [200, 0.2, -3]]".to_string())];
        let p = [0.0, 1.0, 1.0, 0.0, 0.0, 2_500.0, 2.5, 0.03, 1.0, 1.0, 48.0];
        let out = render(&texts, &p, 100, |i| (i as f64 * 0.1).sin());
        for (i, v) in out.iter().enumerate() {
            assert!((v - (i as f64 * 0.1).sin()).abs() < 1e-12);
        }
        let loud = [0.0, 0.5, 4.0, 4.0, 4.0, 300.0, 6.0, 0.5, 1.0, 4.0, 96.0];
        let out = render(&texts, &loud, 20_000, |i| if (i / 50) % 2 == 0 { 1.0 } else { -1.0 });
        assert!(out.iter().all(|v| v.is_finite()));
    }
}
