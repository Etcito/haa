//! `noise` source node: five seeded colours with level-compensated stereo width.
//! Ported from Sona v11 noise.js (`createNoiseState`, `colourSample`, `noiseSample`)
//! and dsp.js (`case 'noise'`); params and labels from catalog.js and translations.
//! Deviations:
//! - Rng owns each generator's seed; the original seed is stored for reset.
//! - Colours arrive as rounded catalog indices instead of strings.
//! - Filter history is initialized, finite f64 state, so JS's `previous* || 0` is redundant.
//! - Channels are returned directly in NodeOut; a source's control value is zero.
//! - Sona's creation-time console message is omitted from the pure core.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;

const COLOR: usize = 0;
const LEVEL: usize = 1;
const WIDTH: usize = 2;

const PINK: usize = 1;
const BROWN: usize = 2;
const BLUE: usize = 3;
const VIOLET: usize = 4;
const SIDE_SEED_XOR: u32 = 0x9e37_79b9;

/// The `noise` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "noise",
    category: Category::Source,
    label_en: "Noise",
    label_fr: "Bruit",
    audio_in: false,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "color",
            kind: ParamKind::Enum {
                values: &["white", "pink", "brown", "blue", "violet"],
                default: PINK,
            },
            unit: "",
            label_en: "Color",
            label_fr: "Couleur",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "level",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.15,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Level",
            label_fr: "Niveau",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "width",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Stereo spread",
            label_fr: "Largeur stéréo",
            modulatable: true,
            smooth: true,
        },
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Noise::new(init.seed))
}

struct Noise {
    seed: u32,
    mid: ColourState,
    side: ColourState,
}

impl Noise {
    fn new(seed: u32) -> Self {
        // sona: generators own the u32 seeds; retain the original seed to restore creation state.
        // sona: omit createNoiseState's console message in the pure core.
        Self {
            seed,
            mid: ColourState::new(seed),
            side: ColourState::new(seed ^ SIDE_SEED_XOR),
        }
    }
}

struct ColourState {
    rng: Rng,
    brown: f64,
    pink0: f64,
    pink1: f64,
    pink2: f64,
    previous_pink: f64,
    previous_white: f64,
}

impl ColourState {
    fn new(seed: u32) -> Self {
        Self {
            rng: Rng::new(seed),
            brown: 0.0,
            pink0: 0.0,
            pink1: 0.0,
            pink2: 0.0,
            previous_pink: 0.0,
            previous_white: 0.0,
        }
    }

    fn colour_sample(&mut self, colour: usize) -> f64 {
        // sona: Sona divides the full u32 state by 2^31 (not 2^32) and subtracts 1.
        let white = f64::from(self.rng.next_u32()) / 2_147_483_648.0 - 1.0;
        // sona: previousPink/previousWhite are initialized and finite, so JS's `|| 0` is redundant.
        let value = match colour {
            BROWN => {
                self.brown = (self.brown + 0.02 * white) / 1.02;
                self.brown * 3.5
            }
            PINK | BLUE => {
                self.pink0 = 0.99765 * self.pink0 + white * 0.099046;
                self.pink1 = 0.963 * self.pink1 + white * 0.2965164;
                self.pink2 = 0.57 * self.pink2 + white * 1.0526913;
                let pink = (self.pink0 + self.pink1 + self.pink2 + white * 0.1848) * 0.12;
                let value = if colour == BLUE {
                    (pink - self.previous_pink) * 4.2
                } else {
                    pink
                };
                self.previous_pink = pink;
                value
            }
            VIOLET => (white - self.previous_white) / std::f64::consts::SQRT_2,
            _ => white,
        };
        self.previous_white = white;
        value
    }
}

