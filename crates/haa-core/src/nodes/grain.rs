//! `grain` source node: a cloud of short sine, noise or filtered-noise grains.
//! Ported from Sona v11 `grains.js` (`createGrainState`, `random`, `startGrain`, `grainSample`)
//! and the `grain` lines of `dsp.js` (controlTime, lastTime and the stereo output); params come
//! from catalog.js `NODE_TYPES.grain`, labels from Sona's translations.
//! Deviations:
//! - The generator is `rng::Rng`; `reset` rebuilds it from the stored seed.
//! - The `wave` enum arrives as a rounded index (1 = noise, 2 = filtered-noise), not a string.
//! - The 64 voices are fixed arrays in a boxed struct, so nothing allocates in `process_sample`.
//! - `resonance ?? .7` is not ported: every param is present in the slice.
//! - Live time is `live_frames`; graph.rs passes the live control clock whenever the graph is
//!   live or performance.mode is live (dsp.js:164-212).

use std::f64::consts::{PI, TAU};

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::patch::GraphMode;
use crate::rng::Rng;

const VOICES: usize = 64;

const WAVE: usize = 0;
const FREQUENCY: usize = 1;
const RESONANCE: usize = 2;
const RATE: usize = 3;
const ATTACK: usize = 4;
const DECAY: usize = 5;
const SPREAD: usize = 6;
const JITTER: usize = 7;
const VARIATION: usize = 8;
const GLIDE: usize = 9;
const DELAY: usize = 10;
const WIDTH: usize = 11;
const LEVEL: usize = 12;

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
    unit: &'static str,
    label_en: &'static str,
    label_fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number {
            min,
            max,
            default,
            step,
            scale: Scale::Lin,
        },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

// catalog.js order.
const GRAIN_PARAMS: [ParamDef; 13] = [
    ParamDef {
        key: "wave",
        kind: ParamKind::Enum {
            values: &["sine", "noise", "filtered-noise"],
            default: 0,
        },
        unit: "",
        label_en: "Grain shape",
        label_fr: "Forme du grain",
        modulatable: false,
        smooth: false,
    },
    ParamDef {
        key: "frequency",
        kind: ParamKind::Number {
            min: 40.0,
            max: 12000.0,
            default: 1800.0,
            step: 1.0,
            scale: Scale::Log,
        },
        unit: "Hz",
        label_en: "Pitch",
        label_fr: "Hauteur",
        modulatable: true,
        smooth: true,
    },
    number(
        "resonance",
        0.25,
        4.0,
        0.7,
        0.05,
        "Q",
        "Grain resonance",
        "Résonance du grain",
    ),
    number("rate", 0.0, 2000.0, 12.0, 0.1, "Hz", "Density", "Densité"),
    number(
        "attack", 0.0001, 0.1, 0.001, 0.0001, "s", "Attack", "Attaque",
    ),
    number(
        "decay",
        0.001,
        2.0,
        0.035,
        0.0001,
        "s",
        "Decay",
        "Décroissance",
    ),
    number(
        "spread",
        0.0,
        36.0,
        5.0,
        0.1,
        "st",
        "Pitch spread",
        "Dispersion de hauteur",
    ),
    number(
        "jitter",
        0.0,
        1.0,
        0.8,
        0.01,
        "",
        "Timing variation",
        "Écart de temps",
    ),
    number(
        "variation",
        0.0,
        1.0,
        0.5,
        0.01,
        "",
        "Grain variation",
        "Variation des grains",
    ),
    number(
        "glide",
        -0.9,
        3.0,
        0.05,
        0.01,
        "×",
        "Pitch glide",
        "Glissement de hauteur",
    ),
    number(
        "delay",
        0.0,
        20.0,
        0.02,
        0.001,
        "s",
        "Start delay",
        "Délai initial",
    ),
    number(
        "width",
        0.0,
        1.0,
        0.25,
        0.01,
        "",
        "Stereo spread",
        "Largeur stéréo",
    ),
    number("level", 0.0, 1.0, 0.3, 0.01, "", "Level", "Niveau"),
];

/// The `grain` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "grain",
    category: Category::Source,
    label_en: "Grain cloud",
    label_fr: "Nuage de grains",
    audio_in: false,
    audio_out: true,
    control_out: false,
    uses_curve: false,
    hidden: false,
    params: &GRAIN_PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let sample_rate = if init.sample_rate.is_finite() && init.sample_rate != 0.0 {
        init.sample_rate
    } else {
        48000.0
    };
    Box::new(Grain::new(init.seed, sample_rate))
}

