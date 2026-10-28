// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// fxKit: the small DSP building blocks shared by the gameplay effect families (module
// audio-fx, owner M6). DSP-safe: it runs unchanged in node (m13 and the build-time bake) and
// on the main thread (the fallback render of a family). Never in AudioWorkletGlobalScope
// processor code: every helper allocates at call entry, which is bake/proof/fallback time.
//
// Every ADD helper writes mono into out from `start`, never past out.length, and returns the
// end index clamped to out.length. Every helper returns without writing on a non-finite or
// non-positive `seconds` or sample rate and never throws. The same inputs and rng state give
// bit-identical output.
import { Svf } from '../dsp/filters.ts'
import { TAU, clamp, dbToGain, finiteOr, lerp } from '../dsp/math.ts'
import { ModalBank, type Mode } from '../dsp/modal.ts'
import { PinkNoise, WhiteNoise } from '../dsp/noise.ts'
import { AdditiveBank } from '../dsp/osc.ts'
import type { AudioRng } from '../dsp/rng.ts'
import { Saturator } from '../dsp/shaper.ts'
import { renderImpact, type ImpactOpts } from './impactModel.ts'

/** ln(1000): exp(-t x / this) reaches -60 dB at t = 1. */
const SIXTY_DB = Math.log(1000)
/** The raised-cosine attack of a sweep (s). */
const SWEEP_ATTACK = 0.006
/** The end fade of a stack, a ping and a rendered span (s). */
const END_FADE = 0.005
/** The click width of a resonant ping (s). */
const PING_CLICK = 0.0005
/** The noise burst of a resonant ping (s). */
const PING_NOISE = 0.004
/** The sub-thump pitch glide is this fraction of `seconds`. */
const SUB_GLIDE = 0.4
/** The click pulse of a sub transient (s). */
const SUB_CLICK = 0.001

/** `start` clamped into [0, len]. */
function safeStart(start: number, len: number): number {
  const s = Math.trunc(finiteOr(start, 0))
  return s < 0 ? 0 : s > len ? len : s
}

/** A usable sample rate: a non-finite or tiny rate means the call is a no-op. */
function goodRate(sr: number): boolean {
  return sr > 1 && Number.isFinite(sr)
}

/** A positive, finite duration in seconds. */
function goodSeconds(seconds: number): boolean {
  return seconds > 0 && Number.isFinite(seconds)
}

/** Raised-cosine ramp from 0 at t = 0 to 1 at t = width. */
function raisedCos(t: number, width: number): number {
  if (!(width > 0)) return 1
  if (t <= 0) return 0
  if (t >= width) return 1
  return 0.5 - 0.5 * Math.cos((Math.PI * t) / width)
}

/** Linear-ramped end fade over the last `fadeSamples` of a span of `length` frames. */
function endFade(remain: number, fadeSamples: number): number {
  return remain < fadeSamples ? Math.max(0, remain) / fadeSamples : 1
}

/**
 * Exponential pitch envelope. x = clamp(t / time, 0, 1), s = 1 - (1 - x)^curve (curve > 1
 * moves the change early, < 1 moves it late). Returns from * (to / from)^s when both ends are
 * positive (a constant musical interval), else the linear lerp(from, to, s). time <= 0 -> to.
 */
export function pitchEnvelope(t: number, o: { from: number; to: number; time: number; curve: number }): number {
  if (!goodSeconds(o.time)) return o.to
  const x = clamp(t / o.time, 0, 1)
  const s = 1 - Math.pow(1 - x, finiteOr(o.curve, 1))
  if (o.from > 0 && o.to > 0) return o.from * Math.pow(o.to / o.from, s)
  return lerp(o.from, o.to, s)
}

/**
 * Pink noise through an SVF bandpass whose centre follows pitchEnvelope(f0 -> f1) every
 * sample, clamped to [20, 0.45 sr]. A 6 ms raised-cosine attack, then (1 - x)^1.5 to 0 at
 * `seconds`, all x level. The bandpass output is divided by sqrt(q) so a q change does not
 * move the peak level by more than about 6 dB. With stereoSeed the noise runs on
 * rng.fork(stereoSeed) and leaves rng itself unadvanced; otherwise it draws from rng.
 */
export function noiseSweep(
  out: Float32Array,
  start: number,
  o: { seconds: number; f0: number; f1: number; q: number; curve: number; level: number; stereoSeed?: number },
  sr: number,
  rng: AudioRng,
): number {
  const s0 = safeStart(start, out.length)
  if (!goodSeconds(o.seconds) || !goodRate(sr)) return s0
  const end = Math.min(s0 + Math.round(o.seconds * sr), out.length)
  if (end <= s0) return end
  const pink = new PinkNoise(o.stereoSeed === undefined ? rng : rng.fork(o.stereoSeed))
  const svf = new Svf()
  const q = clamp(finiteOr(o.q, 1), 0.05, 40)
  const norm = 1 / Math.sqrt(q)
  const level = finiteOr(o.level, 1)
  const nyq = 0.45 * sr
  const invSec = 1 / o.seconds
  for (let i = s0; i < end; i++) {
    const t = (i - s0) / sr
    const hz = clamp(pitchEnvelope(t, { from: o.f0, to: o.f1, time: o.seconds, curve: o.curve }), 20, nyq)
    svf.set(hz, q, sr)
    svf.process(pink.next())
    const amp = raisedCos(t, SWEEP_ATTACK) * Math.pow(1 - clamp(t * invSec, 0, 1), 1.5)
    out[i] += svf.bp * norm * amp * level
  }
  return end
}

