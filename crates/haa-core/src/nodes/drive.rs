//! `drive` effect node: tanh saturation with first-order antiderivative antialiasing.
//! Ported from Sona v11 effects.js (`processEffect`, `case 'drive'`, and `logCosh`).
//! Params and labels from catalog.js `NODE_TYPES.drive` and ui-translations.js.
//! Deviations:
//! - The `mix` default is catalog.js's 0.45; effects.js `EFFECT_PARAMS` gives 0.5.
//! - Mute and bypass are applied by the interpreter, as in dsp.js `process`.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const AMOUNT: usize = 0;
const MIX: usize = 1;

/// The `drive` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "drive",
    category: Category::Effect,
    label_en: "Saturation",
    label_fr: "Saturation",
    audio_in: true,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "amount",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.25,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Amount",
            label_fr: "Intensité",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "mix",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.45,
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

fn create(_init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Drive {
        previous_l: 0.0,
        previous_r: 0.0,
    })
}

struct Drive {
    previous_l: f64,
    previous_r: f64,
}

fn log_cosh(x: f64) -> f64 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - std::f64::consts::LN_2
}

impl Node for Drive {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let gain = 1.0 + params[AMOUNT] * 10.0;
        let compensation = 1.0 / (1.0 + params[AMOUNT] * 5.0).sqrt();
        let dx_l = input.0 - self.previous_l;
        let dx_r = input.1 - self.previous_r;
        let wet_l = if dx_l.abs() > 1e-5 {
            (log_cosh(gain * input.0) - log_cosh(gain * self.previous_l)) / (gain * dx_l)
        } else {
            (gain * (input.0 + self.previous_l) * 0.5).tanh()
        } * compensation;
        let wet_r = if dx_r.abs() > 1e-5 {
            (log_cosh(gain * input.1) - log_cosh(gain * self.previous_r)) / (gain * dx_r)
        } else {
            (gain * (input.1 + self.previous_r) * 0.5).tanh()
        } * compensation;
        self.previous_l = input.0;
        self.previous_r = input.1;
        let left = input.0 * (1.0 - params[MIX]) + wet_l * params[MIX];
        let right = input.1 * (1.0 - params[MIX]) + wet_r * params[MIX];
        // sona: processEffect ends by zeroing each non-finite channel independently.
        let left = if left.is_finite() { left } else { 0.0 };
        let right = if right.is_finite() { right } else { 0.0 };
        // sona: dsp.js stores the effect level as (outL + outR) * .5 in values[index].
        NodeOut {
            l: left,
            r: right,
            value: (left + right) * 0.5,
        }
    }

    fn reset(&mut self) {
        self.previous_l = 0.0;
        self.previous_r = 0.0;
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

    fn node(params: &[f64]) -> Box<dyn Node> {
        (DEF.create)(&NodeInit {
            id: "drive1",
            seed: 0,
            sample_rate: RATE,
            params,
            texts: &[None, None],
            curve: None,
            inputs: &[],
            duration: 1.0,
        })
    }

    #[test]
    fn silence_in_gives_silence_out() {
        for amount in [0.0, 0.25, 1.0] {
            for mix in [0.0, 0.45, 1.0] {
                let params = [amount, mix];
                let mut node = node(&params);
                for _ in 0..64 {
                    let out = node.process_sample(&ctx(), (0.0, 0.0), &params);
                    assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
                }
                node.process_sample(&ctx(), (1.0, -0.5), &params);
                node.reset();
                let out = node.process_sample(&ctx(), (0.0, 0.0), &params);
                assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
            }
        }
    }

    #[test]
    fn drive_output_stays_bounded() {
        let params = [1.0, 1.0];
        let mut node = node(&params);
        for frame in 0..(RATE * 0.1) as usize {
            let square = if frame % 48 < 24 { 1.0 } else { -1.0 };
            let sine = (std::f64::consts::TAU * 440.0 * frame as f64 / RATE).sin();
            let out = node.process_sample(&ctx(), (square, sine), &params);
            assert!(out.l.abs() <= 1.0 && out.r.abs() <= 1.0 && out.value.abs() <= 1.0);
        }
    }

    #[test]
    fn outputs_stay_finite_at_every_param_extreme() {
        for amount in [0.0, 1.0] {
            for mix in [0.0, 1.0] {
                let params = [amount, mix];
                let mut node = node(&params);
                // Consecutive steps exercise both sides of the 1e-5 ADAA threshold.
                for input in [
                    0.0, 5e-6, 25e-6, -5e-6, -10e-6, 1.0, -1.0, f64::MAX, -f64::MAX,
                ] {
                    let out = node.process_sample(&ctx(), (input, -input), &params);
                    assert!(out.l.is_finite() && out.r.is_finite() && out.value.is_finite());
                }
                for input in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                    node.reset();
                    let out = node.process_sample(&ctx(), (input, 0.25), &params);
                    assert_eq!(out.l, 0.0);
                    assert!(out.r.is_finite() && out.r > 0.0 && out.value.is_finite());
                }
            }
        }
    }
}
