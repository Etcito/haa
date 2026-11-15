//! Direct sound and room: a game SFX recording carries the room or the outdoor reflections it was
//! recorded in, and a preset fitted to it learns that tail. In the scene the world's reverb adds its
//! own, so the echo is counted twice (round 16: "l'écho est géré par preset alors que ça devrait être
//! une propriété du monde"). `separate` finds where the recording stops being the source and becomes
//! the room, and keeps the source only.
//!
//! - The room is the recording's late decay: a straight line, in dB, through the RMS envelope (5 ms
//!   frames averaged over 50 ms) from the last frame 15 dB over the noise floor (clear of the floor's
//!   bend) backwards (the last 0.15 s straight within 3 dB RMS), extended while each earlier frame stays within
//!   3 dB of it, at least 0.15 s
//!   long. A room decays exponentially, so its envelope is that line; its RT60 must be one of a space
//!   (0.15-12 s).
//! - A ringing body decays exponentially too (a steel plate, a bell): the late span is a room only if
//!   the averaged spectrum of its late half (at least 0.15 s) has no lines, under a quarter of its energy in bins standing 12 dB over the
//!   spectrum's local median (a sixth of an octave around them). A coloured diffuse tail has none.
//! - The source is everything up to the last moment the envelope (20 ms average) stands 3 dB over the
//!   room's line (a second hit, a bounce, a rattle stay in); from there the recording is the room. The
//!   target is cut there with a 20 ms raised-cosine fade and trimmed. The source's own decay rate (peak
//!   to knee, an EDT-like slope) and the room's are reported.

use crate::fft::{RealFft, hann};

const FRAME_S: f64 = 0.005;
const SMOOTH_FRAMES: usize = 4;
/// The room's line is fitted on a longer average (50 ms): a diffuse tail's 20 ms envelope fluctuates.
const LINE_SMOOTH_FRAMES: usize = 10;
const MIN_SPAN_S: f64 = 0.15;
const MIN_KNEE_S: f64 = 0.02;
const FADE_S: f64 = 0.02;
const MAX_RESIDUAL_DB: f64 = 3.0;
const ABOVE_FLOOR_DB: f64 = 15.0;
const ABOVE_ROOM_DB: f64 = 3.0;
const LINE_PROMINENCE_DB: f64 = 12.0;
const MAX_TONAL_SHARE: f64 = 0.25;

/// What `separate` found.
#[derive(Clone, Copy, Debug)]
pub struct Room {
    /// Where the room takes over, from the start of the recording (s).
    pub knee_s: f64,
    /// The source's own decay, peak to knee (dB/s), and the room tail's (dB/s, negative).
    pub source_db_per_s: f64,
    pub tail_db_per_s: f64,
    /// The tail's RT60 (s), its level at the knee under the peak (dB, negative), and the share of its
    /// energy in spectral lines.
    pub tail_rt60_s: f64,
    pub tail_db: f64,
    pub tonal_share: f64,
}

impl Room {
    pub fn json(&self) -> serde_json::Value {
        let r = |v: f64| (v * 1000.0).round() / 1000.0;
        serde_json::json!({
            "kneeS": r(self.knee_s), "sourceDbPerS": r(self.source_db_per_s), "tailDbPerS": r(self.tail_db_per_s),
            "tailRt60S": r(self.tail_rt60_s), "tailDb": r(self.tail_db), "tonalShare": r(self.tonal_share),
        })
    }
}

/// Least-squares line through (x, y): (slope, intercept, residual RMS).
fn line(x: &[f64], y: &[f64]) -> (f64, f64, f64) {
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
    }
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    let icpt = my - slope * mx;
    let res = (x.iter().zip(y).map(|(a, b)| (b - (icpt + slope * a)).powi(2)).sum::<f64>() / n).sqrt();
    (slope, icpt, res)
}

/// The share of the energy of `x` in spectral lines: bins of its averaged power spectrum (Hann frames
/// of up to 4096 samples) standing `LINE_PROMINENCE_DB` over the median of the sixth of an octave
/// around them.
fn tonal_share(x: &[f32], sample_rate: f64) -> f64 {
    let n = 4096.min((x.len() / 2).next_power_of_two()).max(256);
    if x.len() < n {
        return 0.0;
    }
    let w = hann(n);
    let mut fft = RealFft::new(n);
    let mut mag = vec![0.0; n / 2 + 1];
    let mut power = vec![0.0; n / 2 + 1];
    let mut frame = vec![0.0; n];
    let mut start = 0;
    while start + n <= x.len() {
        for (i, f) in frame.iter_mut().enumerate() {
            *f = x[start + i] as f64 * w[i];
        }
        fft.magnitudes(&frame, &mut mag);
        for (p, m) in power.iter_mut().zip(&mag) {
            *p += m * m;
        }
        start += n / 2;
    }
    let bin_hz = sample_rate / n as f64;
    let total: f64 = power.iter().skip(1).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let ratio = 10f64.powf(LINE_PROMINENCE_DB / 10.0);
    let mut lines = 0.0;
    for k in 1..power.len() {
        let hz = k as f64 * bin_hz;
        let (lo, hi) = ((hz / 2f64.powf(1.0 / 12.0) / bin_hz) as usize, ((hz * 2f64.powf(1.0 / 12.0) / bin_hz).ceil() as usize).min(power.len() - 1));
        let (lo, hi) = (lo.min(k.saturating_sub(3)).max(1), hi.max(k + 3).min(power.len() - 1));
        let mut around: Vec<f64> = power[lo..=hi].to_vec();
        around.sort_by(f64::total_cmp);
        if power[k] > ratio * around[around.len() / 2] {
            lines += power[k];
        }
    }
    lines / total
}

