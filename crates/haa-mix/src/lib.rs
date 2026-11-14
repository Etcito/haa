//! haa-mix: the sfx mixer of the render workers (docs/physics-sound.md, "Mix thread").
//!
//! Every voice a render worker synthesises goes through its spatial chain here, in wasm, and is
//! pre-mixed onto 8 world-fixed virtual speakers (the corners of a cube around the listener): the
//! audio thread then plays those 8 buses through 8 static HRTF panners and rotates them with the
//! listener's orientation, whatever the number of voices.
//!
//! Per voice: a delay line read at the propagation delay (Doppler when it moves), the air
//! absorption shelf (4 kHz), the occlusion bands (shelves at 600 Hz and 2.5 kHz), the direct sound
//! encoded onto the speakers (first-order max-rE panning, narrowed by the source's extent), its
//! unpanned wide part (a large source heard from inside), 4 early-reflection taps encoded at their
//! image directions, and the reverb send (pre-delay, low-pass, weights over the decay steps, input
//! biased toward the diffuse field's arrival direction). The reverb: one 8-line FDN per decay step
//! (the bank's 11 steps, 0.25-8 s), each line feeding one speaker (decorrelated), unit energy like
//! the impulse responses it replaces.

use std::f32::consts::PI;

pub const SPEAKERS: usize = 16;
/// Speakers of the ear-level ring (the FDN lines feed these).
const RING: usize = 8;
pub const BLOCK: usize = 128;
/// Decay steps of the reverb (s): the bank's `BANK_RT60`.
pub const STEPS: [f32; 11] = [0.25, 0.35, 0.5, 0.7, 1.0, 1.4, 2.0, 2.8, 4.0, 5.6, 8.0];
const MAX_DELAY_S: f32 = 0.7;

/// Voice parameter layout (f32), written by the host each block.
pub mod p {
    pub const LEVEL: usize = 0;
    pub const DIR: usize = 1; // 3: unit vector listener -> apparent source, world axes
    pub const DELAY: usize = 4; // s
    pub const LOW: usize = 5; // direct gain below 600 Hz
    pub const MID: usize = 6; // 600 Hz - 2.5 kHz
    pub const HIGH: usize = 7; // above 2.5 kHz
    pub const WIDE: usize = 8; // unpanned share (gain)
    pub const AIR: usize = 9; // linear gain above 4 kHz
    pub const SEND: usize = 10;
    pub const PREDELAY: usize = 11; // s
    pub const SEND_HZ: usize = 12;
    pub const RDIR: usize = 13; // 3: diffuse field arrival direction, length = directionality
    pub const TAP_HZ: usize = 16;
    pub const WEIGHTS: usize = 17; // 11 decay-step weights
    pub const TAPS: usize = 28; // 4 x (dx, dy, dz, delay s, gain)
    pub const FOCUS: usize = 48; // 1 point source .. 0 omni (extent)
    pub const LEN: usize = 50;
}

/// The 16 virtual speakers, fixed in the world: a ring of 8 at ear level every 45 degrees (index 0
/// toward -z, 2 toward +x), 4 above at 45 degrees of elevation and 4 below, between the ring's
/// diagonals. Measured offline against one HRTF per direction (72 directions): ILD error 1.4 dB,
/// ITD 95 us, spectral (elevation) error 2.6 dB, against 3.0 dB, 117 us and 3.7 dB for the ring of 8
/// with two poles; 16 static HRTF panners cost the audio thread 2.6 % of real time.
fn speakers() -> [[f32; 3]; SPEAKERS] {
    let mut s = [[0.0; 3]; SPEAKERS];
    for (k, v) in s.iter_mut().take(RING).enumerate() {
        let a = k as f32 * PI / 4.0;
        *v = [a.sin(), 0.0, -a.cos()];
    }
    let e = PI / 4.0;
    for k in 0..4 {
        let a = PI / 4.0 + k as f32 * PI / 2.0;
        s[RING + k] = [a.sin() * e.cos(), e.sin(), -a.cos() * e.cos()];
        s[RING + 4 + k] = [a.sin() * e.cos(), -e.sin(), -a.cos() * e.cos()];
    }
    s
}

