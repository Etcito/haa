#![allow(non_snake_case)]
//! `bandNoise` source node: white noise shaped by a parametric spectral envelope, a band given by
//! its centre and its width in octaves, with a tilt across the band. HAA-only (not a Sona type).
//! The band edges are `centre · 2^(±bandwidth/2)`, each a 4th-order Butterworth slope (two RBJ
//! biquads, Q 0.7071); the tilt is a high shelf at the centre whose gain is `tilt × bandwidth`,
//! half of it taken back so the centre keeps its level. The level is normalised by the band's
//! width in Hz, so `level` sets about the same loudness at every bandwidth.
//! `width` decorrelates the right channel with a second noise stream.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::biquad::{Biquad, BiquadKind};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;

const FREQUENCY: usize = 0;
const BANDWIDTH: usize = 1;
const TILT: usize = 2;
const LEVEL: usize = 3;
const WIDTH: usize = 4;

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
    number("frequency", 20.0, 18000.0, 1000.0, 1.0, Scale::Log, "Hz", "Band centre", "Centre de bande"),
    number("bandwidth", 0.1, 10.0, 2.0, 0.01, Scale::Log, "oct", "Bandwidth", "Largeur de bande"),
    number("tilt", -12.0, 12.0, 0.0, 0.1, Scale::Lin, "dB/oct", "Tilt", "Pente"),
    number("level", 0.0, 1.0, 0.2, 0.01, Scale::Lin, "", "Level", "Niveau"),
    number("width", 0.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Stereo spread", "Largeur stéréo"),
];

/// The `bandNoise` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "bandNoise",
    category: Category::Source,
    label_en: "Band noise",
    label_fr: "Bruit en bande",
    audio_in: false,
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
    Box::new(BandNoise {
        seed: init.seed,
        sample_rate,
        left_rng: Rng::new(init.seed),
        right_rng: Rng::new(init.seed ^ 0x9E37_79B9),
        chains: [Chain::new(), Chain::new()],
        gain: 0.0,
        counter: 0,
        last: [f64::NAN; 3],
    })
}

fn finite_clamp(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    (if v.is_finite() { v } else { fallback }).clamp(min, max)
}

/// Highpass ×2, lowpass ×2 and the tilt shelf for one channel.
struct Chain {
    filters: [Biquad; 5],
}

impl Chain {
    fn new() -> Chain {
        Chain { filters: [Biquad::new(), Biquad::new(), Biquad::new(), Biquad::new(), Biquad::new()] }
    }

    fn set(&mut self, low: f64, high: f64, centre: f64, shelf_db: f64, sr: f64) {
        let q = std::f64::consts::FRAC_1_SQRT_2;
        self.filters[0].set(BiquadKind::Highpass, low, q, 0.0, sr);
        self.filters[1].set(BiquadKind::Highpass, low, q, 0.0, sr);
        self.filters[2].set(BiquadKind::Lowpass, high, q, 0.0, sr);
        self.filters[3].set(BiquadKind::Lowpass, high, q, 0.0, sr);
        self.filters[4].set(BiquadKind::Highshelf, centre, q, shelf_db, sr);
    }

    fn process(&mut self, x: f64) -> f64 {
        self.filters.iter_mut().fold(x, |v, f| f.process(v))
    }

    fn reset(&mut self) {
        for f in self.filters.iter_mut() {
            f.reset();
        }
    }
}

struct BandNoise {
    seed: u32,
    sample_rate: f64,
    left_rng: Rng,
    right_rng: Rng,
    chains: [Chain; 2],
    gain: f64,
    counter: u32,
    last: [f64; 3],
}

impl BandNoise {
    fn update(&mut self, params: &[f64]) {
        let centre = finite_clamp(params[FREQUENCY], 1000.0, 20.0, 18000.0);
        let bandwidth = finite_clamp(params[BANDWIDTH], 2.0, 0.1, 10.0);
        let tilt = finite_clamp(params[TILT], 0.0, -12.0, 12.0);
        let now = [centre, bandwidth, tilt];
        if now == self.last {
            return;
        }
        self.last = now;
        let nyquist = 0.5 * self.sample_rate;
        let half = 2f64.powf(bandwidth * 0.5);
        let low = (centre / half).max(10.0);
        let high = (centre * half).min(0.43 * self.sample_rate);
        let shelf_db = (tilt * bandwidth).clamp(-36.0, 36.0);
        for chain in self.chains.iter_mut() {
            chain.set(low, high, centre, shelf_db, self.sample_rate);
        }
        // Uniform white noise in −1..1 has RMS 1/√3; aim at RMS ≈ level / 3 inside the band.
        let band = (high - low).max(20.0);
        let compensation = 10f64.powf(-shelf_db / 40.0);
        self.gain = ((nyquist / band).sqrt() * 3f64.sqrt() / 3.0 * compensation).min(60.0);
    }
}

