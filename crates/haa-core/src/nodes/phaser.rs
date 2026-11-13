//! `phaser` effect node: stereo phaser, a swept cascade of first-order allpasses with feedback.
//! HAA-only node (no Sona counterpart).
//!
//! The allpass break frequency sweeps `center` x 2^(±2 octaves x `depth`) with a sine LFO; 4, 6 or
//! 8 stages give 2, 3 or 4 notches when mixed with the dry signal (`mix` 0.5 = deepest).
//! `feedback` (below 1, one sample around unity-gain allpasses) sharpens the notches. The right
//! sweep lags by `width` x 180°.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::mod_delay::lfo_sine;
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use core::f64::consts::PI;

const RATE: usize = 0;
const DEPTH: usize = 1;
const CENTER: usize = 2;
const FEEDBACK: usize = 3;
const MIX: usize = 4;
const WIDTH: usize = 5;
const STAGES: usize = 6;

const MAX_STAGES: usize = 8;
/// Sweep at `depth = 1`, octaves either side of `center`.
const SWEEP_OCTAVES: f64 = 2.0;

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    scale: Scale,
    unit: &'static str,
    label_en: &'static str,
    label_fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number { min, max, default, step: 0.01, scale },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

/// The `phaser` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "phaser",
    category: Category::Effect,
    label_en: "Phaser",
    label_fr: "Phaser",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        number("rate", 0.02, 8.0, 0.4, Scale::Log, "Hz", "Rate", "Vitesse"),
        number("depth", 0.0, 1.0, 0.7, Scale::Lin, "", "Depth", "Profondeur"),
        number("center", 100.0, 4_000.0, 700.0, Scale::Log, "Hz", "Centre frequency", "Fréquence centrale"),
        number("feedback", 0.0, 0.9, 0.3, Scale::Lin, "", "Feedback", "Réinjection"),
        number("mix", 0.0, 1.0, 0.5, Scale::Lin, "", "Mix", "Mélange"),
        number("width", 0.0, 1.0, 0.5, Scale::Lin, "", "Stereo width", "Largeur stéréo"),
        ParamDef {
            key: "stages",
            kind: ParamKind::Enum { values: &["4", "6", "8"], default: 0 },
            unit: "",
            label_en: "Stages",
            label_fr: "Étages",
            modulatable: false,
            smooth: false,
        },
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Phaser::new(init.sample_rate))
}

#[derive(Clone, Copy, Default)]
struct Channel {
    x1: [f64; MAX_STAGES],
    y1: [f64; MAX_STAGES],
    last: f64,
}

impl Channel {
    fn process(&mut self, x: f64, a: f64, stages: usize, feedback: f64) -> f64 {
        let mut y = x + feedback * self.last;
        for i in 0..stages {
            let out = a * y + self.x1[i] - a * self.y1[i];
            self.x1[i] = y;
            self.y1[i] = out;
            y = out;
        }
        self.last = y;
        y
    }
}

struct Phaser {
    sample_rate: f64,
    phase: f64,
    left: Channel,
    right: Channel,
}

impl Phaser {
    fn new(sample_rate: f64) -> Self {
        Self {
            sample_rate: sample_rate.max(1.0),
            phase: 0.0,
            left: Channel::default(),
            right: Channel::default(),
        }
    }

    fn coefficient(&self, centre: f64, depth: f64, lfo: f64) -> f64 {
        let f = (centre * (SWEEP_OCTAVES * depth * lfo).exp2()).clamp(20.0, 0.45 * self.sample_rate);
        let t = (PI * f / self.sample_rate).tan();
        (t - 1.0) / (t + 1.0)
    }
}

impl Node for Phaser {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, input: (f64, f64), p: &[f64]) -> NodeOut {
        self.phase = (self.phase + p[RATE].clamp(0.02, 8.0) / self.sample_rate) % 1.0;
        let centre = p[CENTER].clamp(100.0, 4_000.0);
        let depth = p[DEPTH].clamp(0.0, 1.0);
        let offset = 0.5 * p[WIDTH].clamp(0.0, 1.0);
        let a_l = self.coefficient(centre, depth, lfo_sine(self.phase));
        let a_r = self.coefficient(centre, depth, lfo_sine(self.phase + offset));
        let stages = [4, 6, 8][(p[STAGES].round() as usize).min(2)];
        let feedback = p[FEEDBACK].clamp(0.0, 0.9);
        let mix = p[MIX].clamp(0.0, 1.0);
        let x_l = if input.0.is_finite() { input.0 } else { 0.0 };
        let x_r = if input.1.is_finite() { input.1 } else { 0.0 };
        let wet_l = self.left.process(x_l, a_l, stages, feedback);
        let wet_r = self.right.process(x_r, a_r, stages, feedback);
        let left = x_l * (1.0 - mix) + wet_l * mix;
        let right = x_r * (1.0 - mix) + wet_r * mix;
        let (left, right) = if left.is_finite() && right.is_finite() {
            (left, right)
        } else {
            self.reset();
            (0.0, 0.0)
        };
        NodeOut { l: left, r: right, value: (left + right) * 0.5 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        if (init.sample_rate - self.sample_rate).abs() > 0.5 {
            *self = Phaser::new(init.sample_rate);
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.left = Channel::default();
        self.right = Channel::default();
    }
}