fn det3(m: [[f32; 3]; 3]) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// Speaker gains for a direction: VBAP over the 3 speakers nearest to it (the gains that rebuild the
/// direction from those three, negatives clipped), so a source on a speaker lights that speaker
/// alone and keeps its full interaural and spectral cues; `focus` < 1 blends toward all speakers (a
/// large source). Unit energy.
pub fn pan(spk: &[[f32; 3]; SPEAKERS], d: [f32; 3], focus: f32) -> [f32; SPEAKERS] {
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let mut g = [0.0f32; SPEAKERS];
    if l > 1e-6 {
        let u = [d[0] / l, d[1] / l, d[2] / l];
        let mut near = [(f32::MIN, 0usize); 3];
        for (i, sp) in spk.iter().enumerate() {
            let c = sp[0] * u[0] + sp[1] * u[1] + sp[2] * u[2];
            if c > near[2].0 {
                near[2] = (c, i);
                near.sort_by(|a, b| b.0.total_cmp(&a.0));
            }
        }
        let (a, b, c) = (spk[near[0].1], spk[near[1].1], spk[near[2].1]);
        let m = [[a[0], b[0], c[0]], [a[1], b[1], c[1]], [a[2], b[2], c[2]]];
        let dm = det3(m);
        if dm.abs() < 1e-6 {
            g[near[0].1] = 1.0;
        } else {
            for k in 0..3 {
                let mut mk = m;
                for (r, row) in mk.iter_mut().enumerate() {
                    row[k] = u[r];
                }
                g[near[k].1] = (det3(mk) / dm).max(0.0);
            }
        }
    }
    let focus = if l > 1e-6 { focus.clamp(0.0, 1.0) } else { 0.0 };
    let ud = 1.0 / (SPEAKERS as f32).sqrt();
    let mut e = 0.0;
    let mut eg = 0.0;
    for x in g.iter() {
        eg += x * x;
    }
    let kg = if eg > 1e-12 { 1.0 / eg.sqrt() } else { 0.0 };
    for x in g.iter_mut() {
        *x = focus * *x * kg + (1.0 - focus) * ud;
        e += *x * *x;
    }
    let k = 1.0 / e.max(1e-9).sqrt();
    for x in g.iter_mut() {
        *x *= k;
    }
    g
}

fn coef(hz: f32, sr: f32) -> f32 {
    1.0 - (-2.0 * PI * hz.clamp(10.0, 0.45 * sr) / sr).exp()
}

struct Voice {
    line: Vec<f32>,
    w: usize,
    cur: [f32; p::LEN],
    fresh: bool,
    air: f32,
    b1: f32,
    b2: f32,
    send_lp: f32,
    tap_lp: [f32; 4],
    /// The level, smoothed per sample (10 ms): a mute or a fade never steps.
    level: f32,
    /// Resumed after thinning: fade in from silence instead of starting at the level.
    fade_in: bool,
}

impl Voice {
    fn new(sr: f32) -> Voice {
        Voice { line: vec![0.0; (MAX_DELAY_S * sr) as usize + 4], w: 0, cur: [0.0; p::LEN], fresh: true, air: 0.0, b1: 0.0, b2: 0.0, send_lp: 0.0, tap_lp: [0.0; 4], level: 0.0, fade_in: false }
    }
    fn reset(&mut self, fade_in: bool) {
        self.line.iter_mut().for_each(|x| *x = 0.0);
        self.fresh = true;
        self.fade_in = fade_in;
        self.air = 0.0;
        self.b1 = 0.0;
        self.b2 = 0.0;
        self.send_lp = 0.0;
        self.tap_lp = [0.0; 4];
        self.level = 0.0;
    }
    /// The line read `d` samples behind the last write (linear interpolation).
    #[inline]
    fn read(&self, d: f32) -> f32 {
        let n = self.line.len();
        let d = d.clamp(1.0, (n - 3) as f32);
        let pos = self.w as f32 - d + n as f32;
        let i = pos as usize;
        let f = pos - i as f32;
        let a = self.line[i % n];
        let b = self.line[(i + 1) % n];
        a + (b - a) * f
    }
}

