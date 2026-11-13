//! `amp` effect node: guitar preamp and amp: input gain, two tube-style clipping stages,
//! tone stack, presence and output level. HAA-only node (no Sona counterpart).
//!
//! Chain per channel: 100 Hz high-pass (tightens the low end before clipping) -> input gain ->
//! 4x polyphase oversampling -> stage 1: asymmetric `tanh(x + b) - tanh(b)` (even harmonics from
//! `bias`) -> coupling high-pass -> stage 2: symmetric soft clip -> decimation -> DC blocker ->
//! tone stack (bass 120 Hz shelf, mid 700 Hz peak, treble 2.8 kHz shelf) -> presence (4.5 kHz
//! peak) -> output level. Both clippers use first-order antiderivative antialiasing on top of the
//! oversampling, so a high-gain lead stays clean of audible aliasing. Clean, crunch and lead are
//! settings of `gain` (and the tone), not separate modes.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::dsp::biquad::{Biquad, BiquadKind};
use crate::dsp::dc_blocker::DcBlocker;
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use core::f64::consts::{PI, TAU};

const GAIN: usize = 0;
const BIAS: usize = 1;
const BASS: usize = 2;
const MID: usize = 3;
const TREBLE: usize = 4;
const PRESENCE: usize = 5;
const LEVEL: usize = 6;

/// Oversampling factor.
const OS: usize = 4;
/// Taps per polyphase branch; the prototype low-pass has `OS * TAPS` taps.
const TAPS: usize = 16;
const FIR_LEN: usize = OS * TAPS;
/// Prototype cutoff in cycles per oversampled sample (0.45 of the base Nyquist).
const FIR_CUTOFF: f64 = 0.45 / (2.0 * OS as f64);
/// Stage 2 drive.
const STAGE2: f64 = 1.5;

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
        kind: ParamKind::Number { min, max, default, step, scale: Scale::Lin },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

/// The `amp` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "amp",
    category: Category::Effect,
    label_en: "Guitar amp",
    label_fr: "Ampli guitare",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        number("gain", 0.0, 48.0, 12.0, 0.5, "dB", "Gain", "Gain"),
        number("bias", 0.0, 1.0, 0.3, 0.01, "", "Asymmetry (tube bias)", "Asymétrie (polarisation)"),
        number("bass", -12.0, 12.0, 0.0, 0.5, "dB", "Bass", "Graves"),
        number("mid", -12.0, 12.0, 0.0, 0.5, "dB", "Mids", "Médiums"),
        number("treble", -12.0, 12.0, 0.0, 0.5, "dB", "Treble", "Aigus"),
        number("presence", 0.0, 12.0, 3.0, 0.5, "dB", "Presence", "Présence"),
        number("level", -40.0, 12.0, 0.0, 0.5, "dB", "Output level", "Niveau de sortie"),
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Amp::new(init.sample_rate))
}

fn log_cosh(x: f64) -> f64 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - core::f64::consts::LN_2
}

/// First-order ADAA of `f(x) = tanh(k x + b) - tanh(b)`, antiderivative `logcosh(k x + b)/k - x tanh(b)`.
struct Clipper {
    previous: f64,
}

impl Clipper {
    fn process(&mut self, x: f64, k: f64, b: f64) -> f64 {
        let tb = b.tanh();
        let dx = x - self.previous;
        let y = if dx.abs() > 1e-6 {
            let f = |v: f64| log_cosh(k * v + b) / k - v * tb;
            (f(x) - f(self.previous)) / dx
        } else {
            (k * 0.5 * (x + self.previous) + b).tanh() - tb
        };
        self.previous = x;
        y
    }
}

/// One channel of the amp.
struct Channel {
    pre_hp: Biquad,
    up_hist: [f64; TAPS],
    up_pos: usize,
    down_hist: [f64; FIR_LEN],
    down_pos: usize,
    stage1: Clipper,
    stage2: Clipper,
    couple_x: f64,
    couple_y: f64,
    dc: DcBlocker,
    tone: [Biquad; 4],
}

impl Channel {
    fn new(sample_rate: f64) -> Self {
        let mut pre_hp = Biquad::new();
        pre_hp.set(BiquadKind::Highpass, 100.0, 0.707, 0.0, sample_rate);
        Self {
            pre_hp,
            up_hist: [0.0; TAPS],
            up_pos: 0,
            down_hist: [0.0; FIR_LEN],
            down_pos: 0,
            stage1: Clipper { previous: 0.0 },
            stage2: Clipper { previous: 0.0 },
            couple_x: 0.0,
            couple_y: 0.0,
            dc: DcBlocker::new(sample_rate),
            tone: [Biquad::new(), Biquad::new(), Biquad::new(), Biquad::new()],
        }
    }

