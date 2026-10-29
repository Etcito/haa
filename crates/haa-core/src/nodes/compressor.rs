//! `compressor` effect node: a soft-knee peak compressor, Sona's "Compresseur".
//! Ported from Sona v11 effects.js (`createEffectState`, `case 'compressor'`) and `dbToGain`;
//! params and labels from catalog.js `NODE_TYPES.compressor` and ui-translations.js.
//! Deviations:
//! - The `attack`, `release` and `makeup` defaults are catalog.js's (.015, .15, 3); effects.js
//!   `EFFECT_PARAMS` gives .008, .12, 2, which Sona's `validateGraph` fills in when a node omits them.
//! - Mute and bypass are not in this file: the interpreter applies them as dsp.js `process` does
//!   before `processEffect`.
//! - The effect frame counter is `ctx.live_frames`, Sona's `totalFrames`.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const THRESHOLD: usize = 0;
const RATIO: usize = 1;
const ATTACK: usize = 2;
const RELEASE: usize = 3;
const MAKEUP: usize = 4;

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
    unit: &'static str,
    label_en: &'static str,
    label_fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number {
            min,
            max,
            default,
            step,
            scale: Scale::Lin,
        },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

// catalog.js order.
static PARAMS: [ParamDef; 5] = [
    number(
        "threshold",
        -48.0,
        0.0,
        -20.0,
        0.5,
        "dB",
        "Threshold",
        "Seuil",
    ),
    number("ratio", 1.0, 12.0, 2.5, 0.1, ":1", "Ratio", "Ratio"),
    number("attack", 0.001, 0.2, 0.015, 0.001, "s", "Attack", "Attaque"),
    number("release", 0.02, 1.0, 0.15, 0.01, "s", "Release", "Retour"),
    number("makeup", 0.0, 18.0, 3.0, 0.5, "dB", "Gain", "Gain"),
];

/// The `compressor` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "compressor",
    category: Category::Effect,
    label_en: "Compressor",
    label_fr: "Compresseur",
    audio_in: true,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &PARAMS,
    create,
};

fn db_to_gain(value: f64) -> f64 {
    10f64.powf(value / 20.0)
}

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Compressor {
        sample_rate: init.sample_rate,
        envelope: 0.0,
        compression: 1.0,
        target_gain: 1.0,
        attack: 0.0,
        release: 0.0,
        makeup: 1.0,
        fresh: true,
    })
}

struct Compressor {
    sample_rate: f64,
    envelope: f64,
    compression: f64,
    target_gain: f64,
    attack: f64,
    release: f64,
    makeup: f64,
    fresh: bool,
}

impl Compressor {
    fn reset_state(&mut self) {
        // createEffectState: envelope 0, compression 1, targetGain 1, attack/release 0, makeup 1.
        self.envelope = 0.0;
        self.compression = 1.0;
        self.target_gain = 1.0;
        self.attack = 0.0;
        self.release = 0.0;
        self.makeup = 1.0;
        self.fresh = true;
    }
}

impl Node for Compressor {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        if (ctx.live_frames & 31) == 0 || self.fresh {
            self.attack = (-1.0 / (params[ATTACK] * self.sample_rate)).exp();
            self.release = (-1.0 / (params[RELEASE] * self.sample_rate)).exp();
            self.makeup = db_to_gain(params[MAKEUP]);
            self.fresh = false;
        }
        let peak = input.0.abs().max(input.1.abs());
        let coef = if peak > self.envelope {
            self.attack
        } else {
            self.release
        };
        self.envelope = peak + coef * (self.envelope - peak);
        if (ctx.live_frames & 7) == 0 {
            let over = 20.0 * self.envelope.max(1e-9).log10() - params[THRESHOLD];
            let knee = 6.0;
            let reduction = if over <= -knee * 0.5 {
                0.0
            } else if over >= knee * 0.5 {
                (1.0 - 1.0 / params[RATIO]) * over
            } else {
                (1.0 - 1.0 / params[RATIO]) * (over + knee * 0.5).powi(2) / (2.0 * knee)
            };
            self.target_gain = db_to_gain(-reduction);
        }
        self.compression += (self.target_gain - self.compression) * 0.015;
        let gain = self.compression * self.makeup;
        let left = input.0 * gain;
        let right = input.1 * gain;
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
        self.reset_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;
    use crate::rng::Rng;
    use std::f64::consts::TAU;

