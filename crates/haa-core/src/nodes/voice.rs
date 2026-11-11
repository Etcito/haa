//! `voice` source node: a one-dimensional vocal tract driven by a glottal flow pulse.
//!
//! The tract is a staggered-grid FDTD line (pressure at cell centres, volume velocity on the
//! branches) with a pressure-release lip and a driven glottis. The area function morphs between
//! five vowel profiles, so the formants emerge from the geometry instead of a formant filter.
//! This is the CPU fallback and reference for the eventual GPU tract: the same equations, run
//! twice per output sample (oversampling) to keep enough resolution for the short tract.
//!
//! Units follow the acoustics literature: cm, cm², cm³/s, g/cm³, dyn/cm².
//! Deviations from a full physical model: no wall compliance, no nasal branch, no turbulence
//! noise placed at a constriction; `breath` is a glottal noise floor. Add those as separate
//! nodes or a later version of this one.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;

const PITCH: usize = 0;
const PRESSURE: usize = 1;
const TENSION: usize = 2;
const VOWEL: usize = 3;
const BREATH: usize = 4;
const GAIN: usize = 5;

/// The `voice` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "voice",
    category: Category::Source,
    label_en: "Voice",
    label_fr: "Voix",
    audio_in: false,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "pitch",
            kind: ParamKind::Number {
                min: 40.0,
                max: 1100.0,
                default: 110.0,
                step: 1.0,
                scale: Scale::Log,
            },
            unit: "Hz",
            label_en: "Pitch",
            label_fr: "Hauteur",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "pressure",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.55,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Pressure",
            label_fr: "Pression",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "tension",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Tension",
            label_fr: "Tension",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "vowel",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.1,
                step: 0.001,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Vowel",
            label_fr: "Voyelle",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "breath",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.08,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Breath",
            label_fr: "Souffle",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "gain",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Level",
            label_fr: "Niveau",
            modulatable: true,
            smooth: true,
        },
    ],
    create,
};

/// Maximum FDTD cells. 192 kHz with a 17 cm tract needs ~93 cells.
const MAX_CELLS: usize = 128;
/// Speed of sound in the tract, cm/s.
const C_SOUND: f64 = 35_000.0;
/// Air density, g/cm³.
const RHO: f64 = 1.14e-3;
/// Adult male tract length, cm.
const TRACT_CM: f64 = 17.0;
/// Grid spacing above the CFL limit (c·dt/dx = 1/CFL), for stability margin.
const CFL: f64 = 1.02;
/// Smallest cell area, cm².
const MIN_AREA: f64 = 0.05;
/// Per-branch loss per internal step (viscous and wall losses, lumped).
const BRANCH_LOSS: f64 = 0.99995;
/// Glottal volume velocity at full pressure, cm³/s.
const GLOTTIS_FLOW: f64 = 320.0;
/// Radiated pressure scale (derivative of lip flow × this).
const RADIATION_SCALE: f64 = 0.012;
/// Output scale before the gain parameter.
const OUTPUT_SCALE: f64 = 1.0;
/// Loudness compensation per vowel: a closed lip radiates much less than an open one.
const VOWEL_GAIN: [f64; 5] = [1.0, 1.5, 2.6, 2.0, 4.5];
/// Vowel area functions in cm², glottis to lips (16 points, adult male).
/// Fitted with a lossless transmission-line model against the male formant targets
/// (a 730/1090/2440, e 530/1840/2480, i 270/2290/3010, o 570/840/2410, u 300/870/2240 Hz).
/// Only area ratios shape the resonances, so the whole set is scaled to keep the widest
/// section near 5.6 cm².
const VOWELS: [[f64; 16]; 5] = [
    // /a/
    [0.312, 0.140, 0.085, 0.078, 0.116, 0.266, 0.857, 3.317, 5.600, 5.600, 5.600, 5.600, 5.600, 5.600, 5.600, 1.907],
    // /e/
    [0.240, 0.260, 0.237, 0.190, 0.144, 0.109, 0.090, 0.084, 0.092, 0.117, 0.169, 0.264, 0.409, 0.585, 0.718, 0.720],
    // /i/
    [0.935, 1.843, 3.307, 4.978, 5.600, 5.583, 4.169, 2.619, 1.502, 0.861, 0.543, 0.404, 0.375, 0.438, 0.631, 1.057],
    // /o/
    [0.289, 0.223, 0.183, 0.167, 0.170, 0.195, 0.247, 0.334, 0.462, 0.625, 0.793, 0.913, 0.936, 0.855, 0.706, 0.543],
    // /u/
    [5.600, 2.367, 1.159, 0.813, 0.853, 1.305, 2.646, 5.600, 5.600, 5.600, 5.600, 5.600, 5.600, 4.849, 1.243, 0.281],
];
/// Profiles are already in cm².
const AREA_SCALE: f64 = 1.0;

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Voice::new(init.sample_rate, init.seed))
}

