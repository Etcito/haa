//! `filter` effect node: Sona's "Filtre", a trapezoidal (TPT) state-variable filter with lowpass, bandpass and highpass outputs.
//! Ported from Sona v11 effects.js (`createEffectState` and `processEffect`, `case 'filter'`).
//! Params and labels from catalog.js `NODE_TYPES.filter` and ui-translations.js.
//! The contract's "RBJ biquad" wording does not apply to `filter`: Sona does not use a biquad here, so this file
//! ports the SVF line by line in f64 and does not use `dsp::biquad`.
//! Deviations:
//! - Defaults are catalog.js's (cutoff 4000, resonance 0.7), not effects.js `EFFECT_PARAMS`.
//! - `mode` arrives as an enum index instead of a string.
//! - Mute and bypass are not in this file: the interpreter applies them as dsp.js `process` does before `processEffect`.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const CUTOFF: usize = 0;
const RESONANCE: usize = 1;
const MODE: usize = 2;

const BANDPASS: usize = 1;
const HIGHPASS: usize = 2;

/// The `filter` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "filter",
    category: Category::Effect,
    label_en: "Filter",
    label_fr: "Filtre",
    audio_in: true,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "cutoff",
            kind: ParamKind::Number {
                min: 10.0,
                max: 18000.0,
                default: 4000.0,
                step: 1.0,
                scale: Scale::Log,
            },
            unit: "Hz",
            label_en: "Cutoff",
            label_fr: "Coupure",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "resonance",
            kind: ParamKind::Number {
                min: 0.1,
                max: 12.0,
                default: 0.7,
                step: 0.1,
                scale: Scale::Lin,
            },
            unit: "Q",
            label_en: "Resonance",
            label_fr: "Résonance",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "mode",
            kind: ParamKind::Enum {
                values: &["lowpass", "bandpass", "highpass"],
                default: 0,
            },
            unit: "",
            label_en: "Mode",
            label_fr: "Mode",
            modulatable: false,
            smooth: false,
        },
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Filter {
        sr: init.sample_rate,
        ic1_l: 0.0,
        ic2_l: 0.0,
        ic1_r: 0.0,
        ic2_r: 0.0,
        a1: 1.0,
        a2: 0.0,
        a3: 0.0,
        k: 1.0,
        fresh: true,
    })
}

struct Filter {
    sr: f64,
    ic1_l: f64,
    ic2_l: f64,
    ic1_r: f64,
    ic2_r: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    k: f64,
    fresh: bool,
}

impl Node for Filter {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        // sona: `frame` is `this.totalFrames`, which HAA carries as `ctx.live_frames`.
        if (ctx.live_frames & 15) == 0 || self.fresh {
            let sr = self.sr;
            let g = (std::f64::consts::PI * (sr * 0.42).min(params[CUTOFF]) / sr).tan();
            let k = 1.0 / params[RESONANCE];
            self.a1 = 1.0 / (1.0 + g * (g + k));
            self.a2 = g * self.a1;
            self.a3 = g * self.a2;
            self.k = k;
            self.fresh = false;
        }
        let (in_l, in_r) = input;
        let v3_l = in_l - self.ic2_l;
        let v1_l = self.a1 * self.ic1_l + self.a2 * v3_l;
        let v2_l = self.ic2_l + self.a2 * self.ic1_l + self.a3 * v3_l;
        let v3_r = in_r - self.ic2_r;
        let v1_r = self.a1 * self.ic1_r + self.a2 * v3_r;
        let v2_r = self.ic2_r + self.a2 * self.ic1_r + self.a3 * v3_r;
        self.ic1_l = (2.0 * v1_l - self.ic1_l).clamp(-64.0, 64.0);
        self.ic2_l = (2.0 * v2_l - self.ic2_l).clamp(-64.0, 64.0);
        self.ic1_r = (2.0 * v1_r - self.ic1_r).clamp(-64.0, 64.0);
        self.ic2_r = (2.0 * v2_r - self.ic2_r).clamp(-64.0, 64.0);
        let mode = params[MODE] as usize;
        let (left, right) = if mode == BANDPASS {
            (v1_l * self.k, v1_r * self.k)
        } else if mode == HIGHPASS {
            (in_l - self.k * v1_l - v2_l, in_r - self.k * v1_r - v2_r)
        } else {
            (v2_l, v2_r)
        };
        // sona: processEffect ends by zeroing a non-finite channel (`Number.isFinite(left) ? left : 0`).
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
        self.ic1_l = 0.0;
        self.ic2_l = 0.0;
        self.ic1_r = 0.0;
        self.ic2_r = 0.0;
        self.fresh = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;
    use crate::rng::Rng;

    const RATE: f64 = 48_000.0;

    fn ctx(frame: u64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: RATE,
            dt: 1.0 / RATE,
            t: 0.0,
            seconds: 0.0,
            duration: 1.0,
            mode: GraphMode::Timeline,
            live_frames: frame,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn process(cutoff: f64, resonance: f64, mode: usize, input: &[(f64, f64)]) -> Vec<NodeOut> {
        let params = [cutoff, resonance, mode as f64];
        let texts: [Option<String>; 3] = [None, None, None];
        let mut node = (DEF.create)(&NodeInit {
            id: "filter1",
            seed: 0,
            sample_rate: RATE,
            params: &params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        });
        input
            .iter()
            .enumerate()
            .map(|(i, &x)| node.process_sample(&ctx(i as u64), x, &params))
            .collect()
    }

    #[test]
    fn silence_in_gives_silence_out() {
        for mode in 0..3 {
            for out in process(4000.0, 0.7, mode, &[(0.0, 0.0); 256]) {
                assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
            }
        }
    }

    #[test]
    fn lowpass_at_minimum_cutoff_attenuates_white_noise() {
        let mut rng = Rng::new(1);
        let noise: Vec<(f64, f64)> = (0..(RATE as usize / 2))
            .map(|_| (rng.next_f64() * 2.0 - 1.0, rng.next_f64() * 2.0 - 1.0))
            .collect();
        let out = process(10.0, 0.7, 0, &noise);
        let rms_in = (noise.iter().map(|x| x.0 * x.0).sum::<f64>() / noise.len() as f64).sqrt();
        let rms_out = (out.iter().map(|o| o.l * o.l).sum::<f64>() / out.len() as f64).sqrt();
        assert!(rms_out < 0.1 * rms_in, "in {rms_in} out {rms_out}");
    }

    #[test]
    fn outputs_stay_finite_at_every_param_extreme() {
        for cutoff in [10.0, 18000.0] {
            for resonance in [0.1, 12.0] {
                for mode in 0..3 {
                    let input: Vec<(f64, f64)> = (0..2000)
                        .map(|i| if i % 2 == 0 { (1.0, -1.0) } else { (-1.0, 1.0) })
                        .chain((0..2000).map(|_| (1.0, 1.0)))
                        .collect();
                    for out in process(cutoff, resonance, mode, &input) {
                        assert!(out.l.is_finite() && out.r.is_finite() && out.value.is_finite());
                    }
                }
            }
        }
    }
}
