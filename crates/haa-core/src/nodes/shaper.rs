//! `shaper` effect node: a continuous waveshaper. HAA-only (not a Sona type).
//! `u = drive·x + bias`; the curve blends a soft clipper `tanh(u)` and a wavefolder `sin(u)`
//! (`fold` 0..1); the static offset `curve(bias)` is subtracted so silence stays silence, and a
//! 10 Hz DC blocker removes what an asymmetric (`bias` ≠ 0) curve adds to a signal. Asymmetry
//! gives even harmonics, drive odd ones, fold dense high ones. No oversampling: high drive on
//! bright input aliases. `mix` blends dry and shaped, `output` is the shaped path's gain.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const DRIVE: usize = 0;
const BIAS: usize = 1;
const FOLD: usize = 2;
const OUTPUT: usize = 3;
const MIX: usize = 4;

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

const PARAMS: [ParamDef; 5] = [
    number("drive", 0.0, 36.0, 6.0, 0.1, "dB", "Drive", "Saturation"),
    number("bias", -1.0, 1.0, 0.0, 0.01, "", "Asymmetry", "Asymétrie"),
    number("fold", 0.0, 1.0, 0.0, 0.01, "", "Fold", "Repli"),
    number("output", -36.0, 12.0, 0.0, 0.1, "dB", "Output", "Sortie"),
    number("mix", 0.0, 1.0, 1.0, 0.01, "", "Mix", "Mélange"),
];

/// The `shaper` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "shaper",
    category: Category::Effect,
    label_en: "Waveshaper",
    label_fr: "Modeleur d’onde",
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
    Box::new(Shaper { coefficient: (-std::f64::consts::TAU * 10.0 / sample_rate).exp(), x1: [0.0; 2], y1: [0.0; 2] })
}

fn finite_clamp(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    (if v.is_finite() { v } else { fallback }).clamp(min, max)
}

fn curve(u: f64, fold: f64) -> f64 {
    (1.0 - fold) * u.tanh() + fold * u.sin()
}

struct Shaper {
    coefficient: f64,
    x1: [f64; 2],
    y1: [f64; 2],
}

impl Node for Shaper {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, input: (f64, f64), params: &[f64]) -> NodeOut {
        let drive = 10f64.powf(finite_clamp(params[DRIVE], 6.0, 0.0, 36.0) / 20.0);
        let bias = finite_clamp(params[BIAS], 0.0, -1.0, 1.0);
        let fold = finite_clamp(params[FOLD], 0.0, 0.0, 1.0);
        let output = 10f64.powf(finite_clamp(params[OUTPUT], 0.0, -36.0, 12.0) / 20.0);
        let mix = finite_clamp(params[MIX], 1.0, 0.0, 1.0);
        let rest = curve(bias, fold);
        let mut out = [0.0; 2];
        for (c, x) in [input.0, input.1].into_iter().enumerate() {
            let x = if x.is_finite() { x } else { 0.0 };
            let shaped = curve(drive * x + bias, fold) - rest;
            // DC blocker: y = x − x₋₁ + R·y₋₁.
            let y = shaped - self.x1[c] + self.coefficient * self.y1[c];
            self.x1[c] = shaped;
            self.y1[c] = y;
            out[c] = x * (1.0 - mix) + y * output * mix;
        }
        NodeOut { l: out[0], r: out[1], value: 0.0 }
    }

    fn reset(&mut self) {
        self.x1 = [0.0; 2];
        self.y1 = [0.0; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 48_000.0;

    fn run(p: [f64; 5], input: impl Fn(usize) -> f64, frames: usize) -> Vec<f64> {
        let init = NodeInit { id: "s", seed: 1, sample_rate: SR, params: &[], texts: &[], curve: None, inputs: &[], duration: 1.0 };
        let mut n = (DEF.create)(&init);
        let c = SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        };
        (0..frames).map(|i| { let x = input(i); n.process_sample(&c, (x, x), &p).l }).collect()
    }

    #[test]
    fn silence_gives_silence_at_any_bias() {
        for bias in [-1.0, 0.3, 1.0] {
            assert!(run([36.0, bias, 0.5, 12.0, 1.0], |_| 0.0, 960).iter().all(|v| *v == 0.0));
        }
    }

    #[test]
    fn output_is_bounded_and_extremes_stay_finite() {
        let sine = |i: usize| (std::f64::consts::TAU * 220.0 * i as f64 / SR).sin() * 4.0;
        for p in [[36.0, 1.0, 1.0, 12.0, 1.0], [0.0, -1.0, 0.0, -36.0, 0.0], [f64::NAN, 9.0, -9.0, f64::INFINITY, 0.5]] {
            let out = run(p, sine, 9_600);
            assert!(out.iter().all(|v| v.is_finite() && v.abs() < 20.0), "{p:?}");
        }
        let out = run([24.0, 0.0, 0.0, 0.0, 1.0], sine, 9_600);
        assert!(out[4_800..].iter().all(|v| v.abs() <= 2.05));
    }
}