/**
 * An AdditiveBank of `partials` partials with a pitch glide. Ratio of partial k (1-based) is
 * k * sqrt(1 + inharm k^2) * 2^(d_k / 1200), d_k a seeded offset in +/-detuneCents drawn once
 * per call. Amplitude dbToGain(tiltDb log2(k)) (a negative tilt darkens), phase seeded in
 * [0, TAU). Rendered in 128-frame blocks, f0 per block from pitchEnvelope(f0 -> f1, curve
 * 1.5). Envelope: a raised-cosine attack over `attack` s, then exp(-t / decay), to 0 at
 * `seconds` with a 5 ms fade, all x level. The bank's 0.45 sr band limit handles aliasing.
 */
export function harmonicStack(
  out: Float32Array,
  start: number,
  o: {
    seconds: number
    f0: number
    f1: number
    partials: number
    tiltDb: number
    inharm: number
    detuneCents: number
    attack: number
    decay: number
    level: number
  },
  sr: number,
  rng: AudioRng,
): number {
  const s0 = safeStart(start, out.length)
  if (!goodSeconds(o.seconds) || !goodRate(sr)) return s0
  const end = Math.min(s0 + Math.round(o.seconds * sr), out.length)
  if (end <= s0) return end
  const count = Math.max(1, Math.floor(finiteOr(o.partials, 1)))
  const bank = new AdditiveBank(count)
  const inharm = finiteOr(o.inharm, 0)
  const detune = finiteOr(o.detuneCents, 0)
  const tilt = finiteOr(o.tiltDb, 0)
  for (let k = 1; k <= count; k++) {
    const ratio = k * Math.sqrt(1 + inharm * k * k) * Math.pow(2, ((rng.next() * 2 - 1) * detune) / 1200)
    const amp = dbToGain(tilt * (Math.log(k) / Math.LN2))
    bank.setPartial(k - 1, ratio, amp, rng.next())
  }
  const level = finiteOr(o.level, 1)
  const attack = Math.max(0, finiteOr(o.attack, 0))
  const decay = Math.max(1e-4, finiteOr(o.decay, 0.1))
  const fadeN = Math.max(1, Math.round(END_FADE * sr))
  const nyq = 0.45 * sr
  const scratch = new Float32Array(128)
  let i = s0
  while (i < end) {
    const m = Math.min(scratch.length, end - i)
    scratch.fill(0, 0, m)
    const f0 = clamp(pitchEnvelope((i - s0) / sr, { from: o.f0, to: o.f1, time: o.seconds, curve: 1.5 }), 0, nyq)
    bank.render(f0, scratch, m, sr, 1)
    for (let j = 0; j < m; j++) {
      const t = (i + j - s0) / sr
      const att = attack > 0 ? raisedCos(t, attack) : 1
      const env = att * Math.exp(-t / decay) * endFade(end - (i + j), fadeN) * level
      out[i + j] += scratch[j] * env
    }
    i += m
  }
  return end
}

/**
 * A click plus a pitch-dropping sub thump. Click: a raised-cosine pulse 1 ms wide at
 * 0.5 x level. Thump: a phase-accumulated sine gliding f0 -> f1 (pitchEnvelope, time
 * 0.4 x seconds, curve 2) under an exp decay that reaches -60 dB at `seconds`, x level. It is
 * the only allowed bare sine: it sits under a transient.
 */
export function subTransient(
  out: Float32Array,
  start: number,
  o: { f0: number; f1: number; seconds: number; level: number },
  sr: number,
): number {
  const s0 = safeStart(start, out.length)
  if (!goodSeconds(o.seconds) || !goodRate(sr)) return s0
  const end = Math.min(s0 + Math.round(o.seconds * sr), out.length)
  if (end <= s0) return end
  const level = finiteOr(o.level, 1)
  const clickN = Math.max(1, Math.round(SUB_CLICK * sr))
  const decay = o.seconds / SIXTY_DB
  const glideTime = SUB_GLIDE * o.seconds
  const nyq = 0.45 * sr
  let phase = 0
  for (let i = s0; i < end; i++) {
    const t = (i - s0) / sr
    const hz = clamp(pitchEnvelope(t, { from: o.f0, to: o.f1, time: glideTime, curve: 2 }), 0, nyq)
    phase += hz / sr
    if (phase >= 1 || phase < 0) phase -= Math.floor(phase)
    const thump = Math.sin(TAU * phase) * Math.exp(-t / decay) * level
    const j = i - s0
    const click = j < clickN ? (0.5 - 0.5 * Math.cos((TAU * j) / clickN)) * 0.5 * level : 0
    out[i] += thump + click
  }
  return end
}

