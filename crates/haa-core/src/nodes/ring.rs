//! `ring` effect node: ring and amplitude modulation of the input by an internal carrier.
//! HAA-only (not a Sona type).
//! Carrier `c = tanh(s·sin φ)/tanh(s)` with `s = 1 + 9·shape` (sine → near-square).
//! Modulator `m = bias·(0.5 + 0.5·c) + (1 − bias)·c`: `bias = 0` is ring modulation (sum and
//! difference tones, no carrier), `bias = 1` is AM (a tremolo at low rates).
//! Output `x·(1 − depth + depth·m)`. The right carrier runs `width × 90°` ahead.
//! The carrier phase is continuous through frequency changes (no clicks).

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const FREQUENCY: usize = 0;
const DEPTH: usize = 1;
const BIAS: usize = 2;
const SHAPE: usize = 3;
const WIDTH: usize = 4;

const TAU: f64 = std::f64::consts::TAU;

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

const PARAMS: [ParamDef; 5] = [
    number("frequency", 0.1, 8000.0, 30.0, 0.01, Scale::Log, "Hz", "Carrier", "Porteuse"),
    number("depth", 0.0, 1.0, 1.0, 0.01, Scale::Lin, "", "Depth", "Profondeur"),
    number("bias", 0.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Ring → AM", "Anneau → AM"),
    number("shape", 0.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Carrier shape", "Forme de porteuse"),
    number("width", 0.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Stereo phase", "Phase stéréo"),
];

/// The `ring` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "ring",
    category: Category::Effect,
    label_en: "Ring / AM modulator",
    label_fr: "Modulateur en anneau / AM",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let sample_rate = if init.sample_rate.is_finite() && init.sample_rate > 0.0 { init.sample_rate } else { 48_000.0 };
    Box::new(Ring { sample_rate, phase: 0.0 })
}

fn finite_clamp(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    (if v.is_finite() { v } else { fallback }).clamp(min, max)
}

struct Ring {
    sample_rate: f64,
    phase: f64,
}

impl Node for Ring {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, input: (f64, f64), params: &[f64]) -> NodeOut {
        let frequency = finite_clamp(params[FREQUENCY], 30.0, 0.1, 8000.0);
        let depth = finite_clamp(params[DEPTH], 1.0, 0.0, 1.0);
        let bias = finite_clamp(params[BIAS], 0.0, 0.0, 1.0);
        let s = 1.0 + 9.0 * finite_clamp(params[SHAPE], 0.0, 0.0, 1.0);
        let width = finite_clamp(params[WIDTH], 0.0, 0.0, 1.0);
        let norm = 1.0 / s.tanh();
        let modulator = |phase: f64| {
            let c = (s * (TAU * phase).sin()).tanh() * norm;
            let m = bias * (0.5 + 0.5 * c) + (1.0 - bias) * c;
            1.0 - depth + depth * m
        };
        let ml = modulator(self.phase);
        let mr = modulator(self.phase + 0.25 * width);
        let p = self.phase + frequency / self.sample_rate;
        self.phase = p - p.floor();
        let l = if input.0.is_finite() { input.0 * ml } else { 0.0 };
        let r = if input.1.is_finite() { input.1 * mr } else { 0.0 };
        NodeOut { l, r, value: 0.0 }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 48_000.0;

    fn run(p: [f64; 5], input: impl Fn(usize) -> f64, frames: usize) -> Vec<f64> {
        let init = NodeInit { id: "r", seed: 1, sample_rate: SR, params: &[], texts: &[], curve: None, inputs: &[], duration: 1.0 };
        let mut n = (DEF.create)(&init);
        let c = SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        };
        (0..frames).map(|i| { let x = input(i); n.process_sample(&c, (x, x), &p).l }).collect()
    }

    #[test]
    fn ring_of_a_constant_is_the_carrier_and_am_stays_positive() {
        let ring = run([100.0, 1.0, 0.0, 0.0, 0.0], |_| 1.0, 480);
        assert!((ring[120] - 1.0).abs() < 1e-3, "quarter period {}", ring[120]);
        assert!(ring.iter().any(|v| *v < -0.99));
        let am = run([100.0, 1.0, 1.0, 1.0, 0.0], |_| 1.0, 480);
        assert!(am.iter().all(|v| (-1e-12..=1.0 + 1e-12).contains(v)));
    }

    #[test]
    fn silence_gives_silence_and_bad_params_stay_finite() {
        assert!(run([8000.0, 1.0, 0.5, 1.0, 1.0], |_| 0.0, 480).iter().all(|v| *v == 0.0));
        let out = run([f64::NAN, f64::INFINITY, -4.0, 9.0, f64::NAN], |i| (i as f64 * 0.01).sin(), 4_800);
        assert!(out.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }
}
