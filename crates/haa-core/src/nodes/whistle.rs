//! `whistle` source node: a nearly pure tone with breath noise and a little vibrato.
//!
//! A human whistle is a sine-like air tone: the pitch comes from the lip opening, the level
//! and the airy noise from the breath. The node is deliberately simple and takes its pitch and
//! breath from control wires, so a `curve` can play a melody and phrase it.
//!
//! Params: `pitch` (Hz), `breath` (level and noise), `vibrato` (depth), `gain`.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;

const PITCH: usize = 0;
const BREATH: usize = 1;
const VIBRATO: usize = 2;
const GAIN: usize = 3;

/// Vibrato centre rate, Hz.
const VIBRATO_HZ: f64 = 5.0;
/// Vibrato depth at `vibrato = 1`, cents.
const VIBRATO_CENTS: f64 = 30.0;
/// Sine level before the breath envelope.
const TONE_LEVEL: f64 = 0.62;
/// Breath noise level before the breath envelope.
const NOISE_LEVEL: f64 = 0.05;

/// The `whistle` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "whistle",
    category: Category::Source,
    label_en: "Whistle",
    label_fr: "Sifflet",
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
                min: 200.0,
                max: 2_000.0,
                default: 880.0,
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
            key: "breath",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.6,
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
            key: "vibrato",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.15,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Vibrato",
            label_fr: "Vibrato",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "gain",
            kind: ParamKind::Number {
                min: 0.0,
                max: 2.0,
                default: 0.8,
                step: 0.01,
                scale: Scale::Lin,
            },
            unit: "×",
            label_en: "Level",
            label_fr: "Niveau",
            modulatable: true,
            smooth: true,
        },
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Whistle {
        sample_rate: init.sample_rate,
        phase: 0.0,
        lfo: 0.0,
        noise: 0.0,
        rng: Rng::new(init.seed ^ 0x7741_5717),
    })
}

struct Whistle {
    sample_rate: f64,
    phase: f64,
    lfo: f64,
    noise: f64,
    rng: Rng,
}

impl Node for Whistle {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let rate = self.sample_rate.max(1.0);
        let pitch = params[PITCH].clamp(200.0, 2_000.0);
        let breath = params[BREATH].clamp(0.0, 1.0);
        let vibrato = params[VIBRATO].clamp(0.0, 1.0);
        let gain = params[GAIN].clamp(0.0, 2.0);

        self.lfo = (self.lfo + VIBRATO_HZ / rate) % 1.0;
        let cents = VIBRATO_CENTS * vibrato * (core::f64::consts::TAU * self.lfo).sin();
        let frequency = pitch * (cents / 1_200.0).exp2();
        self.phase = (self.phase + frequency / rate) % 1.0;
        let tone = (core::f64::consts::TAU * self.phase).sin();

        // A gentle low-passed breath layer; the engine already smoothed `breath` over 5 ms.
        let white = self.rng.next_f64() * 2.0 - 1.0;
        self.noise += 0.25 * (white - self.noise);

        let sample = (tone * TONE_LEVEL + self.noise * NOISE_LEVEL) * breath * gain;
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
            self.sample_rate = init.sample_rate;
            self.phase = 0.0;
            self.lfo = 0.0;
            self.noise = 0.0;
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.lfo = 0.0;
        self.noise = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    fn make_node(rate: f64, params: &[f64]) -> Box<dyn Node> {
        let texts: [Option<String>; 4] = [None, None, None, None];
        (DEF.create)(&NodeInit {
            id: "whistle1",
            seed: 3,
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

    #[test]
    fn no_breath_is_silent() {
        let params = [880.0, 0.0, 0.2, 1.0];
        for sample in run(48_000.0, &params, 4_800) {
            assert_eq!(sample, 0.0);
        }
    }

    #[test]
    fn tone_is_finite_and_audible() {
        let params = [880.0, 0.7, 0.1, 0.8];
        let output = run(48_000.0, &params, 24_000);
        let rms = (output.iter().map(|x| x * x).sum::<f64>() / output.len() as f64).sqrt();
        assert!(rms > 0.05, "rms was {rms}");
        assert!(output.iter().all(|x| x.is_finite() && x.abs() <= 1.5));
    }

    #[test]
    fn pitch_sets_the_fundamental() {
        let params = [1_000.0, 0.8, 0.0, 1.0];
        let output = run(48_000.0, &params, 24_000);
        let start = 8_000;
        let window = 4_000;
        let signal = &output[start..start + window];
        let mut best_lag = 0;
        let mut best = f64::MIN;
        for lag in 20..200 {
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
        assert!((f0 - 1_000.0).abs() < 30.0, "estimated f0 {f0}");
    }

    #[test]
    fn reset_returns_to_silence() {
        let params = [880.0, 0.8, 0.2, 1.0];
        let mut node = make_node(48_000.0, &params);
        for frame in 0..4_800 {
            let _ = node.process_sample(&ctx(frame, 48_000.0), (0.0, 0.0), &params);
        }
        node.reset();
        let first = node.process_sample(&ctx(0, 48_000.0), (0.0, 0.0), &params);
        // The tone restarts at phase zero but advances one sample before the sine is read;
        // only that first step and the fresh noise sample remain.
        assert!(first.l.abs() < 0.12, "first sample {}", first.l);
    }

    #[test]
    fn extremes_are_finite_at_every_rate() {
        for rate in [8_000.0, 48_000.0, 192_000.0] {
            for params in [
                [200.0, 1.0, 1.0, 2.0],
                [2_000.0, 1.0, 0.0, 1.0],
                [2_000.0, 0.0, 1.0, 0.0],
            ] {
                for sample in run(rate, &params, 8_000) {
                    assert!(sample.is_finite(), "rate {rate} params {params:?}");
                    assert!(sample.abs() <= 1.5);
                }
            }
        }
    }
}