    fn reset(&mut self) {
        self.pre_hp.reset();
        self.up_hist = [0.0; TAPS];
        self.up_pos = 0;
        self.down_hist = [0.0; FIR_LEN];
        self.down_pos = 0;
        self.stage1.previous = 0.0;
        self.stage2.previous = 0.0;
        self.couple_x = 0.0;
        self.couple_y = 0.0;
        self.dc.reset();
        for filter in &mut self.tone {
            filter.reset();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process(&mut self, x: f64, fir: &[f64; FIR_LEN], drive: f64, bias: f64, couple: f64, comp: f64) -> f64 {
        let x = self.pre_hp.process(x) * drive;
        self.up_pos = (self.up_pos + TAPS - 1) % TAPS;
        self.up_hist[self.up_pos] = x;
        let mut acc = 0.0;
        for phase in 0..OS {
            // Polyphase interpolation: y[OS n + phase] = OS * sum_k h[OS k + phase] x[n - k].
            let mut up = 0.0;
            for k in 0..TAPS {
                up += fir[OS * k + phase] * self.up_hist[(self.up_pos + k) % TAPS];
            }
            up *= OS as f64;
            let s1 = self.stage1.process(up, 1.0, bias);
            // Coupling capacitor between the stages: one-pole high-pass at about 15 Hz.
            self.couple_y = couple * (self.couple_y + s1 - self.couple_x);
            self.couple_x = s1;
            let s2 = self.stage2.process(self.couple_y, STAGE2, 0.0) / STAGE2;
            self.down_pos = (self.down_pos + FIR_LEN - 1) % FIR_LEN;
            self.down_hist[self.down_pos] = s2;
            if phase == OS - 1 {
                for (j, h) in fir.iter().enumerate() {
                    acc += h * self.down_hist[(self.down_pos + j) % FIR_LEN];
                }
            }
        }
        let mut y = self.dc.process(acc) * comp;
        for filter in &mut self.tone {
            y = filter.process(y);
        }
        y
    }
}

struct Amp {
    sample_rate: f64,
    fir: [f64; FIR_LEN],
    couple: f64,
    left: Channel,
    right: Channel,
    fresh: bool,
}

impl Amp {
    fn new(sample_rate: f64) -> Self {
        let sample_rate = sample_rate.max(1.0);
        // Blackman-windowed sinc prototype, unity DC gain.
        let mut fir = [0.0; FIR_LEN];
        let centre = (FIR_LEN - 1) as f64 / 2.0;
        let mut sum = 0.0;
        for (i, h) in fir.iter_mut().enumerate() {
            let t = i as f64 - centre;
            let sinc = if t.abs() < 1e-12 { 2.0 * FIR_CUTOFF } else { (TAU * FIR_CUTOFF * t).sin() / (PI * t) };
            let n = i as f64 / (FIR_LEN - 1) as f64;
            let window = 0.42 - 0.5 * (TAU * n).cos() + 0.08 * (2.0 * TAU * n).cos();
            *h = sinc * window;
            sum += *h;
        }
        for h in &mut fir {
            *h /= sum;
        }
        let os_rate = sample_rate * OS as f64;
        Self {
            sample_rate,
            fir,
            couple: 1.0 / (1.0 + TAU * 15.0 / os_rate),
            left: Channel::new(sample_rate),
            right: Channel::new(sample_rate),
            fresh: true,
        }
    }

    fn set_tone(&mut self, p: &[f64]) {
        let sr = self.sample_rate;
        for channel in [&mut self.left, &mut self.right] {
            channel.tone[0].set(BiquadKind::Lowshelf, 120.0, 0.707, p[BASS], sr);
            channel.tone[1].set(BiquadKind::Peaking, 700.0, 0.7, p[MID], sr);
            channel.tone[2].set(BiquadKind::Highshelf, 2_800.0, 0.707, p[TREBLE], sr);
            channel.tone[3].set(BiquadKind::Peaking, 4_500.0, 0.8, p[PRESENCE], sr);
        }
    }
}

impl Node for Amp {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, input: (f64, f64), p: &[f64]) -> NodeOut {
        if (ctx.live_frames & 31) == 0 || self.fresh {
            self.set_tone(p);
            self.fresh = false;
        }
        let drive = 10f64.powf(p[GAIN].clamp(0.0, 48.0) / 20.0);
        let bias = 0.8 * p[BIAS].clamp(0.0, 1.0);
        // Saturated stages output about +-1, so the makeup only undoes the first 6 dB of gain.
        let comp = 1.0 / drive.min(2.0) * 10f64.powf(p[LEVEL].clamp(-40.0, 12.0) / 20.0);
        let x_l = if input.0.is_finite() { input.0.clamp(-8.0, 8.0) } else { 0.0 };
        let x_r = if input.1.is_finite() { input.1.clamp(-8.0, 8.0) } else { 0.0 };
        let left = self.left.process(x_l, &self.fir, drive, bias, self.couple, comp);
        let right = self.right.process(x_r, &self.fir, drive, bias, self.couple, comp);
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
            *self = Amp::new(init.sample_rate);
        }
    }

    fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
        self.fresh = true;
    }
}