/// Line lengths at 48 kHz (5-35 ms, primes): the first echo 5 ms after the send, as the rendered
/// responses start (2.8-7.4 ms; with 12 ms lines the onset was hollow).
const FDN_LEN_48K: [usize; 8] = [241, 409, 593, 787, 991, 1213, 1433, 1667];
/// The lines' mean length (19 ms): a send reaches the outputs that long after it on average. The
/// field's onset is the room's (its first late paths after the direct sound), so the send is read
/// that much sooner: timed from the send, the hall's field arrived 19 ms late and its first 50 ms held
/// 3 dB less than Barron's estimate for the room (C50 -8.7 dB for -5.7).
const FDN_ONSET_S: f32 = (241 + 409 + 593 + 787 + 991 + 1213 + 1433 + 1667) as f32 / 8.0 / 48000.0;
/// The engine's diffuse level is relative to the direct sound at 2 m, and that direct sound reaches
/// each ear through an HRTF panner at -5.0 dB; a diffuse field of unit total power over the 8 ring
/// speakers reaches each ear at -5.3 dB (both measured offline in Chrome): +0.3 dB. (It was +5.3 dB,
/// matched to the former convolver bank, whose stereo response reached the ears at 0 dB, 5 dB above
/// the engine's own estimate: a ball in the hall came out 5 dB wetter than its physics.)
const DIFFUSE_GAIN: f32 = 1.035;
/// Air absorption sets the upper bands' decay (ISO 9613-1, 20 C, 50 % RH: 29.7 dB/km at 4 kHz, 105 at
/// 8 kHz; on its own a room decays in 0.161 / 4m: 5.9 s at 4 kHz, 1.7 s at 8 kHz). A band decays at
/// 1 / (1 / RT + 1 / RT_air): a measured large hard hall (Hamilton Mausoleum, 9000 m3) rings 0.28 x
/// its 500 Hz time at 4 kHz and 0.12 x at 8 kHz; the FDN rang 0.52 x and 0.43 x (3.7 s at 8 kHz in
/// an 8 s hall, above what air allows).
const AIR_RT_4K: f32 = 5.9;
const AIR_RT_8K: f32 = 1.7;
/// The low band's decay relative to the mid (below 250 Hz): the measured hard halls ring longer
/// there (1.5 x at 125 Hz); the walls' estimate is a mid value, kept as it is below.
const LOW_RT_RATIO: f32 = 1.0;
const SHELF_LOW_HZ: f32 = 250.0;
const SHELF_4K_HZ: f32 = 2800.0;
const SHELF_8K_HZ: f32 = 6500.0;

struct Fdn {
    rt: f32,
    lines: Vec<Vec<f32>>,
    pos: [usize; 8],
    /// Mid-band loop gain per line, and the low/high bands' gains relative to it (shelves in the loop).
    g: [f32; 8],
    gl: [f32; 8],
    gh: [f32; 8],
    g8: [f32; 8],
    lp_lo: [f32; 8],
    lp_hi: [f32; 8],
    lp_8k: [f32; 8],
    k_lo: f32,
    k_hi: f32,
    k_8k: f32,
    out_gain: f32,
    input: Vec<[f32; BLOCK]>,
    fed: bool,
    /// Samples left before the tail is under -90 dB (the step is skipped then).
    alive: usize,
}

impl Fdn {
    fn gains(lens: &[usize], rt: f32, sr: f32) -> [f32; 8] {
        let mut g = [0.0; 8];
        for i in 0..8 {
            g[i] = 10f32.powf(-3.0 * lens[i] as f32 / (rt * sr));
        }
        g
    }