struct Voice {
    sample_rate: f64,
    /// Internal steps per output sample (2 below 60 kHz, 1 above).
    oversample: usize,
    /// FDTD cells.
    cells: usize,
    dx: f64,
    p: [f64; MAX_CELLS],
    u: [f64; MAX_CELLS],
    kp: [f64; MAX_CELLS],
    ku: [f64; MAX_CELLS],
    area: [f64; MAX_CELLS],
    /// Smoothing counter for the area function.
    control: u32,
    /// Interpolated loudness compensation for the current vowel.
    level: f64,
    /// Glottal phase, 0..1.
    phase: f64,
    /// Last lip flow, for the radiation derivative.
    lip_prev: f64,
    rng: Rng,
    /// One-pole DC blocker state.
    dc_x: f64,
    dc_y: f64,
}

impl Voice {
    fn new(sample_rate: f64, seed: u32) -> Self {
        let rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate.min(192_000.0)
        } else {
            48_000.0
        };
        let oversample = if rate <= 60_000.0 { 2 } else { 1 };
        let fs = rate * oversample as f64;
        let dt = 1.0 / fs;
        let dx = C_SOUND * dt * CFL;
        let cells = ((TRACT_CM / dx).round() as usize).clamp(8, MAX_CELLS);
        let mut voice = Self {
            sample_rate: rate,
            oversample,
            cells,
            dx,
            p: [0.0; MAX_CELLS],
            u: [0.0; MAX_CELLS],
            kp: [0.0; MAX_CELLS],
            ku: [0.0; MAX_CELLS],
            area: [0.0; MAX_CELLS],
            control: 0,
            level: 1.0,
            phase: 0.0,
            lip_prev: 0.0,
            rng: Rng::new(seed ^ 0x5eed_1234),
            dc_x: 0.0,
            dc_y: 0.0,
        };
        voice.update_areas(0.1);
        voice
    }

    /// Interpolates the vowel profiles and refreshes the FDTD coefficients.
    fn update_areas(&mut self, vowel: f64) {
        let position = vowel.clamp(0.0, 1.0) * (VOWELS.len() - 1) as f64;
        let first = position.floor() as usize;
        let second = (first + 1).min(VOWELS.len() - 1);
        let mix = position - first as f64;
        self.level = VOWEL_GAIN[first] + (VOWEL_GAIN[second] - VOWEL_GAIN[first]) * mix;
        let dt = 1.0 / (self.sample_rate * self.oversample as f64);
        for i in 0..self.cells {
            let x = (i as f64 + 0.5) / self.cells as f64 * 15.0;
            let a = sample_profile(&VOWELS[first], x);
            let b = sample_profile(&VOWELS[second], x);
            let area = ((a + (b - a) * mix) * AREA_SCALE).clamp(MIN_AREA, 20.0);
            self.area[i] = area;
            // dp/dt = -(rho c² / A) du/dx
            self.kp[i] = RHO * C_SOUND * C_SOUND * dt / (area * self.dx);
        }
        // du/dt = -(A / rho) dp/dx, using the area of the branch between cells.
        for i in 0..self.cells {
            let face = if i + 1 < self.cells {
                0.5 * (self.area[i] + self.area[i + 1])
            } else {
                self.area[i]
            };
            self.ku[i] = face * dt / (RHO * self.dx);
        }
    }

    /// The glottal volume velocity for the current internal step.
    fn glottis_flow(&mut self, params: &[f64]) -> f64 {
        let fs = self.sample_rate * self.oversample as f64;
        let f0 = params[PITCH].clamp(40.0, 1_100.0);
        let pressure = params[PRESSURE].clamp(0.0, 1.0);
        let tension = params[TENSION].clamp(0.0, 1.0);
        let breath = params[BREATH].clamp(0.0, 1.0);
        let open_quota = (0.78 - 0.36 * tension).clamp(0.25, 0.98);

        self.phase += f0 / fs;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let shape = if self.phase < open_quota {
            // Full raised cosine: zero flow and zero slope at opening and closure. A half cosine
            // would jump to zero at closure and put an impulse into the tract, tilting the
            // spectrum upwards and masking the formants.
            let x = self.phase / open_quota;
            0.5 * (1.0 - (core::f64::consts::TAU * x).cos())
        } else {
            0.0
        };
        let mut flow = pressure * GLOTTIS_FLOW * shape;
        if breath > 0.0 {
            // Deterministic noise floor: aspiration, kept off the pulse's fundamental.
            let noise = self.rng.next_f64() * 2.0 - 1.0;
            flow += pressure * breath * 40.0 * noise;
        }
        flow
    }

    /// One internal FDTD step.
    fn step(&mut self, params: &[f64]) {
        let source = self.glottis_flow(params);
        for i in 0..self.cells {
            let incoming = if i == 0 { source } else { self.u[i - 1] };
            self.p[i] += self.kp[i] * (incoming - self.u[i]);
        }
        for i in 0..self.cells {
            // The lip sees a pressure-release boundary (p = 0 past the last cell).
            let next = if i + 1 < self.cells { self.p[i + 1] } else { 0.0 };
            self.u[i] = (self.u[i] + self.ku[i] * (self.p[i] - next)) * BRANCH_LOSS;
        }
    }
}

