//! `comb` effect node: a short fractional feedback delay, Sona's "Retard court / peigne".
//! Ported from Sona v11 simple-effects.js (`createSimpleEffectState`, `processSimpleEffect`).
//! Params and labels from catalog.js `NODE_TYPES.comb` and ui-translations.js.
//! Deviations:
//! - Sona's buffers are Float32Array; these are f64 `DelayLine`s from core-dsp. The read and write
//!   order and the interpolation are Sona's; the buffer is only as long as the delay needs.
//! - Sona's `counter` is a JS number; this one is a u32 that wraps. Its low four bits, which
//!   decide the coefficient refresh, are the same.
//! - A non-finite wet sample is zeroed before it enters the delay line, so one NaN cannot poison
//!   the line and the filter for good. A non-finite output channel is zeroed as in gain.rs.
//! - Mute and bypass are not in this file: the interpreter applies them.

use std::f64::consts::PI;

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::delay_line::DelayLine;
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const TIME: usize = 0;
const FEEDBACK: usize = 1;
const DAMPING: usize = 2;
const MIX: usize = 3;

// sona: simple-effects.js sizes every buffer for 55 ms of delay.
const BUFFER_SECONDS: f64 = 0.055;

/// The `comb` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "comb",
    category: Category::Effect,
    label_en: "Short delay / comb",
    label_fr: "Retard court / peigne",
    audio_in: true,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "time",
            kind: ParamKind::Number {
                min: 0.0002,
                max: 0.05,
                default: 0.004,
                step: 0.0001,
                scale: Scale::Lin,
            },
            unit: "s",
            label_en: "Delay time",
            label_fr: "Retard",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "feedback",
            kind: ParamKind::Number {
                min: -0.95,
                max: 0.95,
                default: 0.55,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Feedback",
            label_fr: "Réinjection",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "damping",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.4,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Damping",
            label_fr: "Absorption",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "mix",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Mix",
            label_fr: "Mélange",
            modulatable: true,
            smooth: true,
        },
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let sample_rate = init.sample_rate;
    Box::new(Comb {
        sample_rate,
        // sona: the longest delay read is the buffer length minus 2, with length = ceil(sr * .055) + 4.
        max_delay: (sample_rate * BUFFER_SECONDS).ceil() + 2.0,
        left: DelayLine::new(BUFFER_SECONDS + 2.0 / sample_rate, sample_rate),
        right: DelayLine::new(BUFFER_SECONDS + 2.0 / sample_rate, sample_rate),
        filter_left: 0.0,
        filter_right: 0.0,
        coefficient: 0.5,
        counter: 0,
    })
}

struct Comb {
    sample_rate: f64,
    max_delay: f64,
    left: DelayLine,
    right: DelayLine,
    filter_left: f64,
    filter_right: f64,
    coefficient: f64,
    counter: u32,
}

impl Node for Comb {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let refresh = self.counter & 15 == 0;
        self.counter = self.counter.wrapping_add(1);
        if refresh {
            let open = 1.0 - params[DAMPING];
            let cutoff = 180.0 + open * open * (16_000.0_f64).min(self.sample_rate * 0.42);
            self.coefficient = 1.0 - (-2.0 * PI * cutoff / self.sample_rate).exp();
        }

        let delay = (params[TIME] * self.sample_rate).clamp(1.0, self.max_delay);
        let delayed_left = self.left.read(delay);
        let delayed_right = self.right.read(delay);
        self.filter_left += (delayed_left - self.filter_left) * self.coefficient;
        self.filter_right += (delayed_right - self.filter_right) * self.coefficient;

        let feedback = params[FEEDBACK].clamp(-0.95, 0.95);
        let normalization = 1.0 - feedback.abs();
        let wet_left = finite_or_zero(input.0 * normalization + self.filter_left * feedback);
        let wet_right = finite_or_zero(input.1 * normalization + self.filter_right * feedback);
        self.left.write(wet_left);
        self.right.write(wet_right);

        let mix = params[MIX];
        let left = finite_or_zero(input.0 + (wet_left - input.0) * mix);
        let right = finite_or_zero(input.1 + (wet_right - input.1) * mix);
        NodeOut {
            l: left,
            r: right,
            value: (left + right) * 0.5,
        }
    }

    fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
        self.filter_left = 0.0;
        self.filter_right = 0.0;
        self.counter = 0;
    }
}

// sona: gain.rs zeroes a non-finite channel; here it also keeps a NaN out of the delay line and the filter.
fn finite_or_zero(x: f64) -> f64 {
    if x.is_finite() { x } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;
    use crate::rng::Rng;

    const RATE: f64 = 48_000.0;

    fn ctx() -> SampleCtx<'static> {
        SampleCtx {
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
        }
    }

    fn create_node(sample_rate: f64, params: &[f64]) -> Box<dyn Node> {
        let texts: Vec<Option<String>> = vec![None; params.len()];
        (DEF.create)(&NodeInit {
            id: "comb1",
            seed: 0,
            sample_rate,
            params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        })
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let params = [0.004, 0.55, 0.4, 0.5];
        let mut node = create_node(RATE, &params);
        for _ in 0..4_800 {
            let out = node.process_sample(&ctx(), (0.0, 0.0), &params);
            assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn comb_feedback_at_maximum_stays_finite() {
        for feedback in [-0.95, 0.95] {
            for time in [0.0002, 0.05] {
                let params = [time, feedback, 0.0, 1.0];
                let mut node = create_node(RATE, &params);
                let mut rng = Rng::new(7);
                for _ in 0..RATE as usize {
                    let x = rng.next_f64() * 2.0 - 1.0;
                    let out = node.process_sample(&ctx(), (x, -x), &params);
                    assert!(out.l.is_finite() && out.r.is_finite() && out.value.is_finite());
                    assert!(out.l.abs() < 10.0 && out.r.abs() < 10.0);
                }
            }
        }
    }

    #[test]
    fn time_is_clamped_to_the_buffer() {
        for rate in [8_000.0, 192_000.0] {
            let params = [0.05, 0.95, 0.0, 1.0];
            let mut node = create_node(rate, &params);
            for i in 0..1_000 {
                let x = if i == 0 { 1.0 } else { 0.0 };
                let out = node.process_sample(&ctx(), (x, x), &params);
                assert!(out.l.is_finite() && out.r.is_finite());
            }
        }
    }
}
