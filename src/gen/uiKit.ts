// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// uiKit: modern UI synthesis for the menu sounds and race cues (module audio-ui, owner M8).
// DSP-safe: it runs unchanged in node (m13 and the build-time bake) and on the main thread
// (the fallback render of a family). Every function allocates its scratch buffers at call
// entry (bake/proof/fallback time), never inside the sample loops.
//
// No raw oscillators: every tone is a noise-excited ModalBank, an FM pair with a decaying
// index, or an AdditiveBank stack with detune and random phases. Each function renders its
// components into scratch buffers, balances them, scales the sum to a nominal peak of
// -6 dBFS x level, and ADDS it into out from `at` (default 0), never past out.length. It
// returns the end index clamped to out.length. A non-finite sample aborts the function: it
// zeroes its written range and adds nothing. The same options and rng state give
// bit-identical output.
import { Biquad, OnePole, Svf } from '../dsp/filters.ts'
import { TAU, clamp, finiteOr } from '../dsp/math.ts'
import { ModalBank, type Mode } from '../dsp/modal.ts'
import { PinkNoise, WhiteNoise } from '../dsp/noise.ts'
import { AdditiveBank } from '../dsp/osc.ts'
import { FdnReverb } from '../dsp/reverb.ts'
import type { AudioRng } from '../dsp/rng.ts'

/** Nominal output peak at level 1 (-6 dBFS). */
const NOMINAL_PEAK = 0.5
/** ln(1000): exp(-t x this / t60) reaches -60 dB at t = t60. */
const SIXTY_DB = Math.log(1000)
/** The linear end fade of every render (s). */
const END_FADE = 0.005
/** The raised-cosine click that strikes a modal ping (s). */
const STRIKE = 0.0004
/** Frames between two freqScale updates of a drifting tick. */
const DRIFT_STEP = 16

/** Glass modes (ratio, base gain, t60 scale) of softClick. */
const GLASS_MODES = [
  [1, 1, 1],
  [2.32, 0.55, 0.8],
  [4.25, 0.3, 0.6],
  [6.63, 0.18, 0.45],
] as const
/** Plastic modes of softClick: lower, fewer, duller. */
const PLASTIC_CLICK_MODES = [
  [1, 1, 1],
  [2.11, 0.35, 0.7],
  [3.47, 0.12, 0.5],
] as const
/**
 * Bell modes (ratio, base gain, t60 scale) of chime; the gains of modes k >= 1 scale with
 * bright. The upper modes decay only 1.05..4.5x faster than the fundamental, so the note keeps
 * its octave and inharmonic colour through the decay instead of thinning to a sine.
 */
const BELL_MODES = [
  [1, 1, 1],
  [2.0, 0.85, 0.95],
  [2.76, 0.55, 0.7],
  [4.07, 0.35, 0.5],
  [5.4, 0.2, 0.38],
  [8.9, 0.1, 0.22],
] as const
/** Every bell mode has a doublet partner (a split degenerate mode) this much quieter... */
const DOUBLET_GAIN = 0.35
/** ...decaying at this t60 scale, detuned by a seeded 2.5..7 Hz beat. */
const DOUBLET_T60 = 0.85
/**
 * Wood and plastic tick modes. The last mode is the low housing resonance of the wheel
 * (ratio < 1, longer t60) that makes a tick read as a mechanism rather than a pitched blip.
 */
const WOOD_MODES = [
  [1, 1, 1],
  [2.6, 0.5, 0.6],
  [4.1, 0.3, 0.4],
  [0.43, 0.25, 1.6],
] as const
const PLASTIC_TICK_MODES = [
  [1, 1, 1],
  [2.24, 0.5, 0.7],
  [3.62, 0.3, 0.5],
  [0.5, 0.25, 1.4],
] as const
/** The chime's noise strike weight against its deterministic mallet pulse: the pulse keeps
 * the mode balance of the table, the noise varies it per variant. */
const NOISE_STRIKE = 0.35
/** The FM layer's amplitude t60 as a fraction of the bell t60. */
const FM_T60 = 0.6
/** Chowning bell FM modulator ratio. */
const FM_RATIO = 1.4
/**
 * Bell modes above this frequency lose gain as (AIR_HZ / hz)^2 and t60 as AIR_HZ / hz, so the
 * high partials of high notes sparkle in the strike and never ring on as a harsh tail.
 */
