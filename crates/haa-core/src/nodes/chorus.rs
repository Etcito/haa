//! `chorus` effect node: stereo chorus, one swept delay voice per channel.
//! HAA-only node (no Sona counterpart).
//!
//! Each channel reads its own Hermite-interpolated delay line at `delay` ± a sine sweep of up to
//! 4 ms (`depth`). The right sweep lags the left one by `width` x 180°, so a mono guitar comes out
//! wide. `feedback` recirculates the wet signal; `mix` 0.5 is the classic equal blend.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::mod_delay::{ModDelay, lfo_sine};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const RATE: usize = 0;
const DEPTH: usize = 1;
const DELAY: usize = 2;
const FEEDBACK: usize = 3;
const MIX: usize = 4;
const WIDTH: usize = 5;

/// Sweep at `depth = 1`, milliseconds either side of `delay`.
const SWEEP_MS: f64 = 4.0;
const MAX_SECONDS: f64 = 0.04;

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

/// The `chorus` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "chorus",
    category: Category::Effect,
    label_en: "Chorus",
    label_fr: "Chorus",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        number("rate", 0.05, 8.0, 0.8, Scale::Log, "Hz", "Rate", "Vitesse"),
        number("depth", 0.0, 1.0, 0.5, Scale::Lin, "", "Depth", "Profondeur"),
        number("delay", 5.0, 30.0, 12.0, Scale::Lin, "ms", "Delay", "Retard"),
        number("feedback", 0.0, 0.7, 0.0, Scale::Lin, "", "Feedback", "Réinjection"),
        number("mix", 0.0, 1.0, 0.5, Scale::Lin, "", "Mix", "Mélange"),
        number("width", 0.0, 1.0, 1.0, Scale::Lin, "", "Stereo width", "Largeur stéréo"),
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Chorus::new(init.sample_rate))
}

struct Chorus {
    sample_rate: f64,
    phase: f64,
    left: ModDelay,
    right: ModDelay,
    wet_l: f64,
    wet_r: f64,
}

impl Chorus {
    fn new(sample_rate: f64) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            sample_rate,
            phase: 0.0,
            left: ModDelay::new(MAX_SECONDS, sample_rate),
            right: ModDelay::new(MAX_SECONDS, sample_rate),
            wet_l: 0.0,
            wet_r: 0.0,
        }
    }
}

impl Node for Chorus {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, input: (f64, f64), p: &[f64]) -> NodeOut {
        let sr = self.sample_rate;
        self.phase = (self.phase + p[RATE].clamp(0.05, 8.0) / sr) % 1.0;
        let base_ms = p[DELAY].clamp(5.0, 30.0);
        let sweep_ms = p[DEPTH].clamp(0.0, 1.0) * SWEEP_MS;
        let offset = 0.5 * p[WIDTH].clamp(0.0, 1.0);
        let d_l = (base_ms + sweep_ms * lfo_sine(self.phase)) * 0.001 * sr;
        let d_r = (base_ms + sweep_ms * lfo_sine(self.phase + offset)) * 0.001 * sr;
        let feedback = p[FEEDBACK].clamp(0.0, 0.7);
        let x_l = if input.0.is_finite() { input.0 } else { 0.0 };
        let x_r = if input.1.is_finite() { input.1 } else { 0.0 };
        self.wet_l = self.left.read(d_l);
        self.wet_r = self.right.read(d_r);
        self.left.write(x_l + feedback * self.wet_l);
        self.right.write(x_r + feedback * self.wet_r);
        let mix = p[MIX].clamp(0.0, 1.0);
        let left = x_l * (1.0 - mix) + self.wet_l * mix;
        let right = x_r * (1.0 - mix) + self.wet_r * mix;
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
            *self = Chorus::new(init.sample_rate);
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.left.reset();
        self.right.reset();
        self.wet_l = 0.0;
        self.wet_r = 0.0;
    }
}
