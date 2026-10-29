//! `space` effect node: early reflections and a diffuse tail, Sona's "Espace acoustique".
//! Ported from Sona v11 effects.js (`processEffect`, `case 'space'`) and `acousticSpace`, which
//! lives in dsp/acoustic_space.rs; params and labels from catalog.js `NODE_TYPES.space` and
//! ui-translations.js.
//! Deviations:
//! - The defaults are catalog.js's (size .3, damping .6, mix .13, width .7, distance .15);
//!   effects.js `EFFECT_PARAMS` gives .35, .5, .15, .7, .15, which Sona's `validateGraph` fills in
//!   when a node omits a param.
//! - The effect frame counter is `ctx.live_frames`, Sona's `totalFrames`.
//! - Mute and bypass are not in this file: the interpreter applies them as dsp.js `process` does
//!   before `processEffect`.
//! - The `absorption` and `distance` labels are the same word in catalog.js for both languages;
//!   ui-translations.js has no entry for them.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::acoustic_space::AcousticSpace;
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const SIZE: usize = 0;
const DAMPING: usize = 1;
const MIX: usize = 2;
const WIDTH: usize = 3;
const DISTANCE: usize = 4;

const fn number(
    key: &'static str,
    default: f64,
    label_en: &'static str,
    label_fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number {
            min: 0.0,
            max: 1.0,
            default,
            step: 0.01,
            scale: Scale::Lin,
        },
        unit: "",
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

// catalog.js order.
static PARAMS: [ParamDef; 5] = [
    number("size", 0.3, "Room size", "Volume du lieu"),
    number("damping", 0.6, "Absorption", "Absorption"),
    number("mix", 0.13, "Reflections", "Réflexions"),
    number("width", 0.7, "Width", "Largeur"),
    number("distance", 0.15, "Distance", "Distance"),
];

/// The `space` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "space",
    category: Category::Effect,
    label_en: "Acoustic space",
    label_fr: "Espace acoustique",
    audio_in: true,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Space {
        space: AcousticSpace::new(init.sample_rate),
    })
}

struct Space {
    space: AcousticSpace,
}

impl Node for Space {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let (left, right) = self.space.process(
            input.0,
            input.1,
            params[SIZE],
            params[DAMPING],
            params[MIX],
            params[WIDTH],
            params[DISTANCE],
            ctx.live_frames,
            false,
        );
        // sona: processEffect returns before its `Number.isFinite` zeroing for space and reverb;
        // the voice guard covers a non-finite block.
        NodeOut {
            l: left,
            r: right,
            value: (left + right) * 0.5,
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
    const DEFAULTS: [f64; 5] = [0.3, 0.6, 0.13, 0.7, 0.15];

    fn ctx(live_frames: u64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: RATE,
            dt: 1.0 / RATE,
            t: 0.0,
            seconds: live_frames as f64 / RATE,
            duration: 2.0,
            mode: GraphMode::Timeline,
            live_frames,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn build(params: &[f64]) -> Box<dyn Node> {
        let texts: [Option<String>; 5] = Default::default();
        (DEF.create)(&NodeInit {
            id: "space1",
            seed: 0,
            sample_rate: RATE,
            params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 2.0,
        })
    }

    fn noise(state: &mut u32) -> f64 {
        *state ^= *state << 13;
        *state ^= *state >> 17;
        *state ^= *state << 5;
        (*state as f64 / u32::MAX as f64) * 2.0 - 1.0
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let mut node = build(&DEFAULTS);
        for frame in 0..4800u64 {
            let out = node.process_sample(&ctx(frame), (0.0, 0.0), &DEFAULTS);
            assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn silence_returns_after_the_tail_decays() {
        let mut node = build(&DEFAULTS);
        let frames = (2.0 * RATE) as u64;
        let out: Vec<(f64, f64)> = (0..frames)
            .map(|frame| {
                let input = if frame == 0 { 1.0 } else { 0.0 };
                let out = node.process_sample(&ctx(frame), (input, input), &DEFAULTS);
                (out.l, out.r)
            })
            .collect();
        let early = &out[..9600];
        assert!(early.iter().any(|&(l, r)| l != 0.0 || r != 0.0), "space is silent");
        let tail = &out[out.len() - 480..];
        let peak = tail
            .iter()
            .fold(0.0_f64, |peak, &(l, r)| peak.max(l.abs()).max(r.abs()));
        assert!(peak < 1e-4, "tail peak {peak}");
    }

    #[test]
    fn outputs_stay_finite_at_every_param_extreme() {
        for bits in 0..32u32 {
            let params: [f64; 5] = std::array::from_fn(|i| ((bits >> i) & 1) as f64);
            let mut node = build(&params);
            let mut seed = 0x9e37_79b9u32;
            for frame in 0..2400u64 {
                let l = noise(&mut seed);
                let r = noise(&mut seed);
                let out = node.process_sample(&ctx(frame), (l, r), &params);
                assert!(out.l.is_finite() && out.r.is_finite() && out.value.is_finite());
            }
        }
    }
}
