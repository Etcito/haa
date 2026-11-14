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

pub const SPEAKERS: usize = 10;
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

/// The 10 virtual speakers, fixed in the world: a ring of 8 at ear level every 45 degrees (index 0
/// toward -z, 2 toward +x), then straight up and straight down.
fn speakers() -> [[f32; 3]; SPEAKERS] {
    let mut s = [[0.0; 3]; SPEAKERS];
    for (k, v) in s.iter_mut().take(RING).enumerate() {
        let a = k as f32 * PI / 4.0;
        *v = [a.sin(), 0.0, -a.cos()];
    }
    s[8] = [0.0, 1.0, 0.0];
    s[9] = [0.0, -1.0, 0.0];
    s
}

/// Speaker gains for a direction: pairwise constant-power panning between the two ring speakers
/// around its azimuth (a source on a speaker lights that speaker alone, so a lateral sound keeps
/// its full interaural cues), the poles by its elevation; `focus` < 1 blends toward all speakers
/// (a large source). Unit energy.
pub fn pan(_spk: &[[f32; 3]; SPEAKERS], d: [f32; 3], focus: f32) -> [f32; SPEAKERS] {
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let mut g = [0.0f32; SPEAKERS];
    if l > 1e-6 {
        let (x, y, z) = (d[0] / l, d[1] / l, d[2] / l);
        let h = (x * x + z * z).sqrt();
        let el = y.atan2(h);
        let mut az = x.atan2(-z);
        if az < 0.0 {
            az += 2.0 * PI;
        }
        let pos = az / (PI / 4.0);
        let i = (pos.floor() as usize) % RING;
        let f = pos - pos.floor();
        let c = el.cos();
        g[i] = (f * PI / 2.0).cos() * c;
        g[(i + 1) % RING] = (f * PI / 2.0).sin() * c;
        if el > 0.0 {
            g[8] = el.sin();
        } else {
            g[9] = -el.sin();
        }
    }
    let focus = if l > 1e-6 { focus.clamp(0.0, 1.0) } else { 0.0 };
    let u = 1.0 / (SPEAKERS as f32).sqrt();
    let mut e = 0.0;
    for x in g.iter_mut() {
        *x = focus * *x + (1.0 - focus) * u;
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

/// Line lengths at 48 kHz (12-36 ms, primes): a dense onset, as the rendered responses had.
const FDN_LEN_48K: [usize; 8] = [557, 709, 853, 1031, 1201, 1361, 1523, 1709];
/// A diffuse field spread over the 10 HRTF speakers reaches each ear at about half the power a
/// stereo response did (measured: the late energy 2.5-4 dB under the convolver bank): +3 dB.
const DIFFUSE_GAIN: f32 = 1.41;

struct Fdn {
    rt: f32,
    lines: Vec<Vec<f32>>,
    pos: [usize; 8],
    g: [f32; 8],
    damp: [f32; 8],
    damp_k: f32,
    out_gain: f32,
    input: Vec<[f32; BLOCK]>,
    fed: bool,
    /// Samples left before the tail is under -90 dB (the step is skipped then).
    alive: usize,
}

impl Fdn {
    fn new(rt: f32, sr: f32) -> Fdn {
        let lens: Vec<usize> = FDN_LEN_48K.iter().map(|&l| ((l as f32) * sr / 48000.0) as usize).collect();
        let mut g = [0.0; 8];
        for i in 0..8 {
            g[i] = 10f32.powf(-3.0 * lens[i] as f32 / (rt * sr));
        }
        let mut f = Fdn {
            rt,
            lines: lens.iter().map(|&l| vec![0.0; l]).collect(),
            pos: [0; 8],
            g,
            damp: [0.0; 8],
            damp_k: coef(7000.0, sr),
            out_gain: 1.0,
            input: vec![[0.0; BLOCK]; 8],
            fed: false,
            alive: 0,
        };
        f.out_gain = DIFFUSE_GAIN / f.impulse_energy(sr).max(1e-9).sqrt();
        f
    }

    /// Energy of the response (all 8 outputs) to a unit-energy impulse spread over the lines.
    fn impulse_energy(&self, sr: f32) -> f32 {
        let mut probe = Fdn { lines: self.lines.iter().map(|l| vec![0.0; l.len()]).collect(), pos: [0; 8], damp: [0.0; 8], out_gain: 1.0, input: vec![[0.0; BLOCK]; 8], fed: true, alive: usize::MAX, ..*self.shallow() };
        let total = ((self.rt * 1.1).min(9.0) * sr) as usize;
        let mut e = 0.0f64;
        let mut out = [0.0f32; SPEAKERS * BLOCK];
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
            e += out.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>();
            done += n;
        }
        e as f32
    }

    fn shallow(&self) -> Box<Fdn> {
        Box::new(Fdn { rt: self.rt, lines: Vec::new(), pos: [0; 8], g: self.g, damp: [0.0; 8], damp_k: self.damp_k, out_gain: 1.0, input: Vec::new(), fed: false, alive: 0 })
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
                self.damp[j] += self.damp_k * (o - self.damp[j]);
                v[j] = self.damp[j] * self.g[j];
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
                let ys = vo.read(delay + lerp(cur[p::PREDELAY], tgt[p::PREDELAY], t) * sr);
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
        // A source on +x lights the ring speaker at +x alone; one at 22.5 degrees shares two.
        let g = pan(&spk, [1.0, 0.0, 0.0], 1.0);
        assert!((g[2] - 1.0).abs() < 1e-4, "{g:?}");
        let a = PI / 8.0;
        let g = pan(&spk, [a.sin(), 0.0, -a.cos()], 1.0);
        assert!((g[0] - g[1]).abs() < 1e-4 && g[0] > 0.6, "{g:?}");
        // Unit-energy reverb steps: a short and a long decay, both within 1.5 dB of 1.
        for rt in [0.5f32, 4.0] {
            let f = Fdn::new(rt, 48000.0);
            let e = f.impulse_energy(48000.0) * f.out_gain * f.out_gain / (DIFFUSE_GAIN * DIFFUSE_GAIN);
            assert!((10.0 * e.log10()).abs() < 1.5, "rt {rt}: {e}");
        }
    }
}