const AIR_HZ = 6000
/** The FM index is capped so its highest significant sideband, f + (index + 1) fmod, stays
 * below this: high notes keep a clean sparkle instead of a fizzing, harsh tail. */
const FM_TOP_HZ = 12000
/** The swell's noise is highpassed here, so a cue never carries sub-bass rumble. */
const NOISE_FLOOR_HZ = 250

export type ClickMaterial = 'glass' | 'plastic'
export type TickMaterial = 'wood' | 'plastic'

export interface SoftClickOpts {
  material: ClickMaterial
  /** Frequency ratio on the material's base ping (1 = nominal). */
  pitch: number
  /** 0..1: the noise-to-ping balance and the high-mode gains. */
  brightness: number
  level: number
  /** Start sample in out (default 0). */
  at?: number
}

export interface ChimeOpts {
  /** Note frequencies (Hz), struck in order. */
  notes: readonly number[]
  /** Seconds between two note onsets. */
  spacing: number
  /** 0..1: the FM index and the high bell modes. */
  bright: number
  /** RT60 of the rendered-in reverb tail (s). */
  tail: number
  /** Fundamental bell t60 (s), default 0.6. */
  decay?: number
  /** FM layer weight against the modal bell (default 0.2 + 0.2 bright). */
  fm?: number
  /** FM modulator ratio (default 1.4, the Chowning bell; 3 gives harmonic 2f/4f/5f sidebands
   * with no sub-sideband under the note). */
  fmRatio?: number
  /** Reverb return (default 0.25). */
  wet?: number
  /** Strike noise sent into the reverb only (default 0): a soft, diffuse, non-tonal room tail
   * that carries a short note without letting its tone ring on. */
  air?: number
  /** Default 1. */
  level?: number
  at?: number
}

export interface SwellOpts {
  /** Chord note frequencies (Hz). */
  notes: readonly number[]
  seconds: number
  /** 0..1: the noise share and the partial count. */
  bright: number
  level: number
  at?: number
}

export interface TickOpts {
  material: TickMaterial
  /** Frequency ratio on the material's base (1 = nominal). */
  pitch: number
  /** Fractional pitch change across the tick (-0.5..0.5). */
  drift: number
  level: number
  at?: number
}

/** `at` clamped into [0, len]. */
function startOf(at: number | undefined, len: number): number {
  const s = Math.trunc(finiteOr(at ?? 0, 0))
  return s < 0 ? 0 : s > len ? len : s
}

/** A usable sample rate. */
function goodRate(sr: number): boolean {
  return sr > 1 && Number.isFinite(sr)
}

/** Scratch length for `seconds` from s0, clipped to out; 0 when nothing can be written. */
function spanOf(out: Float32Array, s0: number, seconds: number, sr: number): number {
  if (!(seconds > 0) || !Number.isFinite(seconds)) return 0
  return Math.max(0, Math.min(Math.round(seconds * sr), out.length - s0))
}

/** Hann value at frame j of a w-frame window (raised cosine). */
function hann(j: number, w: number): number {
  return 0.5 - 0.5 * Math.cos((TAU * (j + 0.5)) / w)
}

/** Seeded detune: a ratio of 2^(c / 1200) with c uniform in [-cents, cents]. */
function detune(rng: AudioRng, cents: number): number {
  return Math.pow(2, ((rng.next() * 2 - 1) * cents) / 1200)
}

/** Scales buf to a unit finite peak in place; non-finite samples are left for commit. */
function unitPeak(buf: Float32Array): void {
  let peak = 0
  for (let i = 0; i < buf.length; i++) {
    const a = Math.abs(buf[i])
    if (a > peak) peak = a
  }
  if (!(peak > 0) || !Number.isFinite(peak)) return
  const g = 1 / peak
  for (let i = 0; i < buf.length; i++) buf[i] *= g
}

/**
 * Adds buf into out[s0, s0 + buf.length) scaled to a peak of NOMINAL_PEAK x level, with a
 * 5 ms linear end fade. A non-finite sample zeroes that range instead. Returns the end.
 */