    /// Mid, low, 4 kHz and 8 kHz loop gains for a decay `rt` (the upper bands with air absorption),
    /// every band's decay scaled by `scale` (the correction pass). A first-order shelf of gain g only
    /// reaches g far above its corner: at a frequency f it applies the fraction Re(1 - LP(f)) of its
    /// loss in dB (LP the one-pole low-pass of the shelf, as sampled), and each shelf leaks into the
    /// other band. An octave's decay is its slowest part's, its lower edge: the two shelves' losses
    /// solve the 2 x 2 system that gives each band its target there (without it an 8 s hall rang 4.0 s
    /// at 4 kHz and 2.7 s at 8 kHz for 3.4 s and 1.4 s).
    fn band_gains(lens: &[usize], rt: f32, scale: f32, sr: f32) -> ([f32; 8], [f32; 8], [f32; 8], [f32; 8]) {
        let g = Self::gains(lens, rt * scale, sr);
        let lo = Self::gains(lens, rt * LOW_RT_RATIO * scale, sr);
        let h4 = Self::gains(lens, scale / (1.0 / rt + 1.0 / AIR_RT_4K), sr);
        let h8 = Self::gains(lens, scale / (1.0 / rt + 1.0 / AIR_RT_8K), sr);
        let phi = |f: f32, corner: f32| {
            // LP(w) = k / (1 - (1 - k) e^-jw); Re(1 - LP).
            let k = coef(corner, sr);
            let w = 2.0 * PI * f / sr;
            let (dr, di) = (1.0 - (1.0 - k) * w.cos(), (1.0 - k) * w.sin());
            1.0 - k * dr / (dr * dr + di * di)
        };
        let (f4, f8) = (4000.0 / 2f32.sqrt().sqrt(), 8000.0 / 2f32.sqrt());
        let (a44, a48) = (phi(f4, SHELF_4K_HZ), phi(f4, SHELF_8K_HZ));
        let (a84, a88) = (phi(f8, SHELF_4K_HZ), phi(f8, SHELF_8K_HZ));
        let det = a44 * a88 - a48 * a84;
        let mut gl = [1.0; 8];
        let mut gh = [1.0; 8];
        let mut g8 = [1.0; 8];
        for i in 0..8 {
            gl[i] = (lo[i] / g[i]).min(1.0 / g[i]);
            // Log losses wanted at 4 and 8 kHz, relative to the mid band.
            let (l4, l8) = ((h4[i] / g[i]).ln(), (h8[i] / g[i]).ln());
            let a = ((l4 * a88 - l8 * a48) / det).min(0.0);
            let b = ((l8 - a84 * a) / a88).min(0.0);
            gh[i] = a.exp();
            g8[i] = b.exp();
        }
        (g, gl, gh, g8)
    }

    fn new(rt: f32, sr: f32) -> Fdn {
        let lens: Vec<usize> = FDN_LEN_48K.iter().map(|&l| ((l as f32) * sr / 48000.0) as usize).collect();
        let (g, gl, gh, g8) = Self::band_gains(&lens, rt, 1.0, sr);
        let mut f = Fdn {
            rt,
            lines: lens.iter().map(|&l| vec![0.0; l]).collect(),
            pos: [0; 8],
            g,
            gl,
            gh,
            g8,
            lp_lo: [0.0; 8],
            lp_hi: [0.0; 8],
            lp_8k: [0.0; 8],
            k_lo: coef(SHELF_LOW_HZ, sr),
            k_hi: coef(SHELF_4K_HZ, sr),
            k_8k: coef(SHELF_8K_HZ, sr),
            out_gain: 1.0,
            input: vec![[0.0; BLOCK]; 8],
            fed: false,
            alive: 0,
        };
        // One pass that measures the mid band's late T20 (500 Hz-1 kHz, where the walls' estimate is
        // made) and scales every band's decay for the target.
        let (_, t20) = f.impulse_response(sr);
        if t20 > 0.0 && (t20 / rt - 1.0).abs() > 0.02 {
            let (g, gl, gh, g8) = Self::band_gains(&lens, rt, rt / t20, sr);
            f.g = g;
            f.gl = gl;
            f.gh = gh;
            f.g8 = g8;
        }
        // The level is set in the mid band, where the walls' estimate is made: a band's diffuse energy
        // grows with its decay time, so the bands that air absorbs are quieter, not the others louder.
        // (It was the broadband energy: an 8 s hall whose highs die in 1.4-3.4 s lost three quarters of
        // a white response's energy, and the normalisation gave it back to the lows and mids, +6 dB.)
        let (e, _) = f.impulse_response(sr);
        f.out_gain = DIFFUSE_GAIN / e.max(1e-9).sqrt();
        f
    }

