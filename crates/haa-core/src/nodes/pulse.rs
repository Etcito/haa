//! `pulse` source node: a periodic, per-sample-integrated pulse train, Sona's "Train d'impulsions".
//! Ported from Sona v11 `primitives.js` (`createPulseState`, `random`, `startCycle`, `updateWidth`,
//! `area`, `pulseSample`) and the `case 'pulse'` line of `dsp.js`; params and labels from
//! catalog.js `NODE_TYPES.pulse` and ui-translations.js.
//! Deviations:
//! - The generator is `rng::Rng`; Sona keeps the xorshift state in `state.seed`, so `reset`
//!   rebuilds the `Rng` from the stored seed.
//! - The `shape` enum arrives as an index in `params` (1 = exponential) instead of a string.
//! - The `counter++ & 15` counter is a u32 with wrapping add, as JS bit-ands it as an int32.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;

const FREQUENCY: usize = 0;
const WIDTH: usize = 1;
const PHASE: usize = 2;
const JITTER: usize = 3;
const SHIMMER: usize = 4;
const LEVEL: usize = 5;
const SHAPE: usize = 6;

const TAU: f64 = std::f64::consts::TAU;

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
    unit: &'static str,
    label_en: &'static str,
    label_fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number { min, max, default, step, scale: Scale::Lin },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

// catalog.js order.
const PULSE_PARAMS: [ParamDef; 7] = [
    number("frequency", 0.0, 4000.0, 100.0, 0.1, "Hz", "Frequency", "Fréquence"),
    number("width", 0.002, 0.95, 0.08, 0.001, "cycle", "Width", "Largeur"),
    number("phase", 0.0, 1.0, 0.0, 0.01, "cycle", "Phase", "Phase"),
    number("jitter", 0.0, 0.25, 0.02, 0.001, "cycle", "Timing variation", "Écart de temps"),
    number("shimmer", 0.0, 1.0, 0.05, 0.01, "", "Amplitude variation", "Écart d’amplitude"),
    number("level", 0.0, 1.0, 0.5, 0.01, "", "Level", "Niveau"),
    ParamDef {
        key: "shape",
        kind: ParamKind::Enum { values: &["rounded", "exponential"], default: 0 },
        unit: "",
        label_en: "Shape",
        label_fr: "Forme",
        modulatable: false,
        smooth: false,
    },
];

/// The `pulse` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "pulse",
    category: Category::Source,
    label_en: "Pulse train",
    label_fr: "Train d’impulsions",
    audio_in: false,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &PULSE_PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Pulse::new(init.seed, init.sample_rate))
}

/// Sona's `clamp`: `Math.min(max, Math.max(min, value))`.
fn clamp(value: f64, min: f64, max: f64) -> f64 {
    max.min(min.max(value))
}

/// `clamp(Number.isFinite(x) ? x : fallback, min, max)`.
fn finite_clamp(value: f64, fallback: f64, min: f64, max: f64) -> f64 {
    clamp(if value.is_finite() { value } else { fallback }, min, max)
}

/// Sona's `createPulseState` fields.
struct Pulse {
    seed: u32,
    rng: Rng,
    sample_rate: f64,
    phase: f64,
    phase_setting: f64,
    initialized: bool,
    counter: u32,
    delay: f64,
    amplitude: f64,
    dc: f64,
    dc_coefficient: f64,
    width: f64,
    attack: f64,
    decay_end: f64,
    attack_end: f64,
    decay_scale: f64,
    attack_scale: f64,
    normalization: f64,
}

impl Pulse {
    fn new(seed: u32, sample_rate: f64) -> Pulse {
        // sona: `Number(sampleRate) || 48000`, then clamped; NaN and 0 take the fallback.
        let sr = clamp(if sample_rate.is_finite() && sample_rate != 0.0 { sample_rate } else { 48000.0 }, 8000.0, 192000.0);
        Pulse {
            seed,
            rng: Rng::new(seed),
            sample_rate: sr,
            phase: 0.0,
            phase_setting: 0.0,
            initialized: false,
            counter: 0,
            delay: 0.0,
            amplitude: 1.0,
            dc: 0.0,
            dc_coefficient: 1.0 - (-TAU * 8.0 / sr).exp(),
            width: -1.0,
            attack: 0.0,
            decay_end: 0.0,
            attack_end: 0.0,
            decay_scale: 1.0,
            attack_scale: 1.0,
            normalization: 1.0,
        }
    }

