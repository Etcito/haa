//! `oscillator` source node: a periodic waveform (sine, triangle, polyBLEP saw and square).
//! Ported from Sona v11 dsp.js (`blep`, `waveform`, `case 'oscillator'`); params and labels from catalog.js
//! `NODE_TYPES.oscillator` and ui-translations.js.
//! Deviations:
//! - The `level` default is catalog.js's 0.35; dsp.js `PARAMS` falls back to 0.5 when a node omits it.
//! - The `wave` enum follows catalog.js's order (sine, triangle, saw, square); dsp.js `ENUMS` lists
//!   sine, saw, square, triangle. Only the index order differs, the values are the same strings.
//! - The phase starts at 0 and Sona's oscillator never draws a random number, so the node seed is unused.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const TAU: f64 = std::f64::consts::PI * 2.0;

const FREQUENCY: usize = 0;
const WAVE: usize = 1;
const LEVEL: usize = 2;

const SINE: usize = 0;
const TRIANGLE: usize = 1;
const SAW: usize = 2;
const SQUARE: usize = 3;

/// The `oscillator` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "oscillator",
    category: Category::Source,
    label_en: "Oscillator",
    label_fr: "Oscillateur",
    audio_in: false,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "frequency",
            kind: ParamKind::Number {
                min: 20.0,
                max: 12000.0,
                default: 220.0,
                step: 1.0,
                scale: Scale::Log,
            },
            unit: "Hz",
            label_en: "Frequency",
            label_fr: "Fréquence",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "wave",
            kind: ParamKind::Enum {
                values: &["sine", "triangle", "saw", "square"],
                default: SINE,
            },
            unit: "",
            label_en: "Shape",
            label_fr: "Forme",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "level",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.35,
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

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let _ = init;
    Box::new(Oscillator { phase: 0.0 })
}

/// Phase accumulator in cycles, 0..1.
struct Oscillator {
    phase: f64,
}

impl Node for Oscillator {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let step = (params[FREQUENCY] * ctx.dt).min(0.45);
        self.phase = (self.phase + step) % 1.0;
        // sona: the enum is its string in Sona; here it is an index into DEF's values, rounded.
        let wave = params[WAVE].round() as usize;
        let value = waveform(self.phase, step, wave) * params[LEVEL] * 0.65;
        // sona: dsp.js zeroes a non-finite value after the switch (`Number.isFinite(value) ? value : 0`).
        let value = if value.is_finite() { value } else { 0.0 };
        // sona: Sona stores the sample in `values` and copies it to both channels; a source returns value 0.
        NodeOut {
            l: value,
            r: value,
            value: 0.0,
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }
}

/// Polynomial band-limited step correction around a discontinuity at phase 0.
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let u = t / dt;
        return u + u - u * u - 1.0;
    }
    if t > 1.0 - dt {
        let u = (t - 1.0) / dt;
        return u * u + u + u + 1.0;
    }
    0.0
}

fn waveform(phase: f64, dt: f64, wave: usize) -> f64 {
    match wave {
        SAW => 2.0 * phase - 1.0 - blep(phase, dt),
        SQUARE => {
            (if phase < 0.5 { 1.0 } else { -1.0 }) + blep(phase, dt) - blep((phase + 0.5) % 1.0, dt)
        }
        TRIANGLE => 1.0 - 4.0 * (phase - 0.5).abs(),
        // sona: any other value falls through to the sine, like Sona's last `return`.
        _ => (TAU * phase).sin(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const RATE: f64 = 48_000.0;

    fn ctx() -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: RATE,
            dt: 1.0 / RATE,
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

    fn init<'a>(seed: u32, params: &'a [f64], texts: &'a [Option<String>]) -> NodeInit<'a> {
        NodeInit {
            id: "osc1",
            seed,
            sample_rate: RATE,
            params,
            texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        }
    }

    fn render(seed: u32, params: &[f64], frames: usize) -> Vec<f64> {
        let texts = [None, None, None];
        let mut node = (DEF.create)(&init(seed, params, &texts));
        let ctx = ctx();
        (0..frames)
            .map(|_| {
                let out = node.process_sample(&ctx, (0.0, 0.0), params);
                assert_eq!(out.value, 0.0);
                assert_eq!(out.l, out.r);
                out.l
            })
            .collect()
    }

    #[test]
    fn every_waveform_stays_finite_and_bounded_at_min_and_max_frequency() {
        for wave in [SINE, TRIANGLE, SAW, SQUARE] {
            for frequency in [20.0, 12_000.0] {
                let params = [frequency, wave as f64, 1.0];
                for sample in render(1, &params, 4_800) {
                    assert!(
                        sample.is_finite() && sample.abs() <= 1.5,
                        "wave {wave} at {frequency} Hz gave {sample}"
                    );
                }
            }
        }
    }

    #[test]
    fn init_params_at_range_extremes_give_finite_output() {
        for frequency in [20.0, 12_000.0] {
            for wave in [SINE, SQUARE] {
                for level in [0.0, 1.0] {
                    let params = [frequency, wave as f64, level];
                    assert!(render(7, &params, 2_400).iter().all(|s| s.is_finite()));
                }
            }
        }
        let silent = [220.0, SAW as f64, 0.0];
        assert!(render(7, &silent, 480).iter().all(|&s| s == 0.0));
    }

    #[test]
    fn same_seed_gives_same_first_64_samples() {
        for wave in [SINE, TRIANGLE, SAW, SQUARE] {
            let params = [440.0, wave as f64, 0.35];
            assert_eq!(render(42, &params, 64), render(42, &params, 64));
        }
    }
}
