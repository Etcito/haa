//! `delay` effect node: stereo feedback echo, Sona's "Écho".
//! Ported from Sona v11 effects.js (`processEffect`, `case 'delay'`, buffer from `createEffectState`).
//! Params and labels from catalog.js `NODE_TYPES.delay` and ui-translations.js.
//! Deviations:
//! - DelayLine stores f64 where Sona stores f32 delay buffers.
//! - Mute and bypass are applied by the interpreter, not here.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::delay_line::DelayLine;
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const TIME: usize = 0;
const FEEDBACK: usize = 1;
const MIX: usize = 2;

/// Maximum delay in seconds; with the line's +2 it gives Sona's `ceil(sr*1.02)+2` length.
const MAX_SECONDS: f64 = 1.02;

/// The `delay` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "delay",
    category: Category::Effect,
    label_en: "Delay",
    label_fr: "Écho",
    audio_in: true,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "time",
            kind: ParamKind::Number {
                min: 0.01,
                max: 1.0,
                default: 0.24,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "s",
            label_en: "Delay time",
            label_fr: "Délai",
            modulatable: false,
            smooth: true,
        },
        ParamDef {
            key: "feedback",
            kind: ParamKind::Number {
                min: 0.0,
                max: 0.85,
                default: 0.3,
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
            key: "mix",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.2,
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
    Box::new(Delay {
        sample_rate: init.sample_rate,
        left: DelayLine::new(MAX_SECONDS, init.sample_rate),
        right: DelayLine::new(MAX_SECONDS, init.sample_rate),
    })
}

struct Delay {
    sample_rate: f64,
    left: DelayLine,
    right: DelayLine,
}

fn finite(x: f64) -> f64 {
    if x.is_finite() { x } else { 0.0 }
}

impl Node for Delay {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let samples = params[TIME] * self.sample_rate;
        let feedback = params[FEEDBACK];
        let mix = params[MIX];
        let l = self.left.read(samples);
        let r = self.right.read(samples);
        self.left.write((input.0 + l * feedback).clamp(-8.0, 8.0));
        self.right.write((input.1 + r * feedback).clamp(-8.0, 8.0));
        let left = finite(input.0 * (1.0 - mix) + l * mix);
        let right = finite(input.1 * (1.0 - mix) + r * mix);
        NodeOut {
            l: left,
            r: right,
            value: (left + right) * 0.5,
        }
    }

    fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

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

    fn make(params: &[f64]) -> Box<dyn Node> {
        let texts: [Option<String>; 3] = [None, None, None];
        (DEF.create)(&NodeInit {
            id: "delay1",
            seed: 0,
            sample_rate: RATE,
            params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        })
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let params = [0.24, 0.85, 0.5];
        let mut node = make(&params);
        for _ in 0..2000 {
            let out = node.process_sample(&ctx(), (0.0, 0.0), &params);
            assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn impulse_echoes_after_time_and_decays() {
        let params = [0.1, 0.5, 1.0];
        let mut node = make(&params);
        let mut out = Vec::new();
        for i in 0..14_500 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            out.push(node.process_sample(&ctx(), (x, x), &params).l);
        }
        assert_eq!(out[0], 0.0);
        assert!((out[4800] - 1.0).abs() < 1e-9);
        assert!((out[9600] - 0.5).abs() < 1e-9);
        assert!((out[14_400] - 0.25).abs() < 1e-9);
        assert!(out[4801..9600].iter().all(|v| v.abs() < 1e-9));
    }

    #[test]
    fn outputs_stay_finite_at_every_param_extreme() {
        for time in [0.01, 1.0] {
            let params = [time, 0.85, 1.0];
            let mut node = make(&params);
            let mut state = 12345u32;
            for _ in 0..48_000 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let x = state as f64 / u32::MAX as f64 * 2.0 - 1.0;
                let out = node.process_sample(&ctx(), (x, -x), &params);
                assert!(out.l.is_finite() && out.r.is_finite() && out.value.is_finite());
                assert!(out.l.abs() <= 8.0 && out.r.abs() <= 8.0);
            }
        }
    }
}