    /// Mid-band energy (500 Hz-1 kHz, relative to the same band of a white unit-energy impulse) and
    /// mid-band Schroeder T20 (x3) of the response (all outputs) to a unit-energy impulse spread over
    /// the lines, simulated to 1.3 times the target decay (at most 11 s).
    fn impulse_response(&self, sr: f32) -> (f32, f32) {
        let mut probe = Fdn { lines: self.lines.iter().map(|l| vec![0.0; l.len()]).collect(), pos: [0; 8], lp_lo: [0.0; 8], lp_hi: [0.0; 8], lp_8k: [0.0; 8], out_gain: 1.0, input: vec![[0.0; BLOCK]; 8], fed: true, alive: usize::MAX, ..*self.shallow() };
        probe.g = self.g;
        probe.gl = self.gl;
        probe.gh = self.gh;
        probe.g8 = self.g8;
        // The mid band of each output (one-pole high-pass 500 Hz, low-pass 1 kHz) for the decay.
        let (ka, kb) = (coef(500.0, sr), coef(1000.0, sr));
        let mut hp_state = [0.0f32; SPEAKERS];
        let mut lp_state = [0.0f32; SPEAKERS];
        let mut env_mid: Vec<f64> = Vec::with_capacity(((self.rt * 1.3).min(11.0) * sr) as usize / BLOCK + 1);
        let total = ((self.rt * 1.3).min(11.0) * sr) as usize;
        let mut out = vec![0.0f32; SPEAKERS * BLOCK];
        let mut done = 0;
        while done < total {
            let n = BLOCK.min(total - done);
            if done == 0 {
                for j in 0..8 {
                    probe.input[j][0] = 1.0 / 8f32.sqrt();
                }
            }
            out.iter_mut().for_each(|x| *x = 0.0);
            probe.process(&mut out, n);
            let mut em = 0.0f64;
            for c in 0..SPEAKERS {
                for i in 0..n {
                    let x = out[c * BLOCK + i];
                    hp_state[c] += ka * (x - hp_state[c]);
                    let h = x - hp_state[c];
                    lp_state[c] += kb * (h - lp_state[c]);
                    em += f64::from(lp_state[c]) * f64::from(lp_state[c]);
                }
            }
            env_mid.push(em);
            done += n;
        }
        // The mid-band filter's share of a white unit impulse's energy.
        let (mut a, mut b, mut white) = (0.0f32, 0.0f32, 0.0f64);
        for i in 0..(sr as usize / 4) {
            let x = if i == 0 { 1.0 } else { 0.0 };
            a += ka * (x - a);
            b += kb * ((x - a) - b);
            white += f64::from(b) * f64::from(b);
        }
        let e = env_mid.iter().sum::<f64>() / white.max(1e-12);
        let env = env_mid;
        // Schroeder integral from 50 ms (the bank's responses were calibrated on their late decay),
        // T20 from -5 to -25 dB.
        let skip = ((0.05 * sr) as usize / BLOCK).min(env.len().saturating_sub(1));
        let late = &env[skip..];
        let mut sch = vec![0.0f64; late.len()];
        let mut acc = 0.0;
        for i in (0..late.len()).rev() {
            acc += late[i];
            sch[i] = acc;
        }
        let at = |db: f64| -> f32 {
            let lim = sch[0] * 10f64.powf(db / 10.0);
            sch.iter().position(|&v| v < lim).map_or(f32::NAN, |i| (i * BLOCK) as f32 / sr)
        };
        (e as f32, (at(-25.0) - at(-5.0)) * 3.0)
    }

    fn shallow(&self) -> Box<Fdn> {
        Box::new(Fdn { rt: self.rt, lines: Vec::new(), pos: [0; 8], g: self.g, gl: self.gl, gh: self.gh, g8: self.g8, lp_lo: [0.0; 8], lp_hi: [0.0; 8], lp_8k: [0.0; 8], k_lo: self.k_lo, k_hi: self.k_hi, k_8k: self.k_8k, out_gain: 1.0, input: Vec::new(), fed: false, alive: 0 })
    }

