//! `flanger` effect node: stereo flanger, a short swept delay with feedback.
//! HAA-only node (no Sona counterpart).
//!
//! Each channel reads a Hermite-interpolated delay that sweeps from `delay` up to `delay` + 4 ms
//! x `depth` (sine LFO). `feedback` (negative inverts it) deepens the comb; the feedback path is
//! bounded so it never runs away. The right sweep lags by `width` x 180°. `mix` 0.5 gives the
//! deepest notches.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::mod_delay::{ModDelay, lfo_sine};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const RATE: usize = 0;
const DEPTH: usize = 1;
const DELAY: usize = 2;
const FEEDBACK: usize = 3;
const MIX: usize = 4;
const WIDTH: usize = 5;

/// Sweep at `depth = 1`, milliseconds above `delay`.
const SWEEP_MS: f64 = 4.0;
const MAX_SECONDS: f64 = 0.014;

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

/// The `flanger` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "flanger",
    category: Category::Effect,
    label_en: "Flanger",
    label_fr: "Flanger",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        number("rate", 0.02, 5.0, 0.3, Scale::Log, "Hz", "Rate", "Vitesse"),
        number("depth", 0.0, 1.0, 0.7, Scale::Lin, "", "Depth", "Profondeur"),
        number("delay", 0.2, 8.0, 1.0, Scale::Lin, "ms", "Manual delay", "Retard manuel"),
        number("feedback", -0.95, 0.95, 0.6, Scale::Lin, "", "Feedback", "Réinjection"),
        number("mix", 0.0, 1.0, 0.5, Scale::Lin, "", "Mix", "Mélange"),
        number("width", 0.0, 1.0, 0.3, Scale::Lin, "", "Stereo width", "Largeur stéréo"),
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Flanger::new(init.sample_rate))
}

struct Flanger {
    sample_rate: f64,
    phase: f64,
    left: ModDelay,
    right: ModDelay,
}

impl Flanger {
    fn new(sample_rate: f64) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            sample_rate,
            phase: 0.0,
            left: ModDelay::new(MAX_SECONDS, sample_rate),
            right: ModDelay::new(MAX_SECONDS, sample_rate),
        }
    }
}

fn channel(line: &mut ModDelay, x: f64, delay: f64, feedback: f64, mix: f64) -> f64 {
    let wet = line.read(delay);
    line.write(x + (feedback * wet).clamp(-4.0, 4.0));
    x * (1.0 - mix) + wet * mix
}

impl Node for Flanger {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, input: (f64, f64), p: &[f64]) -> NodeOut {
        let sr = self.sample_rate;
        self.phase = (self.phase + p[RATE].clamp(0.02, 5.0) / sr) % 1.0;
        let base_ms = p[DELAY].clamp(0.2, 8.0);
        let sweep_ms = p[DEPTH].clamp(0.0, 1.0) * SWEEP_MS;
        let offset = 0.5 * p[WIDTH].clamp(0.0, 1.0);
        let d_l = (base_ms + sweep_ms * 0.5 * (1.0 + lfo_sine(self.phase))) * 0.001 * sr;
        let d_r = (base_ms + sweep_ms * 0.5 * (1.0 + lfo_sine(self.phase + offset))) * 0.001 * sr;
        let feedback = p[FEEDBACK].clamp(-0.95, 0.95);
        let mix = p[MIX].clamp(0.0, 1.0);
        let x_l = if input.0.is_finite() { input.0 } else { 0.0 };
        let x_r = if input.1.is_finite() { input.1 } else { 0.0 };
        let left = channel(&mut self.left, x_l, d_l, feedback, mix);
        let right = channel(&mut self.right, x_r, d_r, feedback, mix);
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
            *self = Flanger::new(init.sample_rate);
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.left.reset();
        self.right.reset();
    }
}
