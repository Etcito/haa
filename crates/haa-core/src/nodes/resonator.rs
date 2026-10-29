//! `resonator` effect node: six damped, inharmonic two-pole modes that add a body to an excitation,
//! Sona's "Résonateur de matière".
//! Ported from Sona v11 effects.js (`processEffect`, `case 'resonator'`, `biquad`, `runBiquad`,
//! `MODE_RATIOS`, `MODE_GAINS`) and the effect branch of dsp.js `process`.
//! Params and labels from catalog.js `NODE_TYPES.resonator` and ui-translations.js.
//! Deviations:
//! - Defaults are catalog.js's (450 Hz, 0.055 s, 0.45, 0.25); effects.js `EFFECT_PARAMS` gives
//!   600, 0.09, 0.35, 0.4, which Sona's `validateGraph` fills in when a node omits a param.
//! - Sona writes raw coefficients that `dsp::Biquad::set` cannot express, so the stereo two-pole
//!   state lives here as a fixed array of 6, with `runBiquad`'s update order.
//! - `frame` is `ctx.live_frames` (Sona passes `this.totalFrames`).
//! - Mute and bypass are not in this file: the interpreter applies them as dsp.js `process` does before `processEffect`.

use std::f64::consts::TAU;

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const FREQUENCY: usize = 0;
const DECAY: usize = 1;
const COLOR: usize = 2;
const MIX: usize = 3;

const MODES: usize = 6;
const MODE_RATIOS: [f64; MODES] = [1.0, 1.47, 2.13, 2.91, 4.17, 5.43];
const MODE_GAINS: [f64; MODES] = [1.0, 0.68, 0.48, 0.34, 0.21, 0.14];

/// The `resonator` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "resonator",
    category: Category::Effect,
    label_en: "Material resonator",
    label_fr: "Résonateur de matière",
    audio_in: true,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "frequency",
            kind: ParamKind::Number {
                min: 80.0,
                max: 6000.0,
                default: 450.0,
                step: 1.0,
                scale: Scale::Log,
            },
            unit: "Hz",
            label_en: "Fundamental",
            label_fr: "Fondamentale",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "decay",
            kind: ParamKind::Number {
                min: 0.005,
                max: 1.0,
                default: 0.055,
                step: 0.005,
                scale: Scale::Lin,
            },
            unit: "s",
            label_en: "Resonance",
            label_fr: "Résonance",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "color",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.45,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Material",
            label_fr: "Matière",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "mix",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.25,
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
    Box::new(Resonator {
        modes: [Mode::new(); MODES],
        fresh: true,
    })
}

/// Sona's `biquad()` object: raw coefficients plus the stereo transposed direct form II state.
#[derive(Clone, Copy)]
struct Mode {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    l1: f64,
    l2: f64,
    r1: f64,
    r2: f64,
}

impl Mode {
    const fn new() -> Mode {
        Mode {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            l1: 0.0,
            l2: 0.0,
            r1: 0.0,
            r2: 0.0,
        }
    }

    /// Sona's `runBiquad`, returning `(b.left, b.right)`.
    fn run(&mut self, left: f64, right: f64) -> (f64, f64) {
        let l = left * self.b0 + self.l1;
        let r = right * self.b0 + self.r1;
        self.l1 = left * self.b1 - self.a1 * l + self.l2;
        self.l2 = left * self.b2 - self.a2 * l;
        self.r1 = right * self.b1 - self.a1 * r + self.r2;
        self.r2 = right * self.b2 - self.a2 * r;
        (l, r)
    }

    fn clear(&mut self) {
        self.l1 = 0.0;
        self.l2 = 0.0;
        self.r1 = 0.0;
        self.r2 = 0.0;
    }
}

struct Resonator {
    modes: [Mode; MODES],
    fresh: bool,
}