/// Sona's `clamp`: `Math.min(max, Math.max(min, value))`.
fn clamp(value: f64, min: f64, max: f64) -> f64 {
    max.min(min.max(value))
}

/// Sona's `dsp.js` zeroes a non-finite grain channel.
fn finite(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

/// Sona's `createGrainState` fields; the voice arrays are fixed-size.
struct Grain {
    seed: u32,
    rng: Rng,
    sample_rate: f64,
    clock: f64,
    threshold: f64,
    started: bool,
    last_time: f64,
    active: [bool; VOICES],
    phase: [f64; VOICES],
    step: [f64; VOICES],
    target_step: [f64; VOICES],
    glide_rate: [f64; VOICES],
    envelope: [f64; VOICES],
    decay_rate: [f64; VOICES],
    attack_frames: [f64; VOICES],
    age: [f64; VOICES],
    gain: [f64; VOICES],
    left: [f64; VOICES],
    right: [f64; VOICES],
    noise_state: [u32; VOICES],
    filter_g: [f64; VOICES],
    filter_target_g: [f64; VOICES],
    filter_k: [f64; VOICES],
    filter_ic1: [f64; VOICES],
    filter_ic2: [f64; VOICES],
}

impl Grain {
    fn new(seed: u32, sample_rate: f64) -> Grain {
        Grain {
            seed,
            rng: Rng::new(seed),
            sample_rate,
            clock: 0.0,
            threshold: 1.0,
            started: false,
            last_time: -1.0,
            active: [false; VOICES],
            phase: [0.0; VOICES],
            step: [0.0; VOICES],
            target_step: [0.0; VOICES],
            glide_rate: [0.0; VOICES],
            envelope: [0.0; VOICES],
            decay_rate: [0.0; VOICES],
            attack_frames: [0.0; VOICES],
            age: [0.0; VOICES],
            gain: [0.0; VOICES],
            left: [0.0; VOICES],
            right: [0.0; VOICES],
            noise_state: [0; VOICES],
            filter_g: [0.0; VOICES],
            filter_target_g: [0.0; VOICES],
            filter_k: [0.0; VOICES],
            filter_ic1: [0.0; VOICES],
            filter_ic2: [0.0; VOICES],
        }
    }

    fn draw_threshold(&mut self, jitter: f64) -> f64 {
        let u = self.rng.next_f64().max(1e-7);
        0.02_f64.max(1.0 + jitter * (-u.ln() - 1.0))
    }

    fn schedule(&mut self, time: f64, params: &[f64]) {
        if !self.started && time >= params[DELAY] {
            self.start_grain(params);
            self.started = true;
            self.threshold = self.draw_threshold(params[JITTER]);
        } else if self.started && params[RATE] > 0.0 {
            self.clock += params[RATE] / self.sample_rate;
            let mut count = 0;
            while self.clock >= self.threshold && count < 8 {
                count += 1;
                self.clock -= self.threshold;
                self.start_grain(params);
                self.threshold = self.draw_threshold(params[JITTER]);
            }
        }
    }

    /// Takes the quietest voice when none is free; draws pitch, duration, pan, gain, then noise seed.
    fn start_grain(&mut self, params: &[f64]) {
        let sr = self.sample_rate;
        let max_frequency = sr * 0.42;
        let mut index = 0;
        let mut quietest = f64::INFINITY;
        for i in 0..VOICES {
            if !self.active[i] {
                index = i;
                break;
            }
            let level = self.envelope[i] * self.gain[i];
            if level < quietest {
                quietest = level;
                index = i;
            }
        }

        let pitch = 2f64.powf((self.rng.next_f64() * 2.0 - 1.0) * params[SPREAD] / 12.0);
        let frequency = clamp(params[FREQUENCY] * pitch, 20.0, max_frequency);
        let duration =
            params[DECAY] * (1.0 + (self.rng.next_f64() * 2.0 - 1.0) * params[VARIATION] * 0.5);
        let pan = (self.rng.next_f64() * 2.0 - 1.0) * params[WIDTH];
        let step = frequency / sr;
        let target_step = clamp(frequency * (1.0 + params[GLIDE]), 20.0, max_frequency) / sr;

        self.phase[index] = 0.0;
        self.step[index] = step;
        self.target_step[index] = target_step;
        self.glide_rate[index] = 1.0 - (-1.0 / (sr * 0.002_f64.max(duration * 0.4))).exp();
        self.filter_g[index] = (PI * step).tan();
        self.filter_target_g[index] = (PI * target_step).tan();
        self.filter_k[index] = 1.0 / clamp(params[RESONANCE], 0.25, 4.0);
        self.filter_ic1[index] = 0.0;
        self.filter_ic2[index] = 0.0;
        self.envelope[index] = 1.0;
        self.decay_rate[index] = (-1.0 / (sr * duration)).exp();
        self.attack_frames[index] = 1.0_f64.max(params[ATTACK] * sr);

        // Density compensation is frozen at birth, bounded by the voice pool.
        let overlap = 1.0_f64.max((params[RATE] * params[DECAY] * 0.5).min(VOICES as f64));
        self.age[index] = 0.0;
        self.gain[index] = (1.0 - self.rng.next_f64() * params[VARIATION] * 0.7) / overlap.sqrt();
        self.left[index] = ((pan + 1.0) * PI / 4.0).cos();
        self.right[index] = ((pan + 1.0) * PI / 4.0).sin();
        let noise_seed = (self.rng.next_f64() * 4_294_967_295.0) as u32;
        self.noise_state[index] = if noise_seed == 0 { 1 } else { noise_seed };
        self.active[index] = true;
    }

    fn render_voices(&mut self, params: &[f64]) -> (f64, f64) {
        let wave = params[WAVE].round();
        let noise = wave == 1.0 || wave == 2.0;
        let filtered = wave == 2.0;
        let mut sum_l = 0.0;
        let mut sum_r = 0.0;
        for i in 0..VOICES {
            if !self.active[i] {
                continue;
            }
            let age = self.age[i];
            self.age[i] += 1.0;
            let attack = (age / self.attack_frames[i]).min(1.0);
            let envelope = self.envelope[i] * attack * self.gain[i];
            let value = if !noise {
                let value = (TAU * self.phase[i]).sin() * envelope;
                self.phase[i] = (self.phase[i] + self.step[i]) % 1.0;
                self.step[i] += (self.target_step[i] - self.step[i]) * self.glide_rate[i];
                value
            } else {
                // sona: the per-voice xorshift runs on the u32 state; the bits match JS's int32 shifts.
                let mut seed = self.noise_state[i];
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                self.noise_state[i] = seed;
                let white = f64::from(seed) / 2_147_483_648.0 - 1.0;
                if filtered {
                    // A topology-preserving state-variable band-pass; g glides toward the target.
                    let g = self.filter_g[i];
                    let k = self.filter_k[i];
                    let a1 = 1.0 / (1.0 + g * (g + k));
                    let a2 = g * a1;
                    let ic1 = self.filter_ic1[i];
                    let ic2 = self.filter_ic2[i];
                    let v3 = white - ic2;
                    let v1 = a1 * ic1 + a2 * v3;
                    let v2 = ic2 + a2 * ic1 + g * a2 * v3;
                    self.filter_ic1[i] = 2.0 * v1 - ic1;
                    self.filter_ic2[i] = 2.0 * v2 - ic2;
                    self.filter_g[i] += (self.filter_target_g[i] - g) * self.glide_rate[i];
                    v1 * k * envelope
                } else {
                    white * envelope
                }
            };
            sum_l += value * self.left[i];
            sum_r += value * self.right[i];
            self.envelope[i] *= self.decay_rate[i];
            if self.envelope[i] < 0.0001 {
                self.active[i] = false;
            }
        }
        (sum_l, sum_r)
    }
}

impl Node for Grain {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let sr = self.sample_rate;
        let time = match ctx.mode {
            GraphMode::Live => ctx.live_frames as f64 / sr,
            GraphMode::Timeline => ctx.seconds,
        };
        if ctx.perf_rebase {
            self.last_time = time - 1.0 / sr;
        }
        if time < self.last_time {
            self.started = false;
            self.clock = 0.0;
        }
        self.last_time = time;
        self.schedule(time, params);
        let (left, right) = self.render_voices(params);
        NodeOut {
            l: finite(left * params[LEVEL]),
            r: finite(right * params[LEVEL]),
            value: 0.0,
        }
    }

    fn reset(&mut self) {
        *self = Grain::new(self.seed, self.sample_rate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f64 = 48000.0;
    // wave, frequency, resonance, rate, attack, decay, spread, jitter, variation,
    // glide, delay, width, level (DEF order).
    const DEFAULTS: [f64; 13] = [
        0.0, 1800.0, 0.7, 12.0, 0.001, 0.035, 5.0, 0.8, 0.5, 0.05, 0.02, 0.25, 0.3,
    ];

    fn make_node(seed: u32, params: &[f64]) -> Box<dyn Node> {
        (DEF.create)(&NodeInit {
            id: "grain1",
            seed,
            sample_rate: SAMPLE_RATE,
            params,
            texts: &[],
            curve: None,
            inputs: &[],
            duration: 1.0,
        })
    }

    fn ctx(frame: u64, rate: f64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: rate,
            dt: 1.0 / rate,
            t: frame as f64 / rate,
            seconds: frame as f64 / rate,
            duration: 1.0,
            mode: GraphMode::Timeline,
            live_frames: frame,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    #[test]
    fn rate_zero_emits_one_grain_then_silence() {
        let mut params = DEFAULTS;
        params[RATE] = 0.0;
        params[DELAY] = 0.01;
        params[DECAY] = 0.05;
        params[VARIATION] = 0.0;
        let mut node = make_node(123, &params);
        let onset = (params[DELAY] * SAMPLE_RATE) as u64;
        // With duration variation disabled, exp(-age / (sr * decay)) reaches
        // .0001 here. One extra frame allows for rounding of the decay factor.
        let silent_from =
            onset + ((-0.0001_f64.ln() * SAMPLE_RATE * params[DECAY]).ceil() as u64) + 1;
        assert!(silent_from < (SAMPLE_RATE * 0.5) as u64);
        let mut heard_grain = false;
        for frame in 0..SAMPLE_RATE as u64 {
            let out = node.process_sample(&ctx(frame, SAMPLE_RATE), (0.0, 0.0), &params);
            let stereo = (out.l, out.r);
            if frame <= onset || frame >= silent_from {
                assert_eq!(stereo, (0.0, 0.0), "unexpected onset at frame {frame}");
            }
            // The birth sample is zero because both phase and attack start at zero.
            if frame == onset + 1 {
                assert_ne!(stereo, (0.0, 0.0));
            }
            heard_grain |= stereo != (0.0, 0.0);
        }
        assert!(heard_grain);
    }

    #[test]
    fn outputs_stay_finite_at_param_extremes() {
        // 48 cases of .04 s each total 1.92 s of audio.
        for wave in [0.0, 1.0, 2.0] {
            for rate in [0.0, 2000.0] {
                for decay in [0.001, 2.0] {
                    for glide in [-0.9, 3.0] {
                        for resonance in [0.25, 4.0] {
                            let params = [
                                wave, 12000.0, resonance, rate, 0.0001, decay, 36.0, 1.0, 1.0,
                                glide, 0.0, 1.0, 1.0,
                            ];
                            let mut node = make_node(123, &params);
                            let mut heard_grain = false;
                            for frame in 0..1920 {
                                let out = node.process_sample(
                                    &ctx(frame, SAMPLE_RATE),
                                    (0.0, 0.0),
                                    &params,
                                );
                                for value in [out.l, out.r] {
                                    assert!(
                                        value.is_finite() && value.abs() <= 8.0,
                                        "unsafe output {value} at frame {frame}, params {params:?}"
                                    );
                                }
                                assert_eq!(out.value, 0.0);
                                heard_grain |= out.l != 0.0 || out.r != 0.0;
                            }
                            assert!(heard_grain, "silent case: {params:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn same_seed_gives_the_same_cloud() {
        let mut params = DEFAULTS;
        params[WAVE] = 2.0;
        params[RATE] = 200.0;
        params[DELAY] = 0.0;
        let render = |node: &mut dyn Node| -> Vec<(f64, f64)> {
            (0..24000)
                .map(|frame| {
                    let out = node.process_sample(&ctx(frame, SAMPLE_RATE), (0.0, 0.0), &params);
                    (out.l, out.r)
                })
                .collect()
        };
        // Four half-second runs, including replay, total two seconds.
        let mut first_node = make_node(123, &params);
        let first = render(first_node.as_mut());
        assert!(first.iter().any(|&sample| sample != (0.0, 0.0)));
        assert_eq!(first, render(make_node(123, &params).as_mut()));
        assert_ne!(first, render(make_node(456, &params).as_mut()));
        first_node.reset();
        assert_eq!(first, render(first_node.as_mut()));
    }
}