function commit(out: Float32Array, s0: number, buf: Float32Array, level: number, sr: number): number {
  const n = buf.length
  const end = s0 + n
  let peak = 0
  for (let i = 0; i < n; i++) {
    const x = buf[i]
    if (!Number.isFinite(x)) {
      out.fill(0, s0, end)
      return end
    }
    const a = Math.abs(x)
    if (a > peak) peak = a
  }
  const lv = Math.max(0, finiteOr(level, 1))
  if (!(peak > 0) || lv === 0) return end
  const g = (NOMINAL_PEAK * lv) / peak
  const fadeN = Math.max(1, Math.round(END_FADE * sr))
  for (let i = 0; i < n; i++) {
    const remain = n - i
    const fade = remain < fadeN ? remain / fadeN : 1
    out[s0 + i] += buf[i] * g * fade
  }
  return end
}

/** Fills `modes` from a (ratio, gain, t60 scale) table on base hz and t60. */
function loadModes(
  modes: Mode[],
  table: readonly (readonly [number, number, number])[],
  hz: number,
  t60: number,
  highGain: number,
  rng: AudioRng,
  cents: number,
): void {
  modes.length = 0
  for (let k = 0; k < table.length; k++) {
    const [ratio, gain, t60s] = table[k]
    const hg = k === 0 ? 1 : Math.pow(highGain, k)
    modes.push({ hz: hz * ratio * (k === 0 ? 1 : detune(rng, cents)), t60: t60 * t60s, gain: gain * hg })
  }
}

/**
 * A filtered-noise tick plus a short modal ping.
 * - Noise: 1..4 ms (seeded) of WhiteNoise under an exp decay reaching -60 dB at its end,
 *   band-passed by an Svf (q 1.4) at 2.5..6 kHz (rising with brightness, +/-8 % seeded).
 * - Ping: 'glass' has 4 modes (1, 2.32, 4.25, 6.63) on 1.6..3.2 kHz, t60 60..160 ms;
 *   'plastic' 3 modes (1, 2.11, 3.47) on 0.8..1.6 kHz, t60 20..60 ms. Struck by a 0.4 ms
 *   raised-cosine pulse plus the raw burst; high-mode gains scale with brightness.
 * - Mix: noise x (0.25 + 0.75 b), ping x (1 - 0.45 b), each from unit peak.
 * Length: the fundamental t60 plus 5 ms.
 */
export function softClick(out: Float32Array, o: SoftClickOpts, sr: number, rng: AudioRng): number {
  const s0 = startOf(o.at, out.length)
  if (!goodRate(sr)) return s0
  const glass = o.material !== 'plastic'
  const b = clamp(finiteOr(o.brightness, 0.5), 0, 1)
  const pitch = Math.max(0.01, finiteOr(o.pitch, 1))
  const hz = glass
    ? clamp(2200 * pitch * detune(rng, 40), 1600, 3200)
    : clamp(1150 * pitch * detune(rng, 40), 800, 1600)
  const t60 = glass ? 0.06 + 0.1 * (0.35 + 0.65 * rng.next()) * (0.4 + 0.6 * b) : 0.02 + 0.04 * rng.next()
  const n = spanOf(out, s0, t60 + 0.005, sr)
  if (n === 0) return s0
  const burstSec = 0.001 + 0.003 * rng.next()
  const center = clamp((2500 + 3500 * b) * (1 + 0.08 * (rng.next() * 2 - 1)), 2500, 6000)
  const modes: Mode[] = []
  loadModes(modes, glass ? GLASS_MODES : PLASTIC_CLICK_MODES, hz, t60, 0.35 + 0.65 * b, rng, 12)
  const bank = new ModalBank(modes.length, sr)
  bank.setModes(modes)
  const white = new WhiteNoise(rng)
  const svf = new Svf()
  svf.set(center, 1.4, sr)
  const noise = new Float32Array(n)
  const ping = new Float32Array(n)
  const burstN = Math.max(1, Math.round(burstSec * sr))
  const strikeN = Math.max(1, Math.round(STRIKE * sr))
  const k = SIXTY_DB / burstSec
  for (let j = 0; j < n; j++) {
    const w = j < burstN ? white.next() * Math.exp(-(j / sr) * k) : 0
    svf.process(w)
    noise[j] = svf.bp
    const strike = j < strikeN ? hann(j, strikeN) : 0
    ping[j] = bank.process(strike + 0.3 * w)
  }
  unitPeak(noise)
  unitPeak(ping)
  const gn = 0.25 + 0.75 * b
  const gp = 1 - 0.45 * b
  for (let j = 0; j < n; j++) noise[j] = noise[j] * gn + ping[j] * gp
  return commit(out, s0, noise, o.level, sr)
}

