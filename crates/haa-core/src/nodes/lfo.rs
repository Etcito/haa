//! `lfo` control node: a control oscillator from slow movement to audio-rate modulation.
//! Ported from Sona v11 dsp.js (`blep`, `waveform`, `case 'lfo'`); params and labels from
//! catalog.js `NODE_TYPES.lfo` and ui-translations.js.
//! Deviations:
//! - The `rate` and `depth` defaults are catalog.js's 2 and 0.5; dsp.js `PARAMS` falls back to
//!   1 and 1 when a node omits them (same difference as `oscillator`'s level).
//! - The `wave` enum follows catalog.js's order (sine, triangle, square), which matches dsp.js
//!   `ENUMS.lfo`; the value is an index here and is rounded before use.
//! - The phase starts at 0 and Sona's LFO never draws a random number, so the node seed is unused.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const TAU: f64 = std::f64::consts::PI * 2.0;

const RATE: usize = 0;
const DEPTH: usize = 1;
const WAVE: usize = 2;

const SINE: usize = 0;
const TRIANGLE: usize = 1;
const SQUARE: usize = 2;

/// The `lfo` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "lfo",
    category: Category::Control,
    label_en: "LFO",
    label_fr: "LFO",
    audio_in: false,
    audio_out: false,
    control_out: true,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "rate",
            kind: ParamKind::Number {
                min: 0.05,
                max: 2000.0,
                default: 2.0,
                step: 0.05,
                scale: Scale::Log,
            },
            unit: "Hz",
            label_en: "Rate",
            label_fr: "Vitesse",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "depth",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Amplitude",
            label_fr: "Amplitude",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "wave",
            kind: ParamKind::Enum {
                values: &["sine", "triangle", "square"],
                default: SINE,
            },
            unit: "",
            label_en: "Shape",
            label_fr: "Forme",
            modulatable: false,
            smooth: false,
        },
    ],
    create,
};

fn create(_init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Lfo { phase: 0.0 })
}

/// Phase accumulator in cycles, 0..1.
struct Lfo {
    phase: f64,
}

impl Node for Lfo {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let step = params[RATE] * ctx.dt;
        self.phase = (self.phase + step) % 1.0;
        // sona: the enum is its string in Sona; here it is an index into DEF's values, rounded.
        let wave = params[WAVE].round() as usize;
        let value = waveform(self.phase, step, wave) * params[DEPTH];
        // sona: dsp.js zeroes a non-finite value after the switch (`Number.isFinite(value) ? value : 0`).
        let value = if value.is_finite() { value } else { 0.0 };
        // sona: a control returns l = r = 0; Sona copies the value into both channels.
        NodeOut {
            l: 0.0,
            r: 0.0,
            value,
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }
}

/// Polynomial band-limited step correction around a discontinuity at phase 0.
// sona: Sona shares this helper between `oscillator` and `lfo` (dsp.js lines 113-123);
// here `oscillator.rs` belongs to another module, so this file keeps its own private copy.
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

    fn render(params: &[f64], frames: usize) -> Vec<f64> {
        let texts: [Option<String>; 3] = [None, None, None];
        let init = NodeInit {
            id: "lfo1",
            seed: 0,
            sample_rate: RATE,
            params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        };
        let mut node = (DEF.create)(&init);
        let ctx = ctx();
        (0..frames)
            .map(|_| {
                let out = node.process_sample(&ctx, (0.0, 0.0), params);
                assert_eq!(out.l, 0.0);
                assert_eq!(out.r, 0.0);
                out.value
            })
            .collect()
    }

    #[test]
    fn output_stays_within_depth() {
        let depth = 0.4;
        for wave in [SINE, TRIANGLE, SQUARE] {
            let limit = if wave == SQUARE { depth * 1.5 } else { depth };
            for rate in [0.05, 5.0, 2000.0] {
                let params = [rate, depth, wave as f64];
                for value in render(&params, (RATE as usize) / 2) {
                    assert!(
                        value.is_finite() && value.abs() <= limit,
                        "wave {wave} at {rate} Hz gave {value}, limit ±{limit}"
                    );
                }
            }
        }
    }

    #[test]
    fn zero_depth_gives_zero() {
        for wave in [SINE, TRIANGLE, SQUARE] {
            let params = [5.0, 0.0, wave as f64];
            assert!(render(&params, 4_800).iter().all(|&value| value == 0.0));
        }
    }

    #[test]
    fn init_params_at_range_extremes_give_finite_output() {
        for wave in [SINE, TRIANGLE, SQUARE] {
            for rate in [0.05, 2000.0] {
                for depth in [0.0, 1.0] {
                    let params = [rate, depth, wave as f64];
                    assert!(render(&params, 4_800).iter().all(|value| value.is_finite()));
                }
            }
        }
    }
}
