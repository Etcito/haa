#![allow(non_snake_case)]
//! `sampleHold` control node: a sample-and-hold / step LFO. HAA-only (not a Sona type).
//! At `rate` (a phase-continuous clock, so a wired rate such as an engine's cycle rate keeps its
//! phase), it samples `input + noise·r` (r drawn in −1..1 from the node's generator) and holds it;
//! `slew` glides from the held value to the next one (one-pole time constant, 0 = steps).
//! Unwired `input` with `noise = 1` is a random step LFO; a wired `input` is a classic S&H.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;

const INPUT: usize = 0;
const RATE: usize = 1;
const NOISE: usize = 2;
const SLEW: usize = 3;

const PARAMS: [ParamDef; 4] = [
    ParamDef {
        key: "input",
        kind: ParamKind::Number { min: -20000.0, max: 20000.0, default: 0.0, step: 0.001, scale: Scale::Lin },
        unit: "",
        label_en: "Control input",
        label_fr: "Entrée de contrôle",
        modulatable: true,
        smooth: false,
    },
    ParamDef {
        key: "rate",
        kind: ParamKind::Number { min: 0.01, max: 2000.0, default: 4.0, step: 0.01, scale: Scale::Log },
        unit: "Hz",
        label_en: "Rate",
        label_fr: "Cadence",
        modulatable: true,
        smooth: true,
    },
    ParamDef {
        key: "noise",
        kind: ParamKind::Number { min: 0.0, max: 1.0, default: 1.0, step: 0.001, scale: Scale::Lin },
        unit: "",
        label_en: "Random amount",
        label_fr: "Part aléatoire",
        modulatable: true,
        smooth: true,
    },
    ParamDef {
        key: "slew",
        kind: ParamKind::Number { min: 0.0, max: 2.0, default: 0.0, step: 0.0001, scale: Scale::Lin },
        unit: "s",
        label_en: "Slew",
        label_fr: "Glissement",
        modulatable: true,
        smooth: true,
    },
];

/// The `sampleHold` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "sampleHold",
    category: Category::Control,
    label_en: "Sample and hold",
    label_fr: "Échantillonneur-bloqueur",
    audio_in: false,
    audio_out: false,
    control_out: true,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(SampleHold { seed: init.seed, rng: Rng::new(init.seed), phase: 0.0, held: 0.0, value: 0.0, started: false })
}

fn finite_clamp(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    (if v.is_finite() { v } else { fallback }).clamp(min, max)
}

struct SampleHold {
    seed: u32,
    rng: Rng,
    phase: f64,
    held: f64,
    value: f64,
    started: bool,
}

impl Node for SampleHold {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, _input: (f64, f64), params: &[f64]) -> NodeOut {
        let input = finite_clamp(params[INPUT], 0.0, -20000.0, 20000.0);
        let rate = finite_clamp(params[RATE], 4.0, 0.0, 2000.0);
        let noise = finite_clamp(params[NOISE], 1.0, 0.0, 1.0);
        let slew = finite_clamp(params[SLEW], 0.0, 0.0, 2.0);
        self.phase += rate * ctx.dt;
        if !self.started || self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            self.held = input + noise * (self.rng.next_f64() * 2.0 - 1.0);
            if !self.started {
                self.value = self.held;
                self.started = true;
            }
        }
        if slew <= 0.0 {
            self.value = self.held;
        } else {
            self.value += (self.held - self.value) * (1.0 - (-ctx.dt / slew).exp());
        }
        let value = if self.value.is_finite() { self.value } else { 0.0 };
        NodeOut { l: 0.0, r: 0.0, value }
    }

    fn reset(&mut self) {
        *self = SampleHold { seed: self.seed, rng: Rng::new(self.seed), phase: 0.0, held: 0.0, value: 0.0, started: false };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 48_000.0;

    fn run(p: [f64; 4], frames: usize) -> Vec<f64> {
        let init = NodeInit { id: "sh", seed: 7, sample_rate: SR, params: &[], texts: &[], curve: None, inputs: &[], duration: 1.0 };
        let mut n = (DEF.create)(&init);
        let c = SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        };
        (0..frames).map(|_| n.process_sample(&c, (0.0, 0.0), &p).value).collect()
    }

    #[test]
    fn steps_change_only_at_the_rate_and_stay_in_range() {
        let out = run([0.5, 10.0, 0.25, 0.0], 48_000);
        let changes = out.windows(2).filter(|w| w[0] != w[1]).count();
        assert!((9..=10).contains(&changes), "{changes} steps in 1 s at 10 Hz");
        assert!(out.iter().all(|v| (0.25..=0.75).contains(v)));
    }

    #[test]
    fn slew_glides_and_extremes_stay_finite() {
        let out = run([0.0, 20.0, 1.0, 0.05], 9_600);
        let max_jump = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f64::max);
        assert!(max_jump < 0.01, "jump {max_jump}");
        let bad = run([f64::NAN, f64::INFINITY, f64::NAN, -1.0], 4_800);
        assert!(bad.iter().all(|v| v.is_finite()));
    }
}