    fn start_cycle(&mut self, jitter: f64, shimmer: f64) {
        // Timing varies inside a fixed cycle and never accumulates into pitch drift.
        self.delay = self.rng.next_f64() * jitter;
        self.amplitude = 1.0 + (self.rng.next_f64() * 2.0 - 1.0) * shimmer;
    }

    fn update_width(&mut self, width: f64) {
        self.width = width;
        self.attack = width * 0.08;
        self.decay_end = (-1.0 / width).exp();
        self.attack_end = (-1.0 / self.attack).exp();
        self.decay_scale = 1.0 / (1.0 - self.decay_end);
        self.attack_scale = 1.0 / (1.0 - self.attack_end);
        let peak_position = (width * (1.0 - self.decay_end) / (self.attack * (1.0 - self.attack_end))).ln()
            / (1.0 / self.attack - 1.0 / width);
        let peak = ((-peak_position / width).exp() - self.decay_end) * self.decay_scale
            - ((-peak_position / self.attack).exp() - self.attack_end) * self.attack_scale;
        self.normalization = 1.0 / 0.0001_f64.max(peak);
    }

    /// Integral from zero to `phase`; each shape is zero at its cycle boundaries.
    fn area(&self, phase: f64, exponential: bool) -> f64 {
        if phase <= self.delay {
            return 0.0;
        }
        let scale = 1.0 - self.delay;
        let position = 1.0_f64.min((phase - self.delay) / scale);
        let integral = if exponential {
            let i = (self.width * (1.0 - (-position / self.width).exp()) - position * self.decay_end) * self.decay_scale
                - (self.attack * (1.0 - (-position / self.attack).exp()) - position * self.attack_end) * self.attack_scale;
            i * self.normalization
        } else if position >= self.width {
            self.width * 0.5
        } else {
            position * 0.5 - self.width * (TAU * position / self.width).sin() / (2.0 * TAU)
        };
        integral * scale * self.amplitude
    }

    fn sample(&mut self, params: &[f64]) -> f64 {
        let frequency = finite_clamp(params[FREQUENCY], 100.0, 0.0, 4000.0_f64.min(self.sample_rate * 0.5));
        let width = finite_clamp(params[WIDTH], 0.08, 0.002, 0.95);
        let phase_setting = finite_clamp(params[PHASE], 0.0, 0.0, 1.0);
        let jitter = finite_clamp(params[JITTER], 0.02, 0.0, 0.25);
        let shimmer = finite_clamp(params[SHIMMER], 0.05, 0.0, 1.0);
        let level = finite_clamp(params[LEVEL], 0.5, 0.0, 1.0);
        let exponential = params[SHAPE].round() == 1.0;
        if !self.initialized {
            self.phase = phase_setting % 1.0;
            self.phase_setting = phase_setting;
            self.initialized = true;
            self.update_width(width);
            self.start_cycle(jitter, shimmer);
        } else if phase_setting != self.phase_setting {
            self.phase += phase_setting - self.phase_setting;
            self.phase -= self.phase.floor();
            self.phase_setting = phase_setting;
        }
        let tick = self.counter & 15;
        self.counter = self.counter.wrapping_add(1);
        if tick == 0 && self.width != width {
            self.update_width(width);
        }

        // A zero frequency stops the sequence, keeps the phase and discards DC slowly.
        if frequency == 0.0 {
            self.dc *= 1.0 - self.dc_coefficient;
            return 0.0;
        }

        let step = frequency / self.sample_rate;
        let end = self.phase + step;
        let mut integrated;
        if end < 1.0 {
            integrated = self.area(end, exponential) - self.area(self.phase, exponential);
            self.phase = end;
        } else {
            integrated = self.area(1.0, exponential) - self.area(self.phase, exponential);
            self.start_cycle(jitter, shimmer);
            self.phase = end - 1.0;
            integrated += self.area(self.phase, exponential);
        }
        let sample = integrated / step;
        self.dc += (sample - self.dc) * self.dc_coefficient;
        (sample - self.dc) * level
    }
}

