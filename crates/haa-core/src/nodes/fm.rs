//! `fm` source node: two-operator phase modulation (the classic "FM" synthesis pair).
//! HAA-only (not a Sona type).
//! Modulator `m = sin(2π·φm + feedback·π·m₋₁)` at `frequency × ratio`; carrier
//! `sin(2π·φc + index·m)` at `frequency`. Integer ratios give harmonic spectra, others
//! inharmonic (bells, metal); `index` widens the spectrum; `feedback` turns the modulator from a
//! sine toward a saw and then noise. Both phases are continuous through frequency changes.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const FREQUENCY: usize = 0;
const RATIO: usize = 1;
const INDEX: usize = 2;
const FEEDBACK: usize = 3;
const LEVEL: usize = 4;

const TAU: f64 = std::f64::consts::TAU;

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
    number("frequency", 1.0, 12000.0, 220.0, 0.01, Scale::Log, "Hz", "Carrier frequency", "Fréquence porteuse"),
    number("ratio", 0.0625, 16.0, 1.0, 0.0001, Scale::Log, "×", "Modulator ratio", "Rapport du modulateur"),
    number("index", 0.0, 20.0, 2.0, 0.01, Scale::Lin, "rad", "Modulation index", "Indice de modulation"),
    number("feedback", 0.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Modulator feedback", "Réinjection du modulateur"),
    number("level", 0.0, 1.0, 0.3, 0.01, Scale::Lin, "", "Level", "Niveau"),
];

/// The `fm` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "fm",
    category: Category::Source,
    label_en: "FM pair",
    label_fr: "Paire FM",
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
    Box::new(Fm { sample_rate, carrier: 0.0, modulator: 0.0, last: 0.0 })
}

fn finite_clamp(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    (if v.is_finite() { v } else { fallback }).clamp(min, max)
}

struct Fm {
    sample_rate: f64,
    carrier: f64,
    modulator: f64,
    last: f64,
}

impl Node for Fm {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, _input: (f64, f64), params: &[f64]) -> NodeOut {
        let frequency = finite_clamp(params[FREQUENCY], 220.0, 0.0, 12000.0);
        let ratio = finite_clamp(params[RATIO], 1.0, 0.0625, 16.0);
        let index = finite_clamp(params[INDEX], 2.0, 0.0, 20.0);
        let feedback = finite_clamp(params[FEEDBACK], 0.0, 0.0, 1.0);
        let level = finite_clamp(params[LEVEL], 0.3, 0.0, 1.0);
        // Averaging the last two modulator values tames the feedback loop's hunting.
        let m = (TAU * self.modulator + feedback * std::f64::consts::PI * self.last).sin();
        self.last = 0.5 * (self.last + m);
        let v = (TAU * self.carrier + index * m).sin() * level;
        let c = self.carrier + frequency / self.sample_rate;
        self.carrier = c - c.floor();
        let p = self.modulator + (frequency * ratio).min(0.5 * self.sample_rate) / self.sample_rate;
        self.modulator = p - p.floor();
        let v = if v.is_finite() { v } else { 0.0 };
        NodeOut { l: v, r: v, value: 0.0 }
    }

    fn reset(&mut self) {
        self.carrier = 0.0;
        self.modulator = 0.0;
        self.last = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 48_000.0;

    fn run(p: [f64; 5], frames: usize) -> Vec<f64> {
        let init = NodeInit { id: "f", seed: 1, sample_rate: SR, params: &[], texts: &[], curve: None, inputs: &[], duration: 1.0 };
        let mut n = (DEF.create)(&init);
        let c = SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        };
        (0..frames).map(|_| n.process_sample(&c, (0.0, 0.0), &p).l).collect()
    }

    #[test]
    fn index_zero_is_a_pure_sine() {
        let out = run([1000.0, 3.0, 0.0, 0.0, 0.5], 480);
        for (i, v) in out.iter().enumerate() {
            assert!((v - 0.5 * (TAU * 1000.0 * i as f64 / SR).sin()).abs() < 1e-9);
        }
    }

    #[test]
    fn extremes_stay_within_level() {
        for p in [[12000.0, 16.0, 20.0, 1.0, 1.0], [1.0, 0.0625, 20.0, 1.0, 0.7], [f64::NAN, -1.0, 1e9, 9.0, f64::NAN]] {
            let out = run(p, 9_600);
            assert!(out.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "{p:?}");
        }
    }
}