/**
 * A wood or plastic roulette tick with pitch drift: a ratchet pawl striking a peg.
 * - Impulse: 0.3..1 ms (seeded) of Hann-windowed WhiteNoise, then a seeded 1.5..4 ms later a
 *   second, 0.3..0.6 weaker strike (the pawl bouncing back onto the peg).
 * - Body: a 4-mode ModalBank, 'wood' (1, 2.6, 4.1 on 900 Hz..2 kHz) or 'plastic'
 *   (1, 2.24, 3.62 on 1.2..2.6 kHz), each plus a low housing mode (0.43 / 0.5), fundamental
 *   t60 12..32 ms (seeded). freqScale glides linearly from 1 to 1 + drift across the tick
 *   (updated every 16 frames; resonator states are kept).
 * - Click: the impulses through a 4 kHz highpass Biquad, 0.5 of the body peak.
 * Length: 1.6 x the fundamental t60 plus 8 ms.
 */
export function tick(out: Float32Array, o: TickOpts, sr: number, rng: AudioRng): number {
  const s0 = startOf(o.at, out.length)
  if (!goodRate(sr)) return s0
  const wood = o.material !== 'plastic'
  const pitch = Math.max(0.01, finiteOr(o.pitch, 1))
  const hz = wood
    ? clamp(1300 * pitch * detune(rng, 30), 900, 2000)
    : clamp(1800 * pitch * detune(rng, 30), 1200, 2600)
  const t60 = 0.012 + 0.02 * rng.next()
  const n = spanOf(out, s0, 1.6 * t60 + 0.008, sr)
  if (n === 0) return s0
  const drift = clamp(finiteOr(o.drift, 0), -0.5, 0.5)
  const impN = Math.max(1, Math.round((0.0003 + 0.0007 * rng.next()) * sr))
  const bounceAt = Math.round((0.0015 + 0.0025 * rng.next()) * sr)
  const bounceAmp = 0.3 + 0.3 * rng.next()
  const modes: Mode[] = []
  loadModes(modes, wood ? WOOD_MODES : PLASTIC_TICK_MODES, hz, t60, 1, rng, 15)
  const bank = new ModalBank(modes.length, sr)
  bank.setModes(modes)
  const white = new WhiteNoise(rng)
  const hp = new Biquad()
  hp.set('highpass', 4000, 0.707, 0, sr)
  const body = new Float32Array(n)
  const click = new Float32Array(n)
  for (let j = 0; j < n; j++) {
    if (drift !== 0 && j > 0 && j % DRIFT_STEP === 0) bank.setModes(modes, 1 + (drift * j) / n)
    const jb = j - bounceAt
    const x = j < impN ? white.next() * hann(j, impN) : jb >= 0 && jb < impN ? bounceAmp * white.next() * hann(jb, impN) : 0
    body[j] = bank.process(x)
    click[j] = hp.process(x)
  }
  unitPeak(body)
  unitPeak(click)
  for (let j = 0; j < n; j++) body[j] += 0.5 * click[j]
  return commit(out, s0, body, o.level, sr)
}

/**
 * Modal and FM bell notes with a rendered-in short reverb tail. Note i (Hz, clamped to
 * [40, 0.4 sr]) starts i x spacing s after the first.
 * - Modal bell: a 0.08..0.23 ms raised-cosine mallet pulse (shorter when brighter) plus 0.35 x
 *   2..4 ms (seeded) of Hann-windowed WhiteNoise through a OnePole lowpass at 3 + 6 bright kHz
 *   excites 6 modes (1, 2.0, 2.76, 4.07, 5.4, 8.9), each detuned +/-6 cents
 *   (seeded), t60 = decay x (1, 0.95, 0.7, 0.5, 0.38, 0.22), high-mode gains scaled by
 *   0.4 + 0.6 bright. Each mode has a doublet partner 2.5..7 Hz above it (seeded), 0.35 as
 *   loud, t60 x 0.85: the slow beating keeps a held note alive instead of a static sine.
 *   Modes above 6 kHz lose gain as (6 kHz / hz)^2 and t60 as 6 kHz / hz.
 * - FM pair: carrier f, modulator fmRatio x f (default 1.4), index (0.4 + 2.6 bright) x exp
 *   decay (reaching 1/e at the bell t60, so the carrier never thins to a bare sine), capped so
 *   f + (index + 1) fmRatio f stays under min(12 kHz, 0.45 sr); amplitude:
 *   1.5 ms attack then -60 dB at 0.6 x the bell t60: the FM is the bright strike, and the
 *   modal body (octave and inharmonic modes) carries the tail, so its carrier never leaves
 *   a sine on the fundamental.
 * - Dry = modal x 0.7 + FM x fm (default 0.2 + 0.2 bright), each from unit peak. It feeds an
 *   FdnReverb (size 0.12, decay tail, damping 4.5 + 3 bright kHz, predelay 8 ms), so the tail
 *   darkens as it fades; the reverb input also takes air (default 0) x the unit-peak strike
 *   noise. out = dry + wet (default 0.25) x outL.
 * Length: the last onset plus max(decay, tail) plus 20 ms.
 */