    const RATE: f64 = 48_000.0;
    // threshold, ratio, attack, release, makeup (DEF order).
    const DEFAULTS: [f64; 5] = [-20.0, 2.5, 0.015, 0.15, 3.0];

    fn make_node(params: &[f64]) -> Box<dyn Node> {
        (DEF.create)(&NodeInit {
            id: "compressor1",
            seed: 0,
            sample_rate: RATE,
            params,
            texts: &[],
            curve: None,
            inputs: &[],
            duration: 1.0,
        })
    }

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

    #[test]
    fn silence_in_gives_silence_out() {
        let mut params = DEFAULTS;
        for threshold in [-48.0, 0.0] {
            params[THRESHOLD] = threshold;
            let mut node = make_node(&params);
            for frame in 0..1024 {
                let out = node.process_sample(&ctx(frame), (0.0, 0.0), &params);
                assert_eq!(
                    (out.l, out.r, out.value),
                    (0.0, 0.0, 0.0),
                    "silence at frame {frame}, threshold {threshold}"
                );
            }
            node.reset();
            let out = node.process_sample(&ctx(0), (0.0, 0.0), &params);
            assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn compressor_never_boosts_above_makeup() {
        // 4 thresholds x 3 ratios x (2000 noise + 2000 sine) frames = 1 s of audio.
        for threshold in [-48.0, -30.0, -12.0, 0.0] {
            for ratio in [1.0, 2.5, 12.0] {
                let mut params = DEFAULTS;
                params[THRESHOLD] = threshold;
                params[RATIO] = ratio;
                let makeup = db_to_gain(params[MAKEUP]);
                let mut node = make_node(&params);
                let mut rng = Rng::new(0x5eed);
                for frame in 0..4000u64 {
                    let input = if frame < 2000 {
                        (rng.next_f64() * 2.0 - 1.0, rng.next_f64() * 2.0 - 1.0)
                    } else {
                        let t = frame as f64 / RATE;
                        let s = (TAU * 220.0 * t).sin() * 0.9 + (TAU * 3300.0 * t).sin() * 0.1;
                        (s, -s)
                    };
                    let out = node.process_sample(&ctx(frame), input, &params);
                    for (level_in, level_out) in [
                        (input.0, out.l),
                        (input.1, out.r),
                        ((input.0 + input.1) * 0.5, out.value),
                    ] {
                        assert!(
                            level_out.abs() <= level_in.abs() * makeup + 1e-12,
                            "boost beyond makeup: in {level_in}, out {level_out}, threshold {threshold}, ratio {ratio}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn loud_input_is_reduced() {
        let mut params = DEFAULTS;
        params[THRESHOLD] = -30.0;
        params[RATIO] = 12.0;
        let makeup = db_to_gain(params[MAKEUP]);
        let mut node = make_node(&params);
        let start = (RATE * 0.5) as u64;
        let end = RATE as u64;
        let mut in_sq = 0.0;
        let mut out_sq = 0.0;
        let mut count = 0u64;
        for frame in 0..end {
            let s = (TAU * 440.0 * frame as f64 / RATE).sin();
            let out = node.process_sample(&ctx(frame), (s, s), &params);
            if frame >= start {
                in_sq += s * s;
                out_sq += out.l * out.l;
                count += 1;
            }
        }
        let in_rms = (in_sq / count as f64).sqrt();
        let out_rms = (out_sq / count as f64).sqrt();
        assert!(out_rms > 0.0);
        assert!(
            out_rms < makeup * in_rms,
            "out {out_rms} should be below makeup x in {}",
            makeup * in_rms
        );
    }
}
