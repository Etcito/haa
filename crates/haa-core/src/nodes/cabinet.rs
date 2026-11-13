//! `cabinet` effect node: guitar speaker cabinet and microphone, built from fixed biquads (no
//! impulse-response files). HAA-only node (no Sona counterpart).
//!
//! Per voicing: speaker low-end high-pass, low resonance (box thump), a low-mid peak or dip, cone
//! breakup presence, a 4th-order low-pass near 5 kHz (the steep speaker roll-off) and a dip near
//! 8.5 kHz (dust cap / mic off-axis). `mic` moves from the cone centre (0, brighter) to the edge
//! (1, darker); `distance` trades the close-mic proximity bass for a floor reflection (short
//! comb). Each channel is filtered on its own, so a stereo input stays stereo.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::biquad::{Biquad, BiquadKind};
use crate::dsp::delay_line::DelayLine;
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const VOICING: usize = 0;
const MIC: usize = 1;
const DISTANCE: usize = 2;
const MIX: usize = 3;

const FILTERS: usize = 8;
/// Longest floor reflection, seconds.
const MAX_REFLECTION_S: f64 = 0.004;

/// One cabinet voicing: (frequency Hz, Q, gain dB) per band.
struct Voicing {
    high_pass: (f64, f64),
    resonance: (f64, f64, f64),
    low_mid: (f64, f64, f64),
    presence: (f64, f64, f64),
    low_pass: (f64, f64),
    level_db: f64,
}

const VOICINGS: [Voicing; 3] = [
    // open1x12: open back, loose low end, bright and airy.
    Voicing {
        high_pass: (70.0, 0.8),
        resonance: (110.0, 1.4, 3.0),
        low_mid: (520.0, 0.9, 1.5),
        presence: (2_400.0, 1.2, 4.0),
        low_pass: (5_000.0, 1.0),
        level_db: -1.0,
    },
    // closed4x12: sealed box, tight thump, scooped low mids, earlier roll-off.
    Voicing {
        high_pass: (75.0, 1.1),
        resonance: (105.0, 1.8, 5.0),
        low_mid: (420.0, 1.0, -3.0),
        presence: (2_000.0, 1.0, 5.0),
        low_pass: (4_400.0, 1.2),
        level_db: -2.0,
    },
    // combo2x12: open-back combo, mid-forward.
    Voicing {
        high_pass: (85.0, 0.8),
        resonance: (130.0, 1.2, 2.0),
        low_mid: (650.0, 1.0, 2.0),
        presence: (2_800.0, 1.3, 3.0),
        low_pass: (4_700.0, 1.0),
        level_db: -1.0,
    },
];

const fn number(key: &'static str, default: f64, label_en: &'static str, label_fr: &'static str) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number { min: 0.0, max: 1.0, default, step: 0.01, scale: Scale::Lin },
        unit: "",
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

/// The `cabinet` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "cabinet",
    category: Category::Effect,
    label_en: "Speaker cabinet",
    label_fr: "Baffle guitare",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "voicing",
            kind: ParamKind::Enum { values: &["open1x12", "closed4x12", "combo2x12"], default: 0 },
            unit: "",
            label_en: "Cabinet",
            label_fr: "Baffle",
            modulatable: false,
            smooth: false,
        },
        number("mic", 0.3, "Mic position (centre to edge)", "Position du micro (centre vers bord)"),
        number("distance", 0.2, "Mic distance", "Distance du micro"),
        number("mix", 1.0, "Mix", "Mélange"),
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Cabinet::new(init.sample_rate))
}

struct Channel {
    filters: [Biquad; FILTERS],
    reflection: DelayLine,
}

impl Channel {
    fn new(sample_rate: f64) -> Self {
        Self {
            filters: core::array::from_fn(|_| Biquad::new()),
            reflection: DelayLine::new(MAX_REFLECTION_S, sample_rate),
        }
    }
}

