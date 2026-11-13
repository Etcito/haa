//! `harmonics` source node: an additive bank of up to 64 cosine partials whose gains follow a
//! parametric spectral envelope. HAA-only (not a Sona type).
//! Partial k (1-based) has frequency `k·f·sqrt(1 + B·k²)` (B = `inharmonicity`, stiff-string law)
//! detuned by a fixed per-partial random offset scaled by `detune`, and gain
//! `k^(tilt/6.02) · oddEven weight · cos(π·k·offset)` with phase `−π·k·offset`.
//! The `offset` term is the spectrum of two identical events `offset` cycles apart: with
//! `offset = 0.5625` the bank plays the harmonic series of a pair of pulses at 0° and 405° of a
//! 720° cycle; it also gives pluck-position and comb-like spectra.
//! The sum is peak-normalised (divided by Σ|gain|), so `level` is the highest possible peak.
//! Partials above 0.45 × the sample rate fade out over the last 10 % before it.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;

const FREQUENCY: usize = 0;
const PARTIALS: usize = 1;
const TILT: usize = 2;
const ODD_EVEN: usize = 3;
const OFFSET: usize = 4;
const INHARMONICITY: usize = 5;
const DETUNE: usize = 6;
const LEVEL: usize = 7;

const MAX_PARTIALS: usize = 64;
const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

#[allow(clippy::too_many_arguments)]
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
    modulatable: bool,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number { min, max, default, step, scale },
        unit,
        label_en,
        label_fr,
        modulatable,
        smooth: modulatable,
    }
}

const PARAMS: [ParamDef; 8] = [
    number("frequency", 10.0, 8000.0, 110.0, 0.01, Scale::Log, "Hz", "Fundamental", "Fondamentale", true),
    number("partials", 1.0, 64.0, 16.0, 1.0, Scale::Lin, "", "Partials", "Partiels", false),
    number("tilt", -24.0, 6.0, -6.0, 0.1, Scale::Lin, "dB/oct", "Spectral tilt", "Pente spectrale", true),
    number("oddEven", -1.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Odd / even balance", "Équilibre impairs / pairs", true),
    number("offset", 0.0, 1.0, 0.0, 0.0001, Scale::Lin, "cycle", "Pair offset", "Décalage de paire", true),
    number("inharmonicity", 0.0, 0.01, 0.0, 0.00001, Scale::Lin, "", "Inharmonicity", "Inharmonicité", true),
    number("detune", 0.0, 50.0, 0.0, 0.1, Scale::Lin, "cent", "Partial detune", "Désaccord des partiels", true),
    number("level", 0.0, 1.0, 0.3, 0.01, Scale::Lin, "", "Level", "Niveau", true),
];

/// The `harmonics` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "harmonics",
    category: Category::Source,
    label_en: "Harmonic bank",
    label_fr: "Banc d’harmoniques",
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
    let mut rng = Rng::new(init.seed);
    let mut detune = [0.0; MAX_PARTIALS];
    for d in detune.iter_mut() {
        *d = rng.next_f64() * 2.0 - 1.0;
    }
    let sample_rate = if init.sample_rate.is_finite() && init.sample_rate > 0.0 { init.sample_rate } else { 48_000.0 };
    Box::new(Harmonics {
        sample_rate,
        detune,
        phase: [0.0; MAX_PARTIALS],
        gain: [0.0; MAX_PARTIALS],
        shift: [0.0; MAX_PARTIALS],
        ratio: [0.0; MAX_PARTIALS],
        count: 0,
        counter: 0,
        last: [f64::NAN; 6],
    })
}

fn finite_clamp(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    (if v.is_finite() { v } else { fallback }).clamp(min, max)
}

struct Harmonics {
    sample_rate: f64,
    /// Fixed per-partial detune direction in −1..1, drawn from the node seed.
    detune: [f64; MAX_PARTIALS],
    /// Phase of each partial in cycles, 0..1.
    phase: [f64; MAX_PARTIALS],
    /// Peak-normalised signed gain of each partial.
    gain: [f64; MAX_PARTIALS],
    /// Phase shift of each partial in cycles (the pair offset).
    shift: [f64; MAX_PARTIALS],
    /// Frequency of each partial divided by the fundamental.
    ratio: [f64; MAX_PARTIALS],
    count: usize,
    counter: u32,
    /// Spectral params of the last gain update (partials, tilt, oddEven, offset, B, detune).
    last: [f64; 6],
}