impl Node for Pulse {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, _input: (f64, f64), params: &[f64]) -> NodeOut {
        let v = self.sample(params);
        // sona: dsp.js zeroes a non-finite value (`Number.isFinite(value) ? value : 0`).
        let v = if v.is_finite() { v } else { 0.0 };
        NodeOut { l: v, r: v, value: 0.0 }
    }

    fn reset(&mut self) {
        // Restore the creation state: uninitialized, generator reseeded.
        *self = Pulse::new(self.seed, self.sample_rate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 48_000.0;

    fn ctx() -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        }
    }

    fn node() -> Box<dyn Node> {
        let init = NodeInit {
            id: "pulse1", seed: 5, sample_rate: SR, params: &[], texts: &[], curve: None, inputs: &[], duration: 1.0,
        };
        (DEF.create)(&init)
    }

    // frequency, width, phase, jitter, shimmer, level, shape
    fn run(node: &mut dyn Node, p: [f64; 7], frames: usize) -> Vec<f64> {
        let ctx = ctx();
        (0..frames).map(|_| node.process_sample(&ctx, (0.0, 0.0), &p).l).collect()
    }

    const BASE: [f64; 7] = [100.0, 0.08, 0.0, 0.02, 0.05, 0.5, 0.0];

    #[test]
    fn frequency_zero_is_silent_and_restarts_finite() {
        let mut n = node();
        assert!(run(&mut *n, [0.0, 0.08, 0.0, 0.02, 0.05, 0.5, 0.0], 480).iter().all(|v| *v == 0.0));
        let mut n = node();
        let a = run(&mut *n, [50.0, 0.08, 0.0, 0.02, 0.05, 0.5, 1.0], 4_800);
        assert!(a.iter().any(|v| *v != 0.0));
        let z = run(&mut *n, [0.0, 0.08, 0.0, 0.02, 0.05, 0.5, 1.0], 4_800);
        assert!(z.iter().all(|v| *v == 0.0));
        let b = run(&mut *n, [50.0, 0.08, 0.0, 0.02, 0.05, 0.5, 1.0], 4_800);
        assert!(b.iter().any(|v| *v != 0.0));
        assert!(a.iter().chain(&b).all(|v| v.is_finite()));
    }

    #[test]
    fn zero_jitter_and_shimmer_is_periodic() {
        for shape in [0.0, 1.0] {
            let mut n = node();
            let out = run(&mut *n, [100.0, 0.08, 0.0, 0.0, 0.0, 0.5, shape], 48_000);
            for i in 40_000..47_000 {
                assert!((out[i] - out[i + 480]).abs() < 1e-9, "shape {shape} sample {i}");
            }
        }
    }

    #[test]
    fn extremes_and_bad_params_stay_finite() {
        for shape in [0.0, 1.0] {
            for width in [0.002, 0.95] {
                for frequency in [0.1, 4000.0] {
                    let mut n = node();
                    let out = run(&mut *n, [frequency, width, 0.5, 0.25, 1.0, 1.0, shape], 4_800);
                    assert!(out.iter().all(|v| v.is_finite()), "{shape} {width} {frequency}");
                }
            }
        }
        let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, f64::INFINITY, f64::NAN, 0.0];
        assert_eq!(run(&mut *node(), bad, 2_400), run(&mut *node(), BASE, 2_400));
        let wild = [1e9, 50.0, -3.0, 9.0, 9.0, 9.0, 1.0];
        let tame = [4000.0, 0.95, 0.0, 0.25, 1.0, 1.0, 1.0];
        assert_eq!(run(&mut *node(), wild, 2_400), run(&mut *node(), tame, 2_400));
    }
}
