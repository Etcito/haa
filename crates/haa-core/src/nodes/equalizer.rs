//! `equalizer` effect node: Sona's "Égaliseur 3 bandes" (low shelf 170 Hz, peaking mid, high shelf 4800 Hz).
//! Ported from Sona v11 effects.js (`processEffect`, `case 'equalizer'`, and `equalizerCoefficients`).
//! Params and labels from catalog.js `NODE_TYPES.equalizer` and ui-translations.js.
//! Deviations:
//! - Sona's `runBiquad` runs left and right with shared coefficients; here each band owns two `Biquad`s
//!   (left, right) from core-dsp, recomputed together so the coefficients stay shared.
//! - Mute and bypass are not in this file: the interpreter applies them as dsp.js `process` does before `processEffect`.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::biquad::{Biquad, BiquadKind};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const LOW: usize = 0;
const MID: usize = 1;
const HIGH: usize = 2;
const FREQUENCY: usize = 3;

const BANDS: usize = 3;
const LOW_HZ: f64 = 170.0;
const HIGH_HZ: f64 = 4800.0;
const MID_Q: f64 = 0.8;
// sona: the shelf slope is fixed (sin(w)/SQRT2) in Biquad, so this q is never read; 0.707 is only a placeholder.
const SHELF_Q: f64 = 0.707;

const fn gain_param(key: &'static str, label_en: &'static str, label_fr: &'static str) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number {
            min: -18.0,
            max: 18.0,
            default: 0.0,
            step: 0.5,
            scale: Scale::Lin,
        },
        unit: "dB",
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

/// The `equalizer` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "equalizer",
    category: Category::Effect,
    label_en: "3-band equalizer",
    label_fr: "Égaliseur 3 bandes",
    audio_in: true,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[
        gain_param("low", "Bass", "Graves"),
        gain_param("mid", "Mids", "Médiums"),
        gain_param("high", "Treble", "Aigus"),
        ParamDef {
            key: "frequency",
            kind: ParamKind::Number {
                min: 150.0,
                max: 8000.0,
                default: 1200.0,
                step: 10.0,
                scale: Scale::Log,
            },
            unit: "Hz",
            label_en: "Mid frequency",
            label_fr: "Centre médiums",
            modulatable: true,
            smooth: true,
        },
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Equalizer {
        sample_rate: init.sample_rate,
        left: [Biquad::new(), Biquad::new(), Biquad::new()],
        right: [Biquad::new(), Biquad::new(), Biquad::new()],
        fresh: true,
    })
}

struct Equalizer {
    sample_rate: f64,
    left: [Biquad; BANDS],
    right: [Biquad; BANDS],
    fresh: bool,
}

impl Equalizer {
    fn set_band(&mut self, band: usize, kind: BiquadKind, freq: f64, q: f64, gain_db: f64) {
        self.left[band].set(kind, freq, q, gain_db, self.sample_rate);
        self.right[band].set(kind, freq, q, gain_db, self.sample_rate);
    }
}

impl Node for Equalizer {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        if (ctx.live_frames & 31) == 0 || self.fresh {
            self.set_band(0, BiquadKind::Lowshelf, LOW_HZ, SHELF_Q, params[LOW]);
            // sona: peakAlpha = sin(w) / 1.6, which is the RBJ alpha sin(w) / (2q) with q = 0.8.
            self.set_band(1, BiquadKind::Peaking, params[FREQUENCY], MID_Q, params[MID]);
            self.set_band(2, BiquadKind::Highshelf, HIGH_HZ, SHELF_Q, params[HIGH]);
            self.fresh = false;
        }
        let (mut left, mut right) = input;
        for band in 0..BANDS {
            left = self.left[band].process(left);
            right = self.right[band].process(right);
        }
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
        for band in 0..BANDS {
            self.left[band].reset();
            self.right[band].reset();
        }
        self.fresh = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const RATE: f64 = 48_000.0;

    fn ctx(live_frames: u64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: RATE,
            dt: 1.0 / RATE,
            t: 0.0,
            seconds: 0.0,
            duration: 1.0,
            mode: GraphMode::Timeline,
            live_frames,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn make() -> Box<dyn Node> {
        let params = [0.0, 0.0, 0.0, 1200.0];
        let texts: [Option<String>; 4] = [None, None, None, None];
        (DEF.create)(&NodeInit {
            id: "eq1",
            seed: 0,
            sample_rate: RATE,
            params: &params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        })
    }

    #[test]
    fn silence_in_gives_silence_out() {
        let params = [6.0, -6.0, 6.0, 1200.0];
        let mut node = make();
        for i in 0..200 {
            let out = node.process_sample(&ctx(i), (0.0, 0.0), &params);
            assert_eq!((out.l, out.r, out.value), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn flat_gains_pass_the_signal_unchanged() {
        let params = [0.0, 0.0, 0.0, 1200.0];
        let mut node = make();
        for i in 0..100u64 {
            let x = (2.0 * core::f64::consts::PI * 1000.0 * i as f64 / RATE).sin();
            let out = node.process_sample(&ctx(i), (x, x), &params);
            assert!((out.l - x).abs() < 1e-6, "left differs at {i}");
            assert!((out.r - x).abs() < 1e-6, "right differs at {i}");
        }
    }

    #[test]
    fn outputs_stay_finite_at_every_param_extreme() {
        for gain in [-18.0, 18.0] {
            for frequency in [150.0, 8000.0] {
                let params = [gain, gain, gain, frequency];
                let mut node = make();
                for i in 0..2000u64 {
                    let x = (2.0 * core::f64::consts::PI * 440.0 * i as f64 / RATE).sin();
                    let out = node.process_sample(&ctx(i), (x, -x), &params);
                    assert!(out.l.is_finite() && out.r.is_finite() && out.value.is_finite());
                }
            }
        }
    }
}
