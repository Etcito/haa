//! `crossover` effect node: a 3-band Linkwitz-Riley (4th order) band splitter with a gain per
//! band, summed back. HAA-only (not a Sona type).
//! Low band = LR4 lowpass at `lowFrequency`; the rest is split by LR4 at `highFrequency` into mid
//! and high; the low band runs through the high crossover's allpass so the three bands sum to a
//! flat magnitude when every gain is 0 dB. At −60 dB a band is muted, so parallel copies with one
//! band open each work as a splitter. `highFrequency` is kept at least 1.25 × `lowFrequency`.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::biquad::{Biquad, BiquadKind};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const LOW_FREQUENCY: usize = 0;
const HIGH_FREQUENCY: usize = 1;
const LOW_GAIN: usize = 2;
const MID_GAIN: usize = 3;
const HIGH_GAIN: usize = 4;

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
    scale: Scale,
    unit: &'static str,
    label_en: &'static str,
    label_fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number { min, max, default, step, scale },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

const PARAMS: [ParamDef; 5] = [
    number("lowFrequency", 40.0, 2000.0, 250.0, 1.0, Scale::Log, "Hz", "Low crossover", "Coupure grave"),
    number("highFrequency", 500.0, 16000.0, 3000.0, 1.0, Scale::Log, "Hz", "High crossover", "Coupure aiguë"),
    number("lowGain", -60.0, 12.0, 0.0, 0.1, Scale::Lin, "dB", "Low band", "Bande grave"),
    number("midGain", -60.0, 12.0, 0.0, 0.1, Scale::Lin, "dB", "Mid band", "Bande médium"),
    number("highGain", -60.0, 12.0, 0.0, 0.1, Scale::Lin, "dB", "High band", "Bande aiguë"),
];

/// The `crossover` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "crossover",
    category: Category::Effect,
    label_en: "Crossover",
    label_fr: "Filtre répartiteur",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let sample_rate = if init.sample_rate.is_finite() && init.sample_rate > 0.0 { init.sample_rate } else { 48_000.0 };
    Box::new(Crossover { sample_rate, channels: [Split::new(), Split::new()], counter: 0, last: [f64::NAN; 2] })
}

fn finite_clamp(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    (if v.is_finite() { v } else { fallback }).clamp(min, max)
}

fn db(v: f64) -> f64 {
    if v <= -60.0 { 0.0 } else { 10f64.powf(v / 20.0) }
}

/// An LR4 pair (two Butterworth biquads per side).
struct Lr4 {
    low: [Biquad; 2],
    high: [Biquad; 2],
}

impl Lr4 {
    fn new() -> Lr4 {
        Lr4 { low: [Biquad::new(), Biquad::new()], high: [Biquad::new(), Biquad::new()] }
    }

    fn set(&mut self, f: f64, sr: f64) {
        let q = std::f64::consts::FRAC_1_SQRT_2;
        for b in self.low.iter_mut() {
            b.set(BiquadKind::Lowpass, f, q, 0.0, sr);
        }
        for b in self.high.iter_mut() {
            b.set(BiquadKind::Highpass, f, q, 0.0, sr);
        }
    }

    fn split(&mut self, x: f64) -> (f64, f64) {
        let l = self.low[0].process(x);
        let h = self.high[0].process(x);
        (self.low[1].process(l), self.high[1].process(h))
    }

    fn reset(&mut self) {
        for b in self.low.iter_mut().chain(self.high.iter_mut()) {
            b.reset();
        }
    }
}

struct Split {
    first: Lr4,
    second: Lr4,
    /// The second crossover's allpass (its LP + HP) applied to the low band.
    align: Lr4,
}

impl Split {
    fn new() -> Split {
        Split { first: Lr4::new(), second: Lr4::new(), align: Lr4::new() }
    }
}

struct Crossover {
    sample_rate: f64,
    channels: [Split; 2],
    counter: u32,
    last: [f64; 2],
}

impl Node for Crossover {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, input: (f64, f64), params: &[f64]) -> NodeOut {
        if self.counter & 15 == 0 {
            let low = finite_clamp(params[LOW_FREQUENCY], 250.0, 40.0, 2000.0);
            let high = finite_clamp(params[HIGH_FREQUENCY], 3000.0, 500.0, 16000.0).max(low * 1.25);
            if [low, high] != self.last {
                self.last = [low, high];
                for s in self.channels.iter_mut() {
                    s.first.set(low, self.sample_rate);
                    s.second.set(high, self.sample_rate);
                    s.align.set(high, self.sample_rate);
                }
            }
        }
        self.counter = self.counter.wrapping_add(1);
        let gains = [
            db(finite_clamp(params[LOW_GAIN], 0.0, -60.0, 12.0)),
            db(finite_clamp(params[MID_GAIN], 0.0, -60.0, 12.0)),
            db(finite_clamp(params[HIGH_GAIN], 0.0, -60.0, 12.0)),
        ];
        let mut out = [0.0; 2];
        for (c, x) in [input.0, input.1].into_iter().enumerate() {
            let x = if x.is_finite() { x } else { 0.0 };
            let s = &mut self.channels[c];
            let (low, rest) = s.first.split(x);
            let (mid, high) = s.second.split(rest);
            let (a, b) = s.align.split(low);
            out[c] = (a + b) * gains[0] + mid * gains[1] + high * gains[2];
        }
        if out[0].is_finite() && out[1].is_finite() {
            NodeOut { l: out[0], r: out[1], value: 0.0 }
        } else {
            self.reset();
            NodeOut { l: 0.0, r: 0.0, value: 0.0 }
        }
    }

    fn reset(&mut self) {
        for s in self.channels.iter_mut() {
            s.first.reset();
            s.second.reset();
            s.align.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 48_000.0;

    fn sine_rms(p: [f64; 5], hz: f64) -> f64 {
        let init = NodeInit { id: "x", seed: 1, sample_rate: SR, params: &[], texts: &[], curve: None, inputs: &[], duration: 1.0 };
        let mut n = (DEF.create)(&init);
        let c = SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        };
        let mut sum = 0.0;
        for i in 0..48_000 {
            let x = (std::f64::consts::TAU * hz * i as f64 / SR).sin();
            let o = n.process_sample(&c, (x, x), &p);
            assert!(o.l.is_finite());
            if i >= 24_000 {
                sum += o.l * o.l;
            }
        }
        (sum / 24_000.0).sqrt() * std::f64::consts::SQRT_2
    }

    #[test]
    fn flat_gains_sum_flat_and_a_muted_band_is_muted() {
        for hz in [60.0, 250.0, 1000.0, 3000.0, 9000.0] {
            let g = sine_rms([250.0, 3000.0, 0.0, 0.0, 0.0], hz);
            assert!((g - 1.0).abs() < 0.03, "{hz} Hz gain {g}");
        }
        assert!(sine_rms([250.0, 3000.0, 0.0, -60.0, 0.0], 1000.0) < 0.05);
        assert!(sine_rms([250.0, 3000.0, -60.0, 0.0, -60.0], 1000.0) > 0.9);
    }

    #[test]
    fn bad_params_stay_finite() {
        let g = sine_rms([f64::NAN, -5.0, f64::INFINITY, 1e9, f64::NAN], 500.0);
        assert!(g.is_finite());
    }
}