export function chime(out: Float32Array, o: ChimeOpts, sr: number, rng: AudioRng): number {
  const s0 = startOf(o.at, out.length)
  if (!goodRate(sr) || o.notes.length === 0) return s0
  const b = clamp(finiteOr(o.bright, 0.5), 0, 1)
  const spacing = Math.max(0, finiteOr(o.spacing, 0))
  const tail = clamp(finiteOr(o.tail, 0.4), 0.05, 4)
  const decay = clamp(finiteOr(o.decay ?? 0.6, 0.6), 0.05, 4)
  const gf = clamp(finiteOr(o.fm ?? 0.2 + 0.2 * b, 0.3), 0, 2)
  const wet = clamp(finiteOr(o.wet ?? 0.25, 0.25), 0, 2)
  const fmRatio = clamp(finiteOr(o.fmRatio ?? FM_RATIO, FM_RATIO), 0.5, 8)
  const air = clamp(finiteOr(o.air ?? 0, 0), 0, 4)
  const lastOnset = (o.notes.length - 1) * spacing
  const n = spanOf(out, s0, lastOnset + Math.max(decay, tail) + 0.02, sr)
  if (n === 0) return s0
  const modal = new Float32Array(n)
  const fm = new Float32Array(n)
  const strike = new Float32Array(n)
  const modes: Mode[] = []
  const bank = new ModalBank(2 * BELL_MODES.length, sr)
  const white = new WhiteNoise(rng)
  const lp = new OnePole()
  const nyq = 0.45 * sr
  const ampK = SIXTY_DB / (FM_T60 * decay)
  const idxK = 1 / decay
  const attackN = Math.max(1, Math.round(0.0015 * sr))
  for (let i = 0; i < o.notes.length; i++) {
    const on = Math.round(i * spacing * sr)
    if (on >= n) break
    const f = clamp(finiteOr(o.notes[i], 440), 40, 0.4 * sr)
    loadModes(modes, BELL_MODES, f, decay, 0.4 + 0.6 * b, rng, 6)
    const single = modes.length
    for (let k = 0; k < single; k++) {
      const m = modes[k]
      if (m.hz > AIR_HZ) {
        const r = AIR_HZ / m.hz
        m.gain *= r * r
        m.t60 *= r
      }
      modes.push({ hz: m.hz + 2.5 + 4.5 * rng.next(), t60: m.t60 * DOUBLET_T60, gain: m.gain * DOUBLET_GAIN })
    }
    bank.reset()
    bank.setModes(modes)
    lp.reset()
    lp.setLowpass(3000 + 6000 * b, sr)
    const excN = Math.max(1, Math.round((0.002 + 0.002 * rng.next()) * sr))
    const pulseN = Math.max(1, Math.round((0.00008 + 0.00015 * (1 - b)) * sr))
    const fmod = f * fmRatio
    const maxIndex = Math.max(0, (Math.min(nyq, FM_TOP_HZ) - f) / fmod - 1)
    const index0 = Math.min(0.4 + 2.6 * b, maxIndex)
    let pc = rng.next()
    let pm = rng.next()
    const incC = f / sr
    const incM = fmod / sr
    for (let j = on; j < n; j++) {
      const k = j - on
      const t = k / sr
      const x = (k < pulseN ? hann(k, pulseN) : 0) + (k < excN ? NOISE_STRIKE * lp.process(white.next() * hann(k, excN)) : 0)
      strike[j] += x
      modal[j] += bank.process(x)
      const att = k < attackN ? k / attackN : 1
      const env = att * Math.exp(-t * ampK)
      const index = index0 * Math.exp(-t * idxK)
      fm[j] += env * Math.sin(TAU * pc + index * Math.sin(TAU * pm))
      pc += incC
      if (pc >= 1) pc -= 1
      pm += incM
      if (pm >= 1) pm -= 1
    }
  }
  unitPeak(modal)
  unitPeak(fm)
  unitPeak(strike)
  const verb = new FdnReverb(sr)
  verb.set({ size: 0.12, decay: tail, damping: 4500 + 3000 * b, predelay: 0.008, returnDb: 0 })
  for (let j = 0; j < n; j++) {
    const dry = modal[j] * 0.7 + fm[j] * gf
    const send = dry + air * strike[j]
    verb.process(send, send)
    modal[j] = dry + wet * verb.outL
  }
  return commit(out, s0, modal, o.level ?? 1, sr)
}