impl Node for Noise {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        // sona: the enum is its string in Sona; here it is an index into DEF's values, rounded.
        let colour = params[COLOR].round() as usize;
        let gain = params[LEVEL] * 0.6;
        let mid = self.mid.colour_sample(colour) * gain;
        // sona: params are numeric; explicitly retain the NaN fallback of JS's `params.width || 0`.
        let width = if params[WIDTH].is_nan() {
            0.0
        } else {
            params[WIDTH].clamp(0.0, 1.0)
        };
        let (l, r) = if width == 0.0 {
            (mid, mid)
        } else {
            let side = self.side.colour_sample(colour) * gain * width;
            let normalization = 1.0 / (1.0 + width * width).sqrt();
            ((mid + side) * normalization, (mid - side) * normalization)
        };
        // sona: return channels directly instead of outL/outR state; a source returns value 0.
        // sona: dsp.js guards each channel independently with Number.isFinite.
        NodeOut {
            l: if l.is_finite() { l } else { 0.0 },
            r: if r.is_finite() { r } else { 0.0 },
            value: 0.0,
        }
    }

    fn reset(&mut self) {
        *self = Self::new(self.seed);
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
            id: "noise1",
            seed,
            sample_rate: RATE_HZ,
            params: &[],
            texts: &[],
            curve: None,
            inputs: &[],
            duration: 1.0,
        }
    }

    fn sequence(node: &mut dyn Node, params: &[f64], frames: usize) -> Vec<(f64, f64)> {
        let ctx = ctx();
        (0..frames)
            .map(|_| {
                let out = node.process_sample(&ctx, (0.0, 0.0), params);
                assert_eq!(out.value, 0.0);
                (out.l, out.r)
            })
            .collect()
    }

    #[test]
    fn every_colour_stays_finite_and_bounded_for_one_second() {
        for colour in 0..5 {
            let params = [colour as f64, 1.0, 1.0];
            let mut node = (DEF.create)(&init(123));
            for (l, r) in sequence(&mut *node, &params, 48_000) {
                assert!(l.is_finite() && r.is_finite(), "colour {colour}");
                assert!(
                    l.abs() <= 2.0 && r.abs() <= 2.0,
                    "colour {colour}: {l}, {r}"
                );
            }
            let silent = [colour as f64, 0.0, 1.0];
            let out = node.process_sample(&ctx(), (0.0, 0.0), &silent);
            assert_eq!((out.l, out.r), (0.0, 0.0));
            let non_finite = [colour as f64, f64::NAN, 1.0];
            let out = node.process_sample(&ctx(), (0.0, 0.0), &non_finite);
            assert_eq!((out.l, out.r), (0.0, 0.0));
        }
    }

    #[test]
    fn zero_width_is_mono_and_does_not_advance_the_side_generator() {
        for colour in 0..5 {
            let mut node = Noise::new(42);
            for (l, r) in sequence(&mut node, &[colour as f64, 1.0, 0.0], 4_800) {
                assert_eq!(l, r);
            }
            let mut fresh = ColourState::new(42 ^ SIDE_SEED_XOR);
            assert_eq!(node.side.colour_sample(colour), fresh.colour_sample(colour));
        }
    }

    #[test]
    fn seeds_and_reset_repeat_the_sequence_and_width_is_clamped() {
        for colour in 0..5 {
            for seed in [0, 123, SIDE_SEED_XOR] {
                let params = [colour as f64, 1.0, 1.0];
                let mut node = (DEF.create)(&init(seed));
                let expected = sequence(&mut *node, &params, 256);
                let mut same = (DEF.create)(&init(seed));
                assert_eq!(expected, sequence(&mut *same, &params, 256));
                let mut different = (DEF.create)(&init(seed ^ 456));
                assert_ne!(expected, sequence(&mut *different, &params, 256));
                node.reset();
                assert_eq!(expected, sequence(&mut *node, &params, 256));

                node.reset();
                assert_eq!(
                    expected,
                    sequence(&mut *node, &[colour as f64, 1.0, 5.0], 256)
                );
                node.reset();
                let mono = sequence(&mut *node, &[colour as f64, 1.0, 0.0], 256);
                node.reset();
                assert_eq!(mono, sequence(&mut *node, &[colour as f64, 1.0, -1.0], 256));
            }
        }
    }
}
