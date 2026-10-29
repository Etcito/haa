//! `output` node: the graph's listening point, Sona's "Sortie audio". The interpreter sums the output nodes.
//! Ported from Sona v11 effects.js (`processEffect`, `case 'output'`), which shares the gain case with `gain.rs`.
//! Params and labels from catalog.js `NODE_TYPES.output` and ui-translations.js.
//! Deviations:
//! - The `gain` (Niveau) default is catalog.js's 0.75; effects.js `EFFECT_PARAMS` gives 0.8, which Sona's
//!   `validateGraph` fills in when a node omits the param.
//! - Mute and bypass are not in this file: the interpreter applies them as dsp.js `process` does before `processEffect`.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const GAIN: usize = 0;

/// The `output` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "output",
    category: Category::Output,
    label_en: "Audio output",
    label_fr: "Sortie audio",
    audio_in: true,
    audio_out: false,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[ParamDef {
        key: "gain",
        kind: ParamKind::Number {
            min: 0.0,
            max: 1.0,
            default: 0.75,
            step: 0.01,
            scale: Scale::Lin,
        },
        unit: "",
        label_en: "Level",
        label_fr: "Niveau",
        modulatable: true,
        smooth: true,
    }],
    create,
};

fn create(_init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Output)
}

struct Output;

impl Node for Output {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let left = input.0 * params[GAIN];
        let right = input.1 * params[GAIN];
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

    fn reset(&mut self) {}
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

    fn process(level: f64, input: (f64, f64)) -> NodeOut {
        let params = [level];
        let texts: [Option<String>; 1] = [None];
        let mut node = (DEF.create)(&NodeInit {
            id: "out1",
            seed: 0,
            sample_rate: RATE,
            params: &params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        });
        node.process_sample(&ctx(), input, &params)
    }

    #[test]
    fn output_at_unity_level_passes_a_sample_unchanged() {
        let out = process(1.0, (0.37, -0.52));
        assert_eq!((out.l, out.r), (0.37, -0.52));
    }

    #[test]
    fn silence_in_gives_silence_out() {
        for level in [0.0, 0.75, 1.0] {
            let out = process(level, (0.0, 0.0));
            assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn outputs_stay_finite_at_every_param_extreme() {
        for level in [0.0, 1.0] {
            for input in [(1.0, -1.0), (f64::MAX, -f64::MAX)] {
                let out = process(level, input);
                assert!(out.l.is_finite() && out.r.is_finite() && out.value.is_finite());
            }
        }
    }
}
