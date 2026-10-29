//! `random` control node: a smooth random walk between seeded targets.
//! Ported from Sona v11 `dsp.js` (`case 'random'`, `makeState`, `random`) and `rng.rs`
//! (`seedFromId`, xorshift32); params and labels from catalog.js `NODE_TYPES.random`
//! and ui-translations.js.
//! Deviations:
//! - The defaults are catalog.js's rate 2.5 and smooth .8; dsp.js `PARAMS` falls back to
//!   2 and .7 when a node omits the param, as Sona's `validateGraph` does.
//! - Sona keeps the generator state in `state.seed`; here the `Rng` owns it and the node
//!   stores the seed so `reset` can replay the same sequence (Q8).

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;

const RATE: usize = 0;
const DEPTH: usize = 1;
const SMOOTH: usize = 2;

/// The `random` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "random",
    category: Category::Control,
    label_en: "Random modulation",
    label_fr: "Variation organique",
    audio_in: false,
    audio_out: false,
    control_out: true,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "rate",
            kind: ParamKind::Number {
                min: 0.05,
                max: 2000.0,
                default: 2.5,
                step: 0.05,
                scale: Scale::Log,
            },
            unit: "Hz",
            label_en: "Rate",
            label_fr: "Vitesse",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "depth",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Amplitude",
            label_fr: "Amplitude",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "smooth",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.8,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Smoothing",
            label_fr: "Souplesse",
            modulatable: true,
            smooth: true,
        },
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Random::new(init.seed))
}

/// Phase accumulator plus the two targets Sona's `random` interpolates between.
struct Random {
    seed: u32,
    rng: Rng,
    phase: f64,
    start: f64,
    target: f64,
}

impl Random {
    fn new(seed: u32) -> Random {
        let mut rng = Rng::new(seed);
        // sona: makeState draws one target at creation (`randomTarget = random(state)`).
        let target = draw(&mut rng);
        Random {
            seed,
            rng,
            phase: 0.0,
            start: 0.0,
            target,
        }
    }
}

/// Sona's `random(state)`: `seed / 2147483648 - 1`, a value in -1..1.
fn draw(rng: &mut Rng) -> f64 {
    // sona: Sona divides the full u32 state by 2^31 (not 2^32) and subtracts 1.
    f64::from(rng.next_u32()) / 2_147_483_648.0 - 1.0
}

impl Node for Random {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        self.phase += params[RATE] * ctx.dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.start = self.target;
            self.target = draw(&mut self.rng);
        }
        let smooth = params[SMOOTH];
        let u = if smooth <= 0.001 {
            1.0
        } else {
            (self.phase / smooth).min(1.0)
        };
        let blend = u * u * (3.0 - 2.0 * u);
        let value = (self.start + (self.target - self.start) * blend) * params[DEPTH];
        // sona: dsp.js zeroes a non-finite value (`Number.isFinite(value) ? value : 0`).
        let value = if value.is_finite() { value } else { 0.0 };
        // A control returns 0 on its audio channels.
        NodeOut {
            l: 0.0,
            r: 0.0,
            value,
        }
    }

    fn reset(&mut self) {
        // Replay Sona's creation state: phase 0, start 0, a fresh generator and first target.
        self.rng = Rng::new(self.seed);
        self.phase = 0.0;
        self.start = 0.0;
        self.target = draw(&mut self.rng);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const RATE_HZ: f64 = 48_000.0;

    fn ctx() -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: RATE_HZ,
            dt: 1.0 / RATE_HZ,
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

    fn init(seed: u32) -> NodeInit<'static> {
        NodeInit {
            id: "random1",
            seed,
            sample_rate: RATE_HZ,
            params: &[],
            texts: &[],
            curve: None,
            inputs: &[],
            duration: 1.0,
        }
    }

    fn render(seed: u32, params: [f64; 3], frames: usize) -> Vec<f64> {
        let mut node = (DEF.create)(&init(seed));
        let ctx = ctx();
        (0..frames)
            .map(|_| {
                let out = node.process_sample(&ctx, (0.0, 0.0), &params);
                assert_eq!((out.l, out.r), (0.0, 0.0));
                out.value
            })
            .collect()
    }

    #[test]
    fn same_seed_repeats_its_sequence() {
        let params = [30.0, 0.5, 0.8];
        assert_eq!(render(123, params, 4_800), render(123, params, 4_800));
        assert_ne!(render(123, params, 4_800), render(456, params, 4_800));
    }

    #[test]
    fn output_stays_within_depth() {
        for depth in [0.0, 0.25, 0.5, 1.0] {
            let params = [17.0, depth, 0.3];
            for value in render(9, params, 2_400) {
                assert!(value.abs() <= depth + 1e-12, "depth {depth} gave {value}");
            }
        }
    }

    #[test]
    fn extremes_are_finite() {
        let params = [2000.0, 1.0, 0.0];
        assert!(render(7, params, 4_800).iter().all(|v| v.is_finite()));
        let params = [2000.0, 1.0, 1.0];
        assert!(render(7, params, 4_800).iter().all(|v| v.is_finite()));
    }
}