struct Cabinet {
    sample_rate: f64,
    left: Channel,
    right: Channel,
    reflection_samples: f64,
    reflection_gain: f64,
    level: f64,
    fresh: bool,
}

impl Cabinet {
    fn new(sample_rate: f64) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            sample_rate,
            left: Channel::new(sample_rate),
            right: Channel::new(sample_rate),
            reflection_samples: 1.0,
            reflection_gain: 0.0,
            level: 1.0,
            fresh: true,
        }
    }

    fn update(&mut self, p: &[f64]) {
        let v = &VOICINGS[(p[VOICING].round() as usize).min(VOICINGS.len() - 1)];
        let mic = p[MIC].clamp(0.0, 1.0);
        let distance = p[DISTANCE].clamp(0.0, 1.0);
        let sr = self.sample_rate;
        let low_pass_hz = v.low_pass.0 * (1.25 - 0.6 * mic);
        let presence_db = v.presence.2 * (1.25 - 0.9 * mic);
        let proximity_db = 6.0 * (1.0 - distance) - 2.0 * distance;
        for channel in [&mut self.left, &mut self.right] {
            let f = &mut channel.filters;
            f[0].set(BiquadKind::Highpass, v.high_pass.0, v.high_pass.1, 0.0, sr);
            f[1].set(BiquadKind::Peaking, v.resonance.0, v.resonance.1, v.resonance.2, sr);
            f[2].set(BiquadKind::Peaking, v.low_mid.0, v.low_mid.1, v.low_mid.2, sr);
            f[3].set(BiquadKind::Peaking, v.presence.0, v.presence.1, presence_db, sr);
            f[4].set(BiquadKind::Lowpass, low_pass_hz, v.low_pass.1, 0.0, sr);
            f[5].set(BiquadKind::Lowpass, low_pass_hz, 0.54, 0.0, sr);
            f[6].set(BiquadKind::Lowshelf, 180.0, 0.707, proximity_db, sr);
            f[7].set(BiquadKind::Peaking, 8_500.0, 0.8, -9.0, sr);
        }
        self.reflection_samples = ((0.0003 + 0.0027 * distance) * sr).max(1.0);
        self.reflection_gain = 0.1 + 0.35 * distance;
        self.level = 10f64.powf(v.level_db / 20.0) / (1.0 + self.reflection_gain);
    }

    fn channel(channel: &mut Channel, x: f64, delay: f64, gain: f64) -> f64 {
        let mut y = x;
        for filter in &mut channel.filters {
            y = filter.process(y);
        }
        channel.reflection.write(y);
        y + gain * channel.reflection.read(delay)
    }
}

impl Node for Cabinet {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, input: (f64, f64), p: &[f64]) -> NodeOut {
        if (ctx.live_frames & 31) == 0 || self.fresh {
            self.update(p);
            self.fresh = false;
        }
        let mix = p[MIX].clamp(0.0, 1.0);
        let x_l = if input.0.is_finite() { input.0 } else { 0.0 };
        let x_r = if input.1.is_finite() { input.1 } else { 0.0 };
        let (delay, gain, level) = (self.reflection_samples, self.reflection_gain, self.level);
        let wet_l = Self::channel(&mut self.left, x_l, delay, gain) * level;
        let wet_r = Self::channel(&mut self.right, x_r, delay, gain) * level;
        let left = x_l * (1.0 - mix) + wet_l * mix;
        let right = x_r * (1.0 - mix) + wet_r * mix;
        let (left, right) = if left.is_finite() && right.is_finite() {
            (left, right)
        } else {
            self.reset();
            (0.0, 0.0)
        };
        NodeOut { l: left, r: right, value: (left + right) * 0.5 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        if (init.sample_rate - self.sample_rate).abs() > 0.5 {
            *self = Cabinet::new(init.sample_rate);
        }
    }

    fn reset(&mut self) {
        for channel in [&mut self.left, &mut self.right] {
            for filter in &mut channel.filters {
                filter.reset();
            }
            channel.reflection.reset();
        }
        self.fresh = true;
    }
}