    /// Run `n` samples: inputs from `self.input`, outputs added onto `bus` (speaker-major planes).
    fn process(&mut self, bus: &mut [f32], n: usize) {
        let s = 1.0 / 8f32.sqrt();
        for i in 0..n {
            let mut v = [0.0f32; 8];
            for j in 0..8 {
                let l = &self.lines[j];
                let o = l[self.pos[j]];
                bus[j * BLOCK + i] += o * self.out_gain;
                // Low shelf (DC gain gl, 1 above 250 Hz), then high shelf (1 below 3 kHz, gh above):
                // first order, monotone, never above the largest band gain: stable.
                self.lp_lo[j] += self.k_lo * (o - self.lp_lo[j]);
                let x1 = o + (self.gl[j] - 1.0) * self.lp_lo[j];
                self.lp_hi[j] += self.k_hi * (x1 - self.lp_hi[j]);
                let x2 = self.lp_hi[j] + self.gh[j] * (x1 - self.lp_hi[j]);
                self.lp_8k[j] += self.k_8k * (x2 - self.lp_8k[j]);
                let x3 = self.lp_8k[j] + self.g8[j] * (x2 - self.lp_8k[j]);
                v[j] = x3 * self.g[j];
            }
            // 8-point Hadamard (fast Walsh-Hadamard), orthonormal.
            for h in [1usize, 2, 4] {
                for a in (0..8).step_by(h * 2) {
                    for b in a..a + h {
                        let (x, y) = (v[b], v[b + h]);
                        v[b] = x + y;
                        v[b + h] = x - y;
                    }
                }
            }
            for j in 0..8 {
                let len = self.lines[j].len();
                self.lines[j][self.pos[j]] = self.input[j][i] + v[j] * s;
                self.pos[j] = (self.pos[j] + 1) % len;
            }
        }
        for row in self.input.iter_mut() {
            row[..n].iter_mut().for_each(|x| *x = 0.0);
        }
    }
}

pub struct Mixer {
    sr: f32,
    spk: [[f32; 3]; SPEAKERS],
    voices: Vec<Voice>,
    params: Vec<[f32; p::LEN]>,
    fdn: Vec<Fdn>,
    pub input: [f32; BLOCK],
    pub bus: Vec<f32>,
}

impl Mixer {
    pub fn new(sr: f32, voices: usize) -> Mixer {
        Mixer {
            sr,
            spk: speakers(),
            voices: (0..voices).map(|_| Voice::new(sr)).collect(),
            params: vec![[0.0; p::LEN]; voices],
            fdn: STEPS.iter().map(|&rt| Fdn::new(rt, sr)).collect(),
            input: [0.0; BLOCK],
            bus: vec![0.0; SPEAKERS * BLOCK],
        }
    }

    pub fn params_mut(&mut self, v: usize) -> &mut [f32; p::LEN] {
        &mut self.params[v]
    }

    /// A new voice (starts at its level) or, `fade_in`, one resumed after thinning (its delay line is
    /// stale: cleared, and the level fades in over 10 ms).
    pub fn reset_voice(&mut self, v: usize, fade_in: bool) {
        if let Some(x) = self.voices.get_mut(v) {
            x.reset(fade_in);
        }
    }

