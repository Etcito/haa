//! `tremolo` effect node: amplitude modulation, from mono tremolo to auto-pan.
//! HAA-only node (no Sona counterpart).
//!
//! Gain = 1 - `depth` x (1 - lfo) / 2, so the level dips from unity down to 1 - depth. The right
//! LFO lags by `width` x 180°: width 0 is a mono tremolo, width 1 an auto-pan. The square shape is
//! a softened square (tanh) so it never clicks.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use core::f64::consts::TAU;

const RATE: usize = 0;
const DEPTH: usize = 1;
const SHAPE: usize = 2;
const WIDTH: usize = 3;

/// The `tremolo` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "tremolo",
    category: Category::Effect,
    label_en: "Tremolo",
    label_fr: "Trémolo",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "rate",
            kind: ParamKind::Number { min: 0.1, max: 20.0, default: 5.0, step: 0.01, scale: Scale::Log },
            unit: "Hz",
            label_en: "Rate",
            label_fr: "Vitesse",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "depth",
            kind: ParamKind::Number { min: 0.0, max: 1.0, default: 0.5, step: 0.01, scale: Scale::Lin },
            unit: "",
            label_en: "Depth",
            label_fr: "Profondeur",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "shape",
            kind: ParamKind::Enum { values: &["sine", "triangle", "square"], default: 0 },
            unit: "",
            label_en: "Shape",
            label_fr: "Forme",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "width",
            kind: ParamKind::Number { min: 0.0, max: 1.0, default: 0.0, step: 0.01, scale: Scale::Lin },
            unit: "",
            label_en: "Stereo (auto-pan)",
            label_fr: "Stéréo (panoramique auto)",
            modulatable: true,
            smooth: true,
        },
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Tremolo { sample_rate: init.sample_rate.max(1.0), phase: 0.0 })
}

struct Tremolo {
    sample_rate: f64,
    phase: f64,
}

fn shape(phase: f64, kind: usize) -> f64 {
    let phase = phase.rem_euclid(1.0);
    match kind {
        1 => 1.0 - 4.0 * (phase - 0.5).abs(),
        2 => (4.0 * (TAU * phase).sin()).tanh() / 4f64.tanh(),
        _ => (TAU * phase).sin(),
    }
}

impl Node for Tremolo {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, input: (f64, f64), p: &[f64]) -> NodeOut {
        self.phase = (self.phase + p[RATE].clamp(0.1, 20.0) / self.sample_rate) % 1.0;
        let depth = p[DEPTH].clamp(0.0, 1.0);
        let kind = p[SHAPE].round() as usize;
        let offset = 0.5 * p[WIDTH].clamp(0.0, 1.0);
        let gain_l = 1.0 - depth * 0.5 * (1.0 - shape(self.phase, kind));
        let gain_r = 1.0 - depth * 0.5 * (1.0 - shape(self.phase + offset, kind));
        let left = if input.0.is_finite() { input.0 * gain_l } else { 0.0 };
        let right = if input.1.is_finite() { input.1 * gain_r } else { 0.0 };
        NodeOut { l: left, r: right, value: (left + right) * 0.5 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        self.sample_rate = init.sample_rate.max(1.0);
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }
}
