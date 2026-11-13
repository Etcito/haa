//! `waveguide` effect node: a tuned delay-line resonator (a 1-D digital waveguide folded into one
//! loop) with loss, dispersion and end condition. HAA-only (not a Sona type).
//! - `ends = open` (both ends alike): loop delay 1/f, reflection +g: all harmonics of f.
//! - `ends = closed` (one closed end, one open end, like a pipe on a valve or a clarinet):
//!   loop delay 1/(2f), reflection −g: odd harmonics of f (quarter-wave resonances).
//! The loop holds a one-pole lowpass (`damping`, wall and radiation loss growing with frequency)
//! and a first-order allpass (`dispersion`, stiffness: upper modes go sharp or flat).
//! The loop delay is shortened by the filters' low-frequency delay so `frequency` stays the
//! fundamental. The wet path is scaled by √(1 − g²) so the level stays usable as `feedback`
//! moves; `mix` blends dry and wet. Each channel has its own line.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::delay_line::DelayLine;
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const FREQUENCY: usize = 0;
const FEEDBACK: usize = 1;
const DAMPING: usize = 2;
const DISPERSION: usize = 3;
const ENDS: usize = 4;
const MIX: usize = 5;

const MIN_FREQUENCY: f64 = 20.0;
const MAX_FREQUENCY: f64 = 5000.0;

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

const PARAMS: [ParamDef; 6] = [
    number("frequency", MIN_FREQUENCY, MAX_FREQUENCY, 150.0, 0.01, Scale::Log, "Hz", "Fundamental", "Fondamentale"),
    number("feedback", 0.0, 0.995, 0.7, 0.001, Scale::Lin, "", "Reflection", "Réflexion"),
    number("damping", 0.0, 0.95, 0.3, 0.01, Scale::Lin, "", "Damping", "Amortissement"),
    number("dispersion", -0.9, 0.9, 0.0, 0.01, Scale::Lin, "", "Dispersion", "Dispersion"),
    ParamDef {
        key: "ends",
        kind: ParamKind::Enum { values: &["open", "closed"], default: 0 },
        unit: "",
        label_en: "Ends",
        label_fr: "Extrémités",
        modulatable: false,
        smooth: false,
    },
    number("mix", 0.0, 1.0, 1.0, 0.01, Scale::Lin, "", "Mix", "Mélange"),
];

/// The `waveguide` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "waveguide",
    category: Category::Effect,
    label_en: "Waveguide resonator",
    label_fr: "Résonateur à guide d’onde",
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
    let max_seconds = 1.0 / MIN_FREQUENCY + 0.002;
    Box::new(Waveguide {
        sample_rate,
        lines: [DelayLine::new(max_seconds, sample_rate), DelayLine::new(max_seconds, sample_rate)],
        lowpass: [0.0; 2],
        allpass_x: [0.0; 2],
        allpass_y: [0.0; 2],
    })
}

fn finite_clamp(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    (if v.is_finite() { v } else { fallback }).clamp(min, max)
}

struct Waveguide {
    sample_rate: f64,
    lines: [DelayLine; 2],
    lowpass: [f64; 2],
    allpass_x: [f64; 2],
    allpass_y: [f64; 2],
}

impl Waveguide {
    #[allow(clippy::too_many_arguments)]
    fn channel(&mut self, c: usize, x: f64, delay: f64, reflection: f64, damping: f64, dispersion: f64, wet_gain: f64, mix: f64) -> f64 {
        let back = self.lines[c].read(delay);
        // One-pole lowpass: y = (1 − d)·x + d·y₋₁.
        self.lowpass[c] = (1.0 - damping) * back + damping * self.lowpass[c];
        // First-order allpass: y = a·x + x₋₁ − a·y₋₁.
        let a = dispersion;
        let y = a * self.lowpass[c] + self.allpass_x[c] - a * self.allpass_y[c];
        self.allpass_x[c] = self.lowpass[c];
        self.allpass_y[c] = y;
        let into = x + reflection * y;
        self.lines[c].write(if into.is_finite() { into } else { 0.0 });
        x * (1.0 - mix) + into * wet_gain * mix
    }
}