    /// Mix `n` samples of voice `v` (its mono block in `self.input`) into the bus and the reverb inputs.
    pub fn voice(&mut self, v: usize, n: usize) {
        let sr = self.sr;
        let Some(vo) = self.voices.get_mut(v) else { return };
        let tgt = self.params[v];
        if vo.fresh {
            vo.cur = tgt;
            vo.level = if vo.fade_in { 0.0 } else { tgt[p::LEVEL] };
            vo.fresh = false;
        }
        let kl = coef(16.0, sr);
        let level_tgt = tgt[p::LEVEL];
        let cur = vo.cur;
        let g0 = pan(&self.spk, [cur[p::DIR], cur[p::DIR + 1], cur[p::DIR + 2]], cur[p::FOCUS]);
        let g1 = pan(&self.spk, [tgt[p::DIR], tgt[p::DIR + 1], tgt[p::DIR + 2]], tgt[p::FOCUS]);
        let mut t0 = [[0.0f32; SPEAKERS]; 4];
        let mut t1 = [[0.0f32; SPEAKERS]; 4];
        for k in 0..4 {
            let b = p::TAPS + k * 5;
            t0[k] = pan(&self.spk, [cur[b], cur[b + 1], cur[b + 2]], 1.0);
            t1[k] = pan(&self.spk, [tgt[b], tgt[b + 1], tgt[b + 2]], 1.0);
        }
        // Reverb input weights per FDN line: biased toward the diffuse field's direction.
        let rd = [tgt[p::RDIR], tgt[p::RDIR + 1], tgt[p::RDIR + 2]];
        let mut lw = [0.0f32; 8];
        let mut le = 0.0;
        for j in 0..RING {
            lw[j] = (1.0 + 0.8 * (rd[0] * self.spk[j][0] + rd[1] * self.spk[j][1] + rd[2] * self.spk[j][2])).max(0.0);
            le += lw[j] * lw[j];
        }
        let lk = 1.0 / le.max(1e-9).sqrt();
        lw.iter_mut().for_each(|x| *x *= lk);
        let (ka, k1, k2) = (coef(4000.0, sr), coef(600.0, sr), coef(2500.0, sr));
        let ks = coef(tgt[p::SEND_HZ].max(200.0), sr);
        let kt = coef(tgt[p::TAP_HZ].max(200.0), sr);
        let omni = 1.0 / (SPEAKERS as f32).sqrt();
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let fed_steps: Vec<(usize, f32, f32)> = (0..STEPS.len())
            .filter_map(|k| {
                let (a, b) = (cur[p::WEIGHTS + k], tgt[p::WEIGHTS + k]);
                if a.max(b) > 1e-4 && tgt[p::SEND].max(cur[p::SEND]) > 1e-5 { Some((k, a, b)) } else { None }
            })
            .collect();
        for &(k, _, _) in &fed_steps {
            self.fdn[k].fed = true;
        }
        let bus = &mut self.bus;
        for i in 0..n {
            let t = (i + 1) as f32 / n as f32;
            let len = vo.line.len();
            vo.line[vo.w] = self.input[i];
            vo.w = (vo.w + 1) % len;
            vo.level += kl * (level_tgt - vo.level);
            let level = vo.level;
            let delay = lerp(cur[p::DELAY], tgt[p::DELAY], t) * sr;
            let y = vo.read(delay);
            vo.air += ka * (y - vo.air);
            let y2 = vo.air + lerp(cur[p::AIR], tgt[p::AIR], t) * (y - vo.air);
            vo.b1 += k1 * (y2 - vo.b1);
            vo.b2 += k2 * (y2 - vo.b2);
            let z = lerp(cur[p::LOW], tgt[p::LOW], t) * vo.b1 + lerp(cur[p::MID], tgt[p::MID], t) * (vo.b2 - vo.b1) + lerp(cur[p::HIGH], tgt[p::HIGH], t) * (y2 - vo.b2);
            let z = z * level;
            let wide = y2 * lerp(cur[p::WIDE], tgt[p::WIDE], t) * level * omni;
            for s in 0..SPEAKERS {
                bus[s * BLOCK + i] += z * lerp(g0[s], g1[s], t) + wide;
            }
            for k in 0..4 {
                let b = p::TAPS + k * 5;
                let gain = lerp(cur[b + 4], tgt[b + 4], t);
                if gain.abs() < 1e-6 {
                    continue;
                }
                let yt = vo.read(lerp(cur[b + 3], tgt[b + 3], t) * sr);
                vo.tap_lp[k] += kt * (yt - vo.tap_lp[k]);
                let val = vo.tap_lp[k] * gain * level;
                for s in 0..SPEAKERS {
                    bus[s * BLOCK + i] += val * lerp(t0[k][s], t1[k][s], t);
                }
            }
            if !fed_steps.is_empty() {
                let ys = vo.read(delay + (lerp(cur[p::PREDELAY], tgt[p::PREDELAY], t) - FDN_ONSET_S).max(0.0) * sr);
                vo.send_lp += ks * (ys - vo.send_lp);
                let u = vo.send_lp * lerp(cur[p::SEND], tgt[p::SEND], t) * level;
                for &(k, a, b) in &fed_steps {
                    let x = u * lerp(a, b, t);
                    let f = &mut self.fdn[k];
                    for j in 0..8 {
                        f.input[j][i] += x * lw[j];
                    }
                }
            }
        }
        vo.cur = tgt;
    }