/**
 * The excitation drives a 3-mode ModalBank. Modes: hz at gain 1, hz x 2.76 at gain 0.25,
 * hz x 5.40 at gain 0.08; each t60 = min(seconds, 6.91 q / (pi hz)); modes above 0.45 sr are
 * skipped by the bank. Excitation 'click' is a 0.5 ms raised-cosine pulse, 'noise' is 4 ms of
 * white noise under an exp decay. The output is scaled so the first-10 ms peak is about
 * `level`, rendered for `seconds` with a 5 ms end fade.
 */
export function resonantPing(
  out: Float32Array,
  start: number,
  o: { hz: number; q: number; seconds: number; level: number; excite: 'click' | 'noise' },
  sr: number,
  rng: AudioRng,
): number {
  const s0 = safeStart(start, out.length)
  if (!goodSeconds(o.seconds) || !goodRate(sr) || !(o.hz > 0) || !Number.isFinite(o.hz)) return s0
  const end = Math.min(s0 + Math.round(o.seconds * sr), out.length)
  if (end <= s0) return end
  const q = Math.max(0.05, finiteOr(o.q, 1))
  const hz = o.hz
  const t60 = (f: number): number => Math.min(o.seconds, (6.91 * q) / (Math.PI * f))
  const modes: Mode[] = [
    { hz, t60: t60(hz), gain: 1 },
    { hz: hz * 2.76, t60: t60(hz * 2.76), gain: 0.25 },
    { hz: hz * 5.4, t60: t60(hz * 5.4), gain: 0.08 },
  ]
  const bank = new ModalBank(3, sr)
  bank.setModes(modes)
  const m = end - s0
  const exc = new Float32Array(m)
  if (o.excite === 'noise') {
    const white = new WhiteNoise(rng)
    const w = Math.max(1, Math.round(PING_NOISE * sr))
    const dec = PING_NOISE / SIXTY_DB
    const n = Math.min(w, m)
    for (let j = 0; j < n; j++) exc[j] = white.next() * Math.exp(-(j / sr) / dec)
  } else {
    const w = Math.max(1, Math.round(PING_CLICK * sr))
    const n = Math.min(w, m)
    for (let j = 0; j < n; j++) exc[j] = 0.5 - 0.5 * Math.cos((TAU * (j + 0.5)) / w)
  }
  const body = new Float32Array(m)
  const win = Math.min(m, Math.max(1, Math.round(0.01 * sr)))
  let peak = 0
  for (let j = 0; j < m; j++) {
    const y = bank.process(exc[j])
    body[j] = y
    if (j < win) {
      const a = Math.abs(y)
      if (a > peak) peak = a
    }
  }
  const level = finiteOr(o.level, 1)
  const scale = peak > 1e-9 ? level / peak : 0
  const fadeN = Math.max(1, Math.round(END_FADE * sr))
  for (let j = 0; j < m; j++) out[s0 + j] += body[j] * scale * endFade(m - j, fadeN)
  return end
}

/**
 * Wraps renderImpact: it renders into a scratch buffer of length out.length - start, then
 * adds scratch x level from start. Returns start + used, clamped to out.length.
 */
export function modalHit(
  out: Float32Array,
  start: number,
  o: ImpactOpts & { level: number },
  sr: number,
  rng: AudioRng,
): number {
  const s0 = safeStart(start, out.length)
  if (!goodRate(sr)) return s0
  const scratch = new Float32Array(out.length - s0)
  const used = renderImpact(o, sr, rng, scratch)
  const level = finiteOr(o.level, 1)
  const end = Math.min(s0 + used, out.length)
  for (let i = s0; i < end; i++) out[i] += scratch[i - s0] * level
  return end
}

/**
 * A Saturator(factor) with set(drive, 0, 1) processing out[from, to) in place. When drive >
 * 1.5 and factor is 1 the audible aliasing is oversampled: 2x is used instead.
 */
export function shapeBuffer(out: Float32Array, from: number, to: number, drive: number, factor: 1 | 2 | 4): void {
  const a = safeStart(from, out.length)
  const b = safeStart(to, out.length)
  if (b <= a) return
  let f: 1 | 2 | 4 = factor
  if (drive > 1.5 && f === 1) f = 2
  const sat = new Saturator(f)
  sat.set(drive, 0, 1)
  for (let i = a; i < b; i++) out[i] = sat.process(out[i])
}

/**
 * Scales out so max |x| = dbToGain(peakDb). A no-op when the peak is 0 or non-finite or the
 * target is non-positive.
 */
export function normalizePeak(out: Float32Array, peakDb: number): void {
  let peak = 0
  for (let i = 0; i < out.length; i++) {
    const a = Math.abs(out[i])
    if (a > peak) peak = a
  }
  if (!(peak > 0) || !Number.isFinite(peak)) return
  const target = dbToGain(finiteOr(peakDb, 0))
  if (!(target > 0) || !Number.isFinite(target)) return
  const g = target / peak
  for (let i = 0; i < out.length; i++) out[i] *= g
}
