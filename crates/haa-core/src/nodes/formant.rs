//! `formant` effect node: three parallel bandpass resonances, Sona's "Formants".
//! Ported from Sona v11 effects.js (`case 'formant'`) and its `bandpass` / `runBiquad`.
//! Params and labels from catalog.js `NODE_TYPES.formant` and ui-translations.js.
//! Deviations:
//! - Sona's `runBiquad` is stereo with shared coefficients; the core `Biquad` is mono, so
//!   each band keeps one filter per channel with the same coefficients.
//! - Sona's `frame` counter maps to `ctx.live_frames`.
//! - Mute and bypass are not in this file: the interpreter applies them before the effect.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::biquad::{Biquad, BiquadKind};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const FREQUENCY: usize = 0;
const SPREAD: usize = 1;
const RESONANCE: usize = 2;
const MIX: usize = 3;

/// The `formant` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "formant",
    category: Category::Effect,
    label_en: "Formants",
    label_fr: "Formants",
    audio_in: true,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "frequency",
            kind: ParamKind::Number {
                min: 120.0,
                max: 1600.0,
                default: 560.0,
                step: 1.0,
                scale: Scale::Log,
            },
            unit: "Hz",
            label_en: "Low resonance",
            label_fr: "Résonance basse",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "spread",
            kind: ParamKind::Number {
                min: 0.5,
                max: 2.0,
                default: 1.0,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "×",
            label_en: "Spread",
            label_fr: "Ouverture",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "resonance",
            kind: ParamKind::Number {
                min: 0.3,
                max: 6.0,
                default: 1.2,
                step: 0.05,
                scale: Scale::Lin,
            },
            unit: "Q",
            label_en: "Focus",
            label_fr: "Précision",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "mix",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.35,
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
    Box::new(Formant {
        sample_rate: init.sample_rate,
        left: core::array::from_fn(|_| Biquad::new()),
        right: core::array::from_fn(|_| Biquad::new()),
        fresh: true,
    })
}

struct Formant {
    sample_rate: f64,
    left: [Biquad; 3],
    right: [Biquad; 3],
    fresh: bool,
}

impl Formant {
    fn set_band(&mut self, band: usize, frequency: f64, q: f64) {
        self.left[band].set(BiquadKind::Bandpass, frequency, q, 0.0, self.sample_rate);
        self.right[band].set(BiquadKind::Bandpass, frequency, q, 0.0, self.sample_rate);
    }
}

impl Node for Formant {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        if (ctx.live_frames & 31) == 0 || self.fresh {
            let f = params[FREQUENCY];
            let spread = params[SPREAD];
            let resonance = params[RESONANCE];
            self.set_band(0, f, 1.8 + resonance);
            self.set_band(1, f + 900.0 * spread, 2.5 + resonance * 1.2);
            self.set_band(2, f + 2300.0 * spread, 3.2 + resonance * 1.5);
            self.fresh = false;
        }

        let b0l = self.left[0].process(input.0);
        let b1l = self.left[1].process(input.0);
        let b2l = self.left[2].process(input.0);
        let b0r = self.right[0].process(input.1);
        let b1r = self.right[1].process(input.1);
        let b2r = self.right[2].process(input.1);

        let wet_l = b0l * 1.5 + b1l * 1.15 + b2l * 0.65;
        let wet_r = b0r * 1.5 + b1r * 1.15 + b2r * 0.65;
        let mix = params[MIX];
        let left = input.0 * (1.0 - mix) + wet_l * mix;
        let right = input.1 * (1.0 - mix) + wet_r * mix;

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
        for filter in self.left.iter_mut().chain(self.right.iter_mut()) {
            filter.reset();
        }
        self.fresh = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;
    use crate::rng::Rng;

    fn make_node(rate: f64, params: &[f64]) -> Box<dyn Node> {
        let texts: [Option<String>; 4] = [None, None, None, None];
        (DEF.create)(&NodeInit {
            id: "formant1",
            seed: 0,
            sample_rate: rate,
            params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        })
    }

    fn ctx(frame: u64, rate: f64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: rate,
            dt: 1.0 / rate,
            t: frame as f64 / rate,
            seconds: frame as f64 / rate,
            duration: 1.0,
            mode: GraphMode::Timeline,
            live_frames: frame,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn run(rate: f64, params: &[f64], frames: usize) -> Vec<f64> {
        let mut node = make_node(rate, params);
        let mut rng = Rng::new(12_345);
        let mut out = Vec::with_capacity(frames);
        for frame in 0..frames {
            let noise = rng.next_f64() * 2.0 - 1.0;
            let result = node.process_sample(&ctx(frame as u64, rate), (noise, -noise), params);
            out.push(result.l);
            out.push(result.r);
        }
        out
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let params = [560.0, 1.0, 1.2, 0.35];
        let mut node = make_node(48_000.0, &params);
        for frame in 0..512 {
            let out = node.process_sample(&ctx(frame, 48_000.0), (0.0, 0.0), &params);
            assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn formant_vowels_are_finite() {
        let vowels = [
            [300.0, 0.7, 1.2, 0.35],
            [450.0, 0.9, 1.2, 0.35],
            [700.0, 1.1, 1.2, 0.35],
            [350.0, 1.4, 1.2, 0.35],
            [500.0, 1.6, 1.2, 0.35],
            [120.0, 0.5, 0.3, 0.0],
            [1600.0, 2.0, 6.0, 1.0],
        ];
        for params in vowels {
            for sample in run(48_000.0, &params, 24_000) {
                assert!(sample.is_finite());
                assert!(sample.abs() < 20.0);
            }
        }
    }

    #[test]
    fn params_at_extremes_do_not_exceed_nyquist_clamp() {
        let params = [1600.0, 2.0, 6.0, 1.0];
        for sample in run(22_050.0, &params, 11_025) {
            assert!(sample.is_finite());
            assert!(sample.abs() < 20.0);
        }
    }
}