    /// Run the reverb steps that are fed or still ringing; the bus then holds the block (8 planes of
    /// `BLOCK`). The host reads it, then calls `clear`.
    pub fn finish(&mut self, n: usize) {
        let sr = self.sr;
        for f in self.fdn.iter_mut() {
            if f.fed {
                f.alive = ((f.rt * 1.5 + 0.2) * sr) as usize;
                f.fed = false;
            }
            if f.alive == 0 {
                continue;
            }
            f.process(&mut self.bus, n);
            f.alive = f.alive.saturating_sub(n);
        }
    }

    pub fn clear(&mut self) {
        self.bus.iter_mut().for_each(|x| *x = 0.0);
    }

    /// Reverb steps running now (fed or ringing).
    pub fn active_steps(&self) -> usize {
        self.fdn.iter().filter(|f| f.alive > 0).count()
    }
}

// --- wasm ABI --------------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
mod abi {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static M: RefCell<Option<Mixer>> = const { RefCell::new(None) };
    }

    fn with<R>(f: impl FnOnce(&mut Mixer) -> R) -> R {
        M.with(|m| f(m.borrow_mut().as_mut().expect("mix_init first")))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn mix_init(sample_rate: f32, voices: u32) {
        M.with(|m| *m.borrow_mut() = Some(Mixer::new(sample_rate, voices as usize)));
    }

    /// Where the host writes voice `v`'s parameters (`p::LEN` f32).
    #[unsafe(no_mangle)]
    pub extern "C" fn mix_params(v: u32) -> u32 {
        with(|m| m.params_mut(v as usize).as_mut_ptr() as u32)
    }

    /// Where the host writes a voice's mono block (`BLOCK` f32) before `mix_voice`.
    #[unsafe(no_mangle)]
    pub extern "C" fn mix_input() -> u32 {
        with(|m| m.input.as_mut_ptr() as u32)
    }

    /// The bus: 8 planes of `BLOCK` f32.
    #[unsafe(no_mangle)]
    pub extern "C" fn mix_bus() -> u32 {
        with(|m| m.bus.as_mut_ptr() as u32)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn mix_reset_voice(v: u32, fade_in: u32) {
        with(|m| m.reset_voice(v as usize, fade_in != 0));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn mix_voice(v: u32, n: u32) {
        with(|m| m.voice(v as usize, (n as usize).min(BLOCK)));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn mix_finish(n: u32) {
        with(|m| m.finish((n as usize).min(BLOCK)));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn mix_clear() {
        with(|m| m.clear());
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn mix_active_steps() -> u32 {
        with(|m| m.active_steps() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panning_keeps_energy_and_points_the_way() {
        let spk = speakers();
        for d in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, -0.2, 0.9], [0.0, 0.0, 0.0]] {
            let g = pan(&spk, d, 1.0);
            let e: f32 = g.iter().map(|x| x * x).sum();
            assert!((e - 1.0).abs() < 1e-4);
        }
        // A source on +x lights the ring speaker at +x alone; one straight up shares the 4 upper ones
        // that the nearest-3 rule picks (3 of them), none below.
        let g = pan(&spk, [1.0, 0.0, 0.0], 1.0);
        assert!((g[2] - 1.0).abs() < 1e-4, "{g:?}");
        let g = pan(&spk, [0.0, 1.0, 0.0], 1.0);
        assert!(g[RING..RING + 4].iter().sum::<f32>() > 1.0 && g[RING + 4..].iter().all(|&x| x == 0.0), "{g:?}");
        // Unit-energy reverb steps: a short and a long decay, both within 1.5 dB of 1.
        for rt in [0.5f32, 4.0, 8.0] {
            let f = Fdn::new(rt, 48000.0);
            let (e, t20) = f.impulse_response(48000.0);
            let e = e * f.out_gain * f.out_gain / (DIFFUSE_GAIN * DIFFUSE_GAIN);
            assert!((10.0 * e.log10()).abs() < 1.0, "rt {rt}: {e}");
            assert!((t20 / rt - 1.0).abs() < 0.08, "rt {rt}: T20 {t20}");
        }
    }
}