impl Harmonics {
    fn update(&mut self, params: &[f64]) {
        let partials = finite_clamp(params[PARTIALS], 16.0, 1.0, 64.0).round();
        let tilt = finite_clamp(params[TILT], -6.0, -24.0, 6.0);
        let odd_even = finite_clamp(params[ODD_EVEN], 0.0, -1.0, 1.0);
        let offset = finite_clamp(params[OFFSET], 0.0, 0.0, 1.0);
        let b = finite_clamp(params[INHARMONICITY], 0.0, 0.0, 0.01);
        let detune = finite_clamp(params[DETUNE], 0.0, 0.0, 50.0);
        let now = [partials, tilt, odd_even, offset, b, detune];
        if now == self.last {
            return;
        }
        self.last = now;
        self.count = partials as usize;
        let exponent = tilt / 6.020_599_913;
        let mut sum = 0.0;
        for i in 0..self.count {
            let k = (i + 1) as f64;
            let parity = if (i + 1) % 2 == 1 { 1.0 - (-odd_even).max(0.0) } else { 1.0 - odd_even.max(0.0) };
            let pair = (PI * k * offset).cos();
            let g = k.powf(exponent) * parity * pair;
            self.gain[i] = g;
            self.shift[i] = -0.5 * k * offset;
            self.ratio[i] = k * (1.0 + b * k * k).sqrt() * 2f64.powf(detune * self.detune[i] / 1200.0);
            sum += g.abs();
        }
        let norm = if sum > 1e-9 { 1.0 / sum } else { 0.0 };
        for g in self.gain.iter_mut().take(self.count) {
            *g *= norm;
        }
    }
}

impl Node for Harmonics {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, _input: (f64, f64), params: &[f64]) -> NodeOut {
        if self.counter & 15 == 0 {
            self.update(params);
        }
        self.counter = self.counter.wrapping_add(1);
        let frequency = finite_clamp(params[FREQUENCY], 110.0, 0.0, 8000.0);
        let level = finite_clamp(params[LEVEL], 0.3, 0.0, 1.0);
        let nyquist = 0.45 * self.sample_rate;
        let mut out = 0.0;
        for i in 0..self.count {
            let f = frequency * self.ratio[i];
            let fade = ((nyquist - f) / (0.1 * nyquist)).clamp(0.0, 1.0);
            if fade > 0.0 {
                out += self.gain[i] * fade * (TAU * (self.phase[i] + self.shift[i])).cos();
            }
            let p = self.phase[i] + f / self.sample_rate;
            self.phase[i] = p - p.floor();
        }
        let v = out * level;
        let v = if v.is_finite() { v } else { 0.0 };
        NodeOut { l: v, r: v, value: 0.0 }
    }

    fn reset(&mut self) {
        self.phase = [0.0; MAX_PARTIALS];
        self.counter = 0;
        self.last = [f64::NAN; 6];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 48_000.0;

    fn ctx() -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: SR, dt: 1.0 / SR, t: 0.0, seconds: 0.0, duration: 1.0, mode: GraphMode::Timeline,
            live_frames: 0, looped: false, perf_rebase: false, inputs: &[], triggers: &[],
        }
    }

    fn run(p: [f64; 8], frames: usize) -> Vec<f64> {
        let init = NodeInit { id: "h", seed: 3, sample_rate: SR, params: &[], texts: &[], curve: None, inputs: &[], duration: 1.0 };
        let mut n = (DEF.create)(&init);
        let c = ctx();
        (0..frames).map(|_| n.process_sample(&c, (0.0, 0.0), &p).l).collect()
    }

    #[test]
    fn peak_never_exceeds_level_and_extremes_stay_finite() {
        for p in [
            [8000.0, 64.0, 6.0, 1.0, 1.0, 0.01, 50.0, 1.0],
            [10.0, 1.0, -24.0, -1.0, 0.0, 0.0, 0.0, 1.0],
            [f64::NAN, f64::INFINITY, f64::NAN, 9.0, -3.0, 1.0, 1e9, 0.5],
        ] {
            let out = run(p, 4_800);
            let level = if p[LEVEL].is_finite() { p[LEVEL].clamp(0.0, 1.0) } else { 0.3 };
            assert!(out.iter().all(|v| v.is_finite() && v.abs() <= level + 1e-9), "{p:?}");
        }
        assert!(run([110.0, 16.0, -6.0, 0.0, 0.0, 0.0, 0.0, 0.0], 480).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn pair_offset_gives_two_events_per_cycle_at_that_offset() {
        // 32 partials at 10 Hz, flat tilt: the waveform peaks at phase 0 and at the offset.
        let out = run([10.0, 32.0, 0.0, 0.0, 0.5625, 0.0, 0.0, 1.0], 4_800);
        let cycle = &out[0..4_800];
        let first = (0..600).max_by(|a, b| cycle[*a].total_cmp(&cycle[*b])).unwrap();
        let second = (2_400..3_000).max_by(|a, b| cycle[*a].total_cmp(&cycle[*b])).unwrap();
        assert!(first < 5, "first peak at {first}");
        assert!((second as f64 - 0.5625 * 4_800.0).abs() < 5.0, "second peak at {second}");
    }
}