impl Node for Resonator {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let sr = ctx.sample_rate;
        let (input_l, input_r) = input;
        if (ctx.live_frames & 31) == 0 || self.fresh {
            for (i, b) in self.modes.iter_mut().enumerate() {
                let fi = i as f64;
                let ratio =
                    (2.0 * fi + 1.0) * (1.0 - params[COLOR]) + MODE_RATIOS[i] * params[COLOR];
                let f = (params[FREQUENCY] * ratio).min(sr * 0.43);
                let tau = params[DECAY] / (1.0 + fi * 0.22);
                let r = (-1.0 / (tau * sr)).exp();
                b.b0 = 1.0 - r;
                b.b1 = 0.0;
                b.b2 = -(1.0 - r);
                b.a1 = -2.0 * r * (TAU * f / sr).cos();
                b.a2 = r * r;
            }
            self.fresh = false;
        }
        let mut wet_l = 0.0;
        let mut wet_r = 0.0;
        for (i, b) in self.modes.iter_mut().enumerate() {
            let (l, r) = b.run(input_l, input_r);
            wet_l += l * MODE_GAINS[i];
            wet_r += r * MODE_GAINS[i];
        }
        let mix = params[MIX];
        let left = input_l * (1.0 - mix) + wet_l * mix * 1.35;
        let right = input_r * (1.0 - mix) + wet_r * mix * 1.35;
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
        for b in self.modes.iter_mut() {
            b.clear();
        }
        self.fresh = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const RATE: f64 = 48_000.0;
    // frequency, decay, color, mix (DEF order, catalog defaults).
    const DEFAULTS: [f64; 4] = [450.0, 0.055, 0.45, 0.25];

    fn ctx(frame: u64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: RATE,
            dt: 1.0 / RATE,
            t: frame as f64 / RATE,
            seconds: frame as f64 / RATE,
            duration: 1.0,
            mode: GraphMode::Timeline,
            live_frames: frame,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    /// Renders `frames` samples; `input(frame)` gives the stereo input of each one.
    fn render(params: &[f64], frames: u64, input: impl Fn(u64) -> (f64, f64)) -> Vec<NodeOut> {
        let texts: [Option<String>; 4] = [None, None, None, None];
        let mut node = (DEF.create)(&NodeInit {
            id: "resonator1",
            seed: 0,
            sample_rate: RATE,
            params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        });
        (0..frames)
            .map(|frame| node.process_sample(&ctx(frame), input(frame), params))
            .collect()
    }

    #[test]
    fn silence_in_gives_silence_out() {
        for out in render(&DEFAULTS, 4800, |_| (0.0, 0.0)) {
            assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn impulse_rings_then_decays() {
        let params = [450.0, 0.05, 0.45, 0.25];
        let out = render(&params, RATE as u64, |frame| {
            if frame == 0 { (1.0, 1.0) } else { (0.0, 0.0) }
        });
        let ten_ms = (RATE * 0.01) as usize;
        assert!(out[ten_ms].l.abs() > 0.0 && out[ten_ms].r.abs() > 0.0);
        let tail_start = out.len() - (RATE * 0.1) as usize;
        let tail_peak = out[tail_start..]
            .iter()
            .map(|o| o.l.abs().max(o.r.abs()))
            .fold(0.0, f64::max);
        assert!(tail_peak < 1e-4, "tail peak {tail_peak}");
    }

    #[test]
    fn outputs_stay_finite_at_every_param_extreme() {
        for frequency in [80.0, 6000.0] {
            for decay in [0.005, 1.0] {
                for color in [0.0, 1.0] {
                    let params = [frequency, decay, color, 1.0];
                    // A full-scale square at 100 Hz keeps exciting every mode.
                    let out = render(&params, 9600, |frame| {
                        if (frame / 240) % 2 == 0 {
                            (1.0, -1.0)
                        } else {
                            (-1.0, 1.0)
                        }
                    });
                    for o in out {
                        assert!(o.l.is_finite() && o.r.is_finite() && o.value.is_finite());
                    }
                }
            }
        }
    }
}
