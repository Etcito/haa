//! `reverb` effect node: Sona's legacy acoustic-space reverb.
//! Ported from Sona v11 effects.js (`processEffect`, `acousticSpace`).
//! Params and labels from catalog.js `NODE_TYPES.reverb` and ui-translations.js.
//! Deviations: the shared helper uses f64 delay buffers where Sona uses f32;
//! mute and bypass are applied by the interpreter.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::acoustic_space::AcousticSpace;
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const SIZE: usize = 0;
const MIX: usize = 1;

/// The `reverb` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "reverb",
    category: Category::Effect,
    label_en: "Reverb",
    label_fr: "Réverbération",
    audio_in: true,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "size",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.4,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Space",
            label_fr: "Espace",
            modulatable: false,
            smooth: true,
        },
        ParamDef {
            key: "mix",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.18,
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
    Box::new(Reverb {
        space: AcousticSpace::new(init.sample_rate),
    })
}

struct Reverb {
    space: AcousticSpace,
}

impl Node for Reverb {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let (l, r) = self.space.process(
            input.0,
            input.1,
            params[SIZE],
            0.5,
            params[MIX],
            0.7,
            0.0,
            ctx.live_frames,
            true,
        );
        // sona: the acousticSpace branch returns before processEffect's finite zeroing.
        NodeOut {
            l,
            r,
            value: (l + r) * 0.5,
        }
    }

    fn reset(&mut self) {
        self.space.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const RATE: f64 = 48_000.0;

    fn ctx(frame: u64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: RATE,
            dt: 1.0 / RATE,
            t: 0.0,
            seconds: 0.0,
            duration: 2.0,
            mode: GraphMode::Timeline,
            live_frames: frame,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn make(params: &[f64]) -> Box<dyn Node> {
        let texts = [None, None];
        (DEF.create)(&NodeInit {
            id: "reverb1",
            seed: 0,
            sample_rate: RATE,
            params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 2.0,
        })
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let params = [0.4, 0.18];
        let mut node = make(&params);
        for frame in 0..4800 {
            let out = node.process_sample(&ctx(frame), (0.0, 0.0), &params);
            assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn reverb_tail_decays() {
        let params = [1.0, 1.0];
        let mut node = make(&params);
        let mut early = 0.0_f64;
        let mut late = 0.0_f64;
        for frame in 0..96_000 {
            let x = if frame == 0 { 1.0 } else { 0.0 };
            let out = node.process_sample(&ctx(frame), (x, x), &params);
            let peak = out.l.abs().max(out.r.abs());
            if (9_600..19_200).contains(&frame) {
                early = early.max(peak);
            }
            if (86_400..96_000).contains(&frame) {
                late = late.max(peak);
            }
        }
        assert!(early > 0.0, "reverb tail is silent");
        assert!(
            late < early * 0.01,
            "late peak {late} vs early peak {early}"
        );
    }

    #[test]
    fn outputs_stay_finite_at_every_param_extreme() {
        for size in [0.0, 1.0] {
            for mix in [0.0, 1.0] {
                let params = [size, mix];
                let mut node = make(&params);
                let mut state = 12345u32;
                for frame in 0..48_000 {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    let x = state as f64 / u32::MAX as f64 * 2.0 - 1.0;
                    let out = node.process_sample(&ctx(frame), (x, -x), &params);
                    assert!(out.l.is_finite() && out.r.is_finite() && out.value.is_finite());
                }
                node.reset();
                let out = node.process_sample(&ctx(1), (0.0, 0.0), &params);
                assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
            }
        }
    }
}