/// The room of `pcm` if its late decay is one, or why not.
pub fn find(pcm: &[f32], sample_rate: f64) -> Result<Room, String> {
    let hop = ((FRAME_S * sample_rate) as usize).max(1);
    let power: Vec<f64> = pcm.chunks(hop).map(|c| c.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / c.len() as f64).collect();
    if power.len() < 2 * LINE_SMOOTH_FRAMES + 8 {
        return Err("too short".into());
    }
    let smooth = |w: usize| -> Vec<f64> {
        (0..power.len())
            .map(|k| {
                let s = &power[k.saturating_sub(w / 2)..(k + w / 2).min(power.len())];
                10.0 * (s.iter().sum::<f64>() / s.len() as f64 + 1e-20).log10()
            })
            .collect()
    };
    let env = smooth(SMOOTH_FRAMES);
    let slow = smooth(LINE_SMOOTH_FRAMES);
    let t: Vec<f64> = (0..env.len()).map(|k| k as f64 * FRAME_S).collect();
    let peak = (0..env.len()).max_by(|&a, &b| env[a].total_cmp(&env[b])).ok_or("empty")?;
    let mut after: Vec<f64> = slow[peak..].to_vec();
    after.sort_by(f64::total_cmp);
    let floor = after[after.len() / 10];
    let end = (peak..env.len()).rev().find(|&k| slow[k] > floor + ABOVE_FLOOR_DB).ok_or("nothing 15 dB over the floor")?;
    let min_span = (MIN_SPAN_S / FRAME_S).ceil() as usize;
    let min_knee = peak + (MIN_KNEE_S / FRAME_S).ceil() as usize;
    if end < min_knee + min_span {
        return Err(format!("decay too short: {:.3} s from the peak to 15 dB over the floor", t[end] - t[peak]));
    }
    // The late line: from `end` backwards, as long as it stays straight.
    let mut start = end - min_span;
    let mut fit = line(&t[start..=end], &slow[start..=end]);
    if fit.2 >= MAX_RESIDUAL_DB {
        return Err(format!("the last {MIN_SPAN_S} s are not a straight decay (residual {:.1} dB)", fit.2));
    }
    while start > min_knee {
        let k = start - 1;
        if (slow[k] - (fit.1 + fit.0 * t[k])).abs() >= MAX_RESIDUAL_DB {
            break;
        }
        start = k;
        fit = line(&t[start..=end], &slow[start..=end]);
    }
    let (slope, icpt, _) = fit;
    let rt60 = if slope < 0.0 { -60.0 / slope } else { f64::INFINITY };
    if !(0.15..=12.0).contains(&rt60) {
        return Err(format!("late decay RT60 {rt60:.2} s is not a space's"));
    }
    // The late half of the span (at least 0.15 s): its start can still hold the source's last ring.
    let late = start + (end - start) / 2;
    let a = (t[late.min(end - min_span)] * sample_rate) as usize;
    let b = (((t[end] + FRAME_S) * sample_rate) as usize).min(pcm.len());
    let tonal = tonal_share(&pcm[a..b], sample_rate);
    if tonal >= MAX_TONAL_SHARE {
        return Err(format!("the late decay rings ({:.0} % of it in spectral lines): the source's own", 100.0 * tonal));
    }
    // The source: up to the last frame standing 3 dB over the room's line.
    let knee = (peak..start).rev().find(|&k| env[k] > icpt + slope * t[k] + ABOVE_ROOM_DB).map_or(min_knee, |k| (k + 1).max(min_knee));
    Ok(Room {
        knee_s: t[knee],
        source_db_per_s: if knee > peak { (env[knee] - env[peak]) / (t[knee] - t[peak]) } else { 0.0 },
        tail_db_per_s: slope,
        tail_rt60_s: rt60,
        tail_db: icpt + slope * t[knee] - env[peak],
        tonal_share: tonal,
    })
}

/// The source of `pcm` alone: cut at the room's knee with a 20 ms fade and trimmed there, or `pcm`
/// unchanged (with the reason) when no room is found.
pub fn separate(pcm: &[f32], sample_rate: f64) -> (Vec<f32>, Result<Room, String>) {
    let room = find(pcm, sample_rate);
    let Ok(r) = room else { return (pcm.to_vec(), room) };
    let start = ((r.knee_s * sample_rate) as usize).min(pcm.len());
    let fade = (FADE_S * sample_rate) as usize;
    let end = (start + fade).min(pcm.len());
    let mut out = pcm[..end].to_vec();
    for (i, x) in out[start..].iter_mut().enumerate() {
        let g = 0.5 + 0.5 * (std::f64::consts::PI * i as f64 / fade as f64).cos();
        *x *= g as f32;
    }
    (out, room)
}