impl Node for BandNoise {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, _input: (f64, f64), params: &[f64]) -> NodeOut {
        if self.counter & 15 == 0 {
            self.update(params);
        }
        self.counter = self.counter.wrapping_add(1);
        let level = finite_clamp(params[LEVEL], 0.2, 0.0, 1.0);
        let width = finite_clamp(params[WIDTH], 0.0, 0.0, 1.0);
        let a = self.left_rng.next_f64() * 2.0 - 1.0;
        let b = self.right_rng.next_f64() * 2.0 - 1.0;
        // Equal-power blend keeps the right channel's level while it decorrelates.
        let angle = width * std::f64::consts::FRAC_PI_2;
        let right_in = a * angle.cos() + b * angle.sin();
        let l = self.chains[0].process(a) * self.gain * level;
        let r = self.chains[1].process(right_in) * self.gain * level;
        if l.is_finite() && r.is_finite() {
            NodeOut { l, r, value: 0.0 }
        } else {
            self.reset();
            NodeOut { l: 0.0, r: 0.0, value: 0.0 }
        }
    }

    fn reset(&mut self) {
        self.left_rng = Rng::new(self.seed);
        self.right_rng = Rng::new(self.seed ^ 0x9E37_79B9);
        for chain in self.chains.iter_mut() {
            chain.reset();
        }
        self.counter = 0;
        self.last = [f64::NAN; 3];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 48_000.0;

    fn run(p: [f64; 5], frames: usize) -> Vec<(f64, f64)> {
        let init = NodeInit { id: "n", seed: 9, sample_rate: SR, params: &[], texts: &[], curve: None, inputs: &[], duration: 1.0 };
        let mut n = (DEF.create)(&init);
        let c = SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        };
        (0..frames).map(|_| { let o = n.process_sample(&c, (0.0, 0.0), &p); (o.l, o.r) }).collect()
    }

    fn rms(v: &[f64]) -> f64 {
        (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).sqrt()
    }

    #[test]
    fn level_is_close_across_bandwidths_and_extremes_stay_finite() {
        let narrow: Vec<f64> = run([1000.0, 0.3, 0.0, 0.6, 0.0], 48_000).iter().skip(4_800).map(|p| p.0).collect();
        let wide: Vec<f64> = run([1000.0, 6.0, 0.0, 0.6, 0.0], 48_000).iter().skip(4_800).map(|p| p.0).collect();
        let ratio = rms(&narrow) / rms(&wide);
        assert!((0.5..2.0).contains(&ratio), "rms ratio {ratio}");
        for p in [[20.0, 10.0, -12.0, 1.0, 1.0], [18000.0, 0.1, 12.0, 1.0, 0.5], [f64::NAN, -1.0, 1e9, 2.0, f64::NAN]] {
            assert!(run(p, 9_600).iter().all(|(l, r)| l.is_finite() && r.is_finite() && l.abs() < 4.0), "{p:?}");
        }
        assert!(run([1000.0, 2.0, 0.0, 0.0, 0.0], 480).iter().all(|(l, r)| *l == 0.0 && *r == 0.0));
    }

    #[test]
    fn width_zero_is_mono_and_width_one_decorrelates() {
        assert!(run([800.0, 2.0, 0.0, 0.5, 0.0], 4_800).iter().all(|(l, r)| l == r));
        let st = run([800.0, 2.0, 0.0, 0.5, 1.0], 48_000);
        let (mut lr, mut ll) = (0.0, 0.0);
        for (l, r) in &st {
            lr += l * r;
            ll += l * l;
        }
        assert!((lr / ll).abs() < 0.2, "correlation {}", lr / ll);
    }
}