/**
 * A noise swell plus a harmonic-stack chord under a rising-then-falling envelope (a raised
 * cosine up to 60 % of `seconds`, then a raised cosine down to 0).
 * - Noise: PinkNoise through an Svf lowpass (q 0.9) whose cutoff rises exponentially from
 *   400 Hz to 5 kHz across the swell, then a 250 Hz OnePole highpass (no sub rumble).
 * - Chord: one AdditiveBank per note of 6 + round(4 bright) partials, amplitude 1/k, ratio
 *   k detuned +/-3 cents (seeded), random phases; f0 wanders slowly by +/-2 cents (seeded
 *   0.1..0.3 Hz), updated per 128-frame block.
 * - Mix: chord x 1 + noise x (0.25 + 0.35 bright), each from unit peak.
 */
export function swell(out: Float32Array, o: SwellOpts, sr: number, rng: AudioRng): number {
  const s0 = startOf(o.at, out.length)
  if (!goodRate(sr)) return s0
  const seconds = finiteOr(o.seconds, 0)
  const n = spanOf(out, s0, seconds, sr)
  if (n === 0) return s0
  const b = clamp(finiteOr(o.bright, 0.5), 0, 1)
  const total = Math.round(seconds * sr)
  const chord = new Float32Array(n)
  const noise = new Float32Array(n)
  const partials = 6 + Math.round(4 * b)
  const block = new Float32Array(128)
  for (let i = 0; i < o.notes.length; i++) {
    const f = clamp(finiteOr(o.notes[i], 220), 20, 0.45 * sr)
    const bank = new AdditiveBank(partials)
    for (let k = 1; k <= partials; k++) bank.setPartial(k - 1, k * detune(rng, 3), 1 / k, rng.next())
    const rate = 0.1 + 0.2 * rng.next()
    const ph = rng.next()
    for (let j = 0; j < n; j += block.length) {
      const m = Math.min(block.length, n - j)
      block.fill(0, 0, m)
      const wander = Math.pow(2, (2 * Math.sin(TAU * (rate * (j / sr) + ph))) / 1200)
      bank.render(f * wander, block, m, sr, 1)
      for (let q = 0; q < m; q++) chord[j + q] += block[q]
    }
  }
  const pink = new PinkNoise(rng)
  const svf = new Svf()
  const hp = new OnePole()
  hp.setHighpass(NOISE_FLOOR_HZ, sr)
  const ratio = 5000 / 400
  for (let j = 0; j < n; j++) {
    svf.set(400 * Math.pow(ratio, j / total), 0.9, sr)
    svf.process(pink.next())
    noise[j] = hp.process(svf.lp)
  }
  unitPeak(chord)
  unitPeak(noise)
  const gn = 0.25 + 0.35 * b
  const peakAt = 0.6
  for (let j = 0; j < n; j++) {
    const x = j / total
    const env =
      x < peakAt ? 0.5 - 0.5 * Math.cos((Math.PI * x) / peakAt) : 0.5 + 0.5 * Math.cos((Math.PI * (x - peakAt)) / (1 - peakAt))
    chord[j] = (chord[j] + noise[j] * gn) * env
  }
  return commit(out, s0, chord, o.level, sr)
}