/// Linear interpolation of one of the 16-point area profiles.
fn sample_profile(profile: &[f64; 16], x: f64) -> f64 {
    let clamped = x.clamp(0.0, 15.0);
    let first = clamped.floor() as usize;
    let second = (first + 1).min(15);
    let mix = clamped - first as f64;
    profile[first] + (profile[second] - profile[first]) * mix
}

impl Node for Voice {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        // The engine smooths params per output sample; refresh the geometry at that rate.
        self.control = self.control.wrapping_add(1);
        if self.control & 7 == 1 {
            self.update_areas(params[VOWEL]);
        }

        for _ in 0..self.oversample {
            self.step(params);
        }

        let lip = self.u[self.cells - 1];
        // Radiation: the lip pressure is proportional to the derivative of the lip flow.
        let radiated = (lip - self.lip_prev) * RADIATION_SCALE;
        self.lip_prev = lip;

        // 20 Hz DC blocker, matched to the node's own release behaviour.
        let coefficient = (-core::f64::consts::TAU * 20.0 / self.sample_rate).exp();
        self.dc_y = radiated - self.dc_x + coefficient * self.dc_y;
        self.dc_x = radiated;

        let gain = params[GAIN].clamp(0.0, 1.0);
        let sample = self.dc_y * OUTPUT_SCALE * gain * self.level;
        let sample = if sample.is_finite() {
            sample.clamp(-1.5, 1.5)
        } else {
            0.0
        };
        NodeOut {
            l: sample,
            r: sample,
            value: 0.0,
        }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        if (init.sample_rate - self.sample_rate).abs() > 0.5 {
            *self = Voice::new(init.sample_rate, init.seed);
        }
    }

    fn reset(&mut self) {
        self.p = [0.0; MAX_CELLS];
        self.u = [0.0; MAX_CELLS];
        self.control = 0;
        self.phase = 0.0;
        self.lip_prev = 0.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    fn make_node(rate: f64, params: &[f64]) -> Box<dyn Node> {
        let texts: [Option<String>; 6] = [None, None, None, None, None, None];
        (DEF.create)(&NodeInit {
            id: "voice1",
            seed: 7,
            sample_rate: rate,
            params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 2.0,
        })
    }

    fn ctx(frame: u64, rate: f64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: rate,
            dt: 1.0 / rate,
            t: frame as f64 / rate,
            seconds: frame as f64 / rate,
            duration: 2.0,
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
        (0..frames)
            .map(|frame| node.process_sample(&ctx(frame as u64, rate), (0.0, 0.0), params).l)
            .collect()
    }

    fn rms(samples: &[f64]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|x| x * x).sum::<f64>() / samples.len() as f64).sqrt()
    }

    #[test]
    fn gain_at_zero_is_silent() {
        let params = [110.0, 0.6, 0.5, 0.3, 0.0, 0.0];
        for sample in run(48_000.0, &params, 4_800) {
            assert_eq!(sample, 0.0);
        }
    }

    #[test]
    fn vowels_are_finite_and_bounded() {
        for vowel in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let params = [110.0, 0.6, 0.5, vowel, 0.05, 0.5];
            for sample in run(48_000.0, &params, 48_000) {
                assert!(sample.is_finite(), "vowel {vowel} produced {sample}");
                assert!(sample.abs() <= 1.5, "vowel {vowel} produced {sample}");
            }
        }
    }

    #[test]
    fn voice_is_audible() {
        let params = [110.0, 0.6, 0.5, 0.1, 0.05, 0.5];
        let output = run(48_000.0, &params, 24_000);
        assert!(rms(&output) > 0.01, "rms was {}", rms(&output));
    }

    #[test]
    fn vowels_are_different() {
        let params = |vowel| [110.0, 0.6, 0.5, vowel, 0.0, 0.5];
        let a = run(48_000.0, &params(0.0), 12_000);
        let i = run(48_000.0, &params(0.5), 12_000);
        let u = run(48_000.0, &params(1.0), 12_000);
        let difference = |left: &[f64], right: &[f64]| {
            left.iter()
                .zip(right)
                .map(|(x, y)| (x - y).abs())
                .sum::<f64>()
                / left.len() as f64
        };
        assert!(difference(&a, &i) > 0.001);
        assert!(difference(&i, &u) > 0.001);
    }

    #[test]
    fn pitch_sets_the_fundamental() {
        let params = [100.0, 0.7, 0.5, 0.1, 0.0, 0.7];
        let output = run(48_000.0, &params, 24_000);
        // Autocorrelation over the steady part, lags 40..1200 (40..1200 Hz at 48 kHz).
        let start = 8_000;
        let window = 8_000;
        let signal = &output[start..start + window];
        let mut best_lag = 0;
        let mut best = f64::MIN;
        for lag in 40..1_200 {
            let mut sum = 0.0;
            for i in 0..window - lag {
                sum += signal[i] * signal[i + lag];
            }
            if sum > best {
                best = sum;
                best_lag = lag;
            }
        }
        let f0 = 48_000.0 / best_lag as f64;
        assert!((f0 - 100.0).abs() < 15.0, "estimated f0 {f0}");
    }

    #[test]
    fn reset_returns_to_silence() {
        let params = [110.0, 0.6, 0.5, 0.1, 0.0, 0.5];
        let mut node = make_node(48_000.0, &params);
        for frame in 0..4_800 {
            let _ = node.process_sample(&ctx(frame, 48_000.0), (0.0, 0.0), &params);
        }
        node.reset();
        let first = node.process_sample(&ctx(0, 48_000.0), (0.0, 0.0), &params);
        assert!(first.l.abs() < 1e-12);
    }

    #[test]
    fn extreme_params_are_finite_at_every_rate() {
        for rate in [8_000.0, 48_000.0, 192_000.0] {
            for params in [
                [40.0, 1.0, 1.0, 0.0, 1.0, 1.0],
                [1_100.0, 1.0, 0.0, 1.0, 1.0, 1.0],
                [40.0, 0.0, 0.0, 0.5, 0.0, 1.0],
            ] {
                for sample in run(rate, &params, 12_000) {
                    assert!(sample.is_finite(), "rate {rate} params {params:?}");
                    assert!(sample.abs() <= 1.5);
                }
            }
        }
    }
}