impl Node for Waveguide {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, input: (f64, f64), params: &[f64]) -> NodeOut {
        let frequency = finite_clamp(params[FREQUENCY], 150.0, MIN_FREQUENCY, MAX_FREQUENCY);
        let feedback = finite_clamp(params[FEEDBACK], 0.7, 0.0, 0.995);
        let damping = finite_clamp(params[DAMPING], 0.3, 0.0, 0.95);
        let dispersion = finite_clamp(params[DISPERSION], 0.0, -0.9, 0.9);
        let closed = params[ENDS].round() == 1.0;
        let mix = finite_clamp(params[MIX], 1.0, 0.0, 1.0);
        let period = self.sample_rate / frequency * if closed { 0.5 } else { 1.0 };
        // Low-frequency delays of the loop filters: the lowpass d/(1−d) and the allpass
        // (1−a)/(1+a); the line read itself is exactly `delay` samples old.
        let filters = damping / (1.0 - damping) + (1.0 - dispersion) / (1.0 + dispersion);
        let delay = (period - filters).max(1.0);
        let reflection = if closed { -feedback } else { feedback };
        let wet_gain = (1.0 - feedback * feedback).sqrt();
        let l = self.channel(0, input.0, delay, reflection, damping, dispersion, wet_gain, mix);
        let r = self.channel(1, input.1, delay, reflection, damping, dispersion, wet_gain, mix);
        if l.is_finite() && r.is_finite() {
            NodeOut { l, r, value: 0.0 }
        } else {
            self.reset();
            NodeOut { l: 0.0, r: 0.0, value: 0.0 }
        }
    }

    fn reset(&mut self) {
        for line in self.lines.iter_mut() {
            line.reset();
        }
        self.lowpass = [0.0; 2];
        self.allpass_x = [0.0; 2];
        self.allpass_y = [0.0; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 48_000.0;

    fn impulse_response(p: [f64; 6], frames: usize) -> Vec<f64> {
        let init = NodeInit { id: "w", seed: 1, sample_rate: SR, params: &[], texts: &[], curve: None, inputs: &[], duration: 1.0 };
        let mut n = (DEF.create)(&init);
        let c = SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        };
        (0..frames).map(|i| { let x = if i == 0 { 1.0 } else { 0.0 }; n.process_sample(&c, (x, x), &p).l }).collect()
    }

    /// Magnitude of the DFT bin at `hz`.
    fn bin(v: &[f64], hz: f64) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, x) in v.iter().enumerate() {
            let w = std::f64::consts::TAU * hz * i as f64 / SR;
            re += x * w.cos();
            im -= x * w.sin();
        }
        (re * re + im * im).sqrt()
    }

    #[test]
    fn closed_end_rings_on_odd_harmonics_of_the_fundamental() {
        let ir = impulse_response([200.0, 0.95, 0.05, 0.0, 1.0, 1.0], 24_000);
        let f1 = bin(&ir, 200.0);
        let f2 = bin(&ir, 400.0);
        let f3 = bin(&ir, 600.0);
        assert!(f1 > 5.0 * f2 && f3 > 5.0 * f2, "f1 {f1} f2 {f2} f3 {f3}");
        let open = impulse_response([200.0, 0.95, 0.05, 0.0, 0.0, 1.0], 24_000);
        assert!(bin(&open, 400.0) > 5.0 * bin(&open, 300.0));
    }

    #[test]
    fn silence_gives_silence_and_extremes_stay_finite() {
        let init = NodeInit { id: "w", seed: 1, sample_rate: SR, params: &[], texts: &[], curve: None, inputs: &[], duration: 1.0 };
        let mut n = (DEF.create)(&init);
        let c = SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        };
        for _ in 0..4_800 {
            let o = n.process_sample(&c, (0.0, 0.0), &[150.0, 0.995, 0.0, 0.9, 1.0, 1.0]);
            assert_eq!((o.l, o.r), (0.0, 0.0));
        }
        for p in [[20.0, 0.995, 0.0, -0.9, 0.0, 1.0], [5000.0, 0.995, 0.95, 0.9, 1.0, 0.5], [f64::NAN, 9.0, -1.0, f64::INFINITY, 1.0, 2.0]] {
            let ir = impulse_response(p, 48_000);
            assert!(ir.iter().all(|v| v.is_finite() && v.abs() < 10.0), "{p:?}");
        }
    }
}
