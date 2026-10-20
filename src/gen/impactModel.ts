// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// Modal impact synthesis (module audio-collisions, owner M4). DSP-safe: it runs unchanged in
// node (m13 and the build-time bake) and on the main thread (the fallback render of a family).
//
// renderImpact() ADDS three layers into out[0..used) and returns `used`:
//   transient: a raised-cosine contact pulse (high-passed at 150 Hz so it carries no DC) plus
//     high-passed white noise under a short exponential decay; its level is proportional to
//     the velocity;
//   body: the contact pulse (plus a short click whose share grows with velocity x brightness)
//     drives the material's modes. Every mode gets a seeded frequency jitter (1 +/- jitter),
//     a seeded gain jitter, a velocity-dependent spectral tilt and a seeded phase. The phase is
//     made by two ModalBanks tuned alike, one excited one sample later: a sin(theta + w) +
//     b sin(theta) = sin(theta + psi) for a = sin psi / sin w and b = cos psi - a cos w;
//   debris: Crackle grains band-limited to spec.debris.band under an ExpDecay of t60.
// Each layer is peak-normalized to its relative level, the sum to a nominal peak around
// -6 dBFS x SIZE_SCALE.gain. Rendering stops at out.length (with a 10 ms fade) or once every
// layer has fallen below -80 dB. It allocates only at call entry (bake/fallback time, never in
// a process()). The same (opts, sr, rng seed) gives bit-identical output.
import type { MaterialId } from '../contract.ts'
import { Biquad, OnePole } from '../dsp/filters.ts'
import { ExpDecay } from '../dsp/env.ts'
import { ModalBank, type Mode } from '../dsp/modal.ts'
import { Crackle, WhiteNoise } from '../dsp/noise.ts'
import type { AudioRng } from '../dsp/rng.ts'
import { TAU, clamp, finiteOr, lerp } from '../dsp/math.ts'

export type ImpactSize = 'small' | 'medium' | 'large'

export interface MaterialSpec {
  modes: readonly Mode[]
  baseHz: number
  jitter: number
  brightness: number
  transient: { hpHz: number; noise: number }
  debris: { density: number; t60: number; band: readonly [number, number]; level: number }
}

function modesOf(baseHz: number, table: readonly (readonly [number, number, number])[]): readonly Mode[] {
  return table.map(([ratio, t60, gain]) => ({ hz: baseHz * ratio, t60, gain }))
}

/** Mode tables are [ratio to baseHz, t60 s, gain]. */
export const MATERIAL_SPECS: Record<MaterialId, MaterialSpec> = {
  // Plate/bar inharmonic ratios with close pairs that beat; the ring (T60 0.9 s at medium, upper
  // modes down to 0.4 s) dies to -50..-65 dB inside each size's maxSeconds instead of being cut,
  // and stays short enough that the -6 dB metal.small under every pack bump does not pile up.
  metal: {
    baseHz: 700,
    modes: modesOf(700, [
      [1, 0.9, 1],
      [1.013, 0.85, 0.6],
      [2.76, 0.72, 0.9],
      [2.785, 0.68, 0.5],
      [4.19, 0.6, 0.6],
      [5.4, 0.52, 0.7],
      [5.46, 0.5, 0.4],
      [8.93, 0.45, 0.45],
      [13.34, 0.4, 0.3],
      [17.8, 0.4, 0.2],
    ]),
    jitter: 0.02,
    brightness: 0.85,
    transient: { hpHz: 3000, noise: 0.4 },
    debris: { density: 80, t60: 0.25, band: [2500, 9000], level: 0.14 },
  },
  // A hollow mid knock: a cavity mode under a short, damped shell, a clear contact click and a
  // brief rattle.
  plastic: {
    baseHz: 600,
    modes: modesOf(600, [
      [0.62, 0.12, 0.6],
      [1, 0.15, 1],
      [1.58, 0.12, 0.8],
      [2.37, 0.1, 0.65],
      [3.1, 0.08, 0.45],
      [4.3, 0.06, 0.3],
      [5.6, 0.05, 0.18],
    ]),
    jitter: 0.04,
    brightness: 0.55,
    transient: { hpHz: 1200, noise: 0.9 },
    debris: { density: 300, t60: 0.05, band: [1200, 5000], level: 0.2 },
  },
  // Lower, irregular (fibrous) ratios; a dense band-limited crackle of splinters.
  wood: {
    baseHz: 220,
    modes: modesOf(220, [
      [1, 0.3, 1],
      [1.46, 0.25, 0.75],
      [2.1, 0.2, 0.6],
      [2.9, 0.17, 0.5],
      [3.9, 0.14, 0.4],
      [5.3, 0.12, 0.3],
      [7.2, 0.1, 0.2],
    ]),
    jitter: 0.07,
    brightness: 0.35,
    transient: { hpHz: 1200, noise: 0.7 },
    debris: { density: 2500, t60: 0.25, band: [900, 4500], level: 0.5 },
  },
  // Heavily damped low modes (a heavy thud) under a dense 0.7-5 kHz crumble.
  concrete: {
    baseHz: 90,
    modes: modesOf(90, [
      [1, 0.1, 1],
      [1.44, 0.085, 0.8],
      [1.99, 0.07, 0.6],
      [2.5, 0.06, 0.45],
    ]),
    jitter: 0.12,
    brightness: 0.2,
    transient: { hpHz: 800, noise: 0.6 },
    debris: { density: 2500, t60: 0.2, band: [700, 5000], level: 0.38 },
  },
}

export const SIZE_SCALE: Record<ImpactSize, { freq: number; decay: number; gain: number }> = {
  small: { freq: 1.6, decay: 0.7, gain: 0.8 },
  medium: { freq: 1, decay: 1, gain: 1 },
  large: { freq: 0.55, decay: 1.6, gain: 1.15 },
}

export interface ImpactOpts {
  material: MaterialId
  size: ImpactSize
  /** 0..1 */
  velocity: number
  /** Contact duration in seconds, 0.0005..0.008. */
  contact: number
}

/** Nominal peak of a velocity-1 medium impact (about -6 dBFS). */
const NOMINAL_PEAK = 0.5
/** Transient layer peak at velocity 1, relative to the body peak. */
const TRANSIENT_LEVEL = 0.6
/** Contact-noise decay: T60 = contact x NOISE_T60_PER_CONTACT + NOISE_T60_BASE (s), a 15-25 ms
 *  burst the ear reads as the contact scrape/click rather than a sub-millisecond tick. */
const NOISE_T60_PER_CONTACT = 4
const NOISE_T60_BASE = 0.012
/** Width (s) of the short click mixed into the excitation for bright, hard hits. */
const CLICK_SEC = 0.00025
/** Largest seeded rebound, relative to the main contact pulse. */
const BOUNCE_MAX = 0.4
/** The layers' stop threshold: -80 dB. */
const STOP_GAIN = 1e-4
/** Window (samples) over which the body's -80 dB stop is judged (longer than a 40 Hz period). */
const STOP_WINDOW = 2048
/** Fade (s) applied when the render is cut by out.length. */
const FADE_SEC = 0.01
/** Debris attack (s). */
const DEBRIS_ATTACK_SEC = 0.003

function peakOf(x: Float64Array, n: number): number {
  let p = 0
  for (let i = 0; i < n; i++) {
    const a = Math.abs(x[i])
    if (a > p) p = a
  }
  return p
}

/**
 * Renders one impact and ADDS it into out[0..used); returns used. Allocates only here, at
 * call entry. Non-finite options fall back to a medium metal hit at velocity 1, contact 1.5 ms.
 */
export function renderImpact(o: ImpactOpts, sr: number, rng: AudioRng, out: Float32Array): number {
  const n = out.length
  if (!(sr > 1) || !Number.isFinite(sr) || n <= 0) return 0
  const spec = MATERIAL_SPECS[o.material] ?? MATERIAL_SPECS.metal
  const size = SIZE_SCALE[o.size] ?? SIZE_SCALE.medium
  const vel = clamp(finiteOr(o.velocity, 1), 0, 1)
  const contact = clamp(finiteOr(o.contact, 0.0015), 0.0005, 0.008)

  // Excitation: a unit-area raised-cosine contact pulse blended with a unit-area short click,
  // plus a seeded rebound (a weaker second contact 2-8 ms later) that varies the attack shape.
  const w = Math.max(2, Math.round(contact * sr))
  const wc = Math.max(2, Math.round(CLICK_SEC * sr))
  const click = clamp(spec.brightness * vel * vel * 0.5, 0, 0.5)
  const bounceAt = Math.round(rng.range(0.002, 0.008) * sr)
  const bounce = rng.range(0, BOUNCE_MAX)
  const bounceW = Math.max(2, Math.round(w * 0.7))
  const excLen = Math.max(w, wc, bounceAt + bounceW)
  const pulse = new Float64Array(excLen)
  const exc = new Float64Array(excLen)
  let pulseSum = 0
  let clickSum = 0
  for (let i = 0; i < w; i++) {
    pulse[i] = 0.5 - 0.5 * Math.cos((TAU * (i + 0.5)) / w)
    pulseSum += pulse[i]
  }
  for (let i = 0; i < bounceW; i++) pulse[bounceAt + i] += bounce * (0.5 - 0.5 * Math.cos((TAU * (i + 0.5)) / bounceW))
  for (let i = 0; i < wc; i++) clickSum += 0.5 - 0.5 * Math.cos((TAU * (i + 0.5)) / wc)
  for (let i = 0; i < excLen; i++) {
    const c = i < wc ? (0.5 - 0.5 * Math.cos((TAU * (i + 0.5)) / wc)) / clickSum : 0
    exc[i] = ((1 - click) * pulse[i]) / pulseSum + click * c
  }

  // Body: seeded per-mode frequency, decay, gain and phase; velocity x brightness tilt.
  const count = spec.modes.length
  const modesA: Mode[] = new Array(count)
  const modesB: Mode[] = new Array(count)
  const tilt = -0.9 * (1 - vel * clamp(spec.brightness, 0, 1))
  const nyq = 0.45 * sr
  for (let m = 0; m < count; m++) {
    const src = spec.modes[m]
    const hz = src.hz * (1 + spec.jitter * (2 * rng.next() - 1))
    const t60 = src.t60 * (1 + 0.25 * (2 * rng.next() - 1))
    const ratio = Math.max(0.25, hz / spec.baseHz)
    const g = src.gain * (1 + 0.4 * (2 * rng.next() - 1)) * Math.pow(ratio, tilt)
    const psi = (2 * rng.next() - 1) * Math.PI
    const wm = (TAU * hz * size.freq) / sr
    const s = Math.sin(wm)
    let ga = 0
    let gb = g
    if (hz * size.freq < nyq && s > 1e-3) {
      ga = (g * Math.sin(psi)) / s
      gb = g * (Math.cos(psi) - (Math.sin(psi) * Math.cos(wm)) / s)
    }
    modesA[m] = { hz, t60, gain: finiteOr(ga, 0) }
    modesB[m] = { hz, t60, gain: finiteOr(gb, 0) }
  }
  const bankA = new ModalBank(count, sr)
  const bankB = new ModalBank(count, sr)
  bankA.setModes(modesA, size.freq, size.decay)
  bankB.setModes(modesB, size.freq, size.decay)

  // Transient: the DC-free contact pulse plus high-passed noise under a short decay.
  const pulseHp = new Biquad()
  pulseHp.set('highpass', 150, Math.SQRT1_2, 0, sr)
  const noiseHp = new Biquad()
  noiseHp.set('highpass', spec.transient.hpHz * Math.sqrt(size.freq), Math.SQRT1_2, 0, sr)
  const white = new WhiteNoise(rng.fork(1))
  const noiseEnv = new ExpDecay()
  noiseEnv.trigger(1, contact * NOISE_T60_PER_CONTACT + NOISE_T60_BASE, sr)
  const noiseW = spec.transient.noise * 4

  // Debris: Crackle grains band-limited to spec.debris.band under an ExpDecay.
  const crackle = new Crackle(rng.fork(2))
  const debHp = new Biquad()
  debHp.set('highpass', spec.debris.band[0], Math.SQRT1_2, 0, sr)
  const debLp = new Biquad()
  debLp.set('lowpass', spec.debris.band[1], Math.SQRT1_2, 0, sr)
  const debSmooth = new OnePole()
  debSmooth.setLowpass(Math.min(spec.debris.band[1] * 1.5, 0.45 * sr), sr)
  const debEnv = new ExpDecay()
  debEnv.trigger(1, Math.max(0.01, spec.debris.t60 * size.decay), sr)
  const debRate = Math.max(0, spec.debris.density) * (0.5 + 0.5 * vel)
  const debAttack = Math.max(1, Math.round(DEBRIS_ATTACK_SEC * sr))
  // Seeded layer balance (+/-2.5 dB on the transient and the debris).
  const transJit = 1 + 0.3 * (2 * rng.next() - 1)
  const debJit = 1 + 0.3 * (2 * rng.next() - 1)

  const body = new Float64Array(n)
  const trans = new Float64Array(n)
  const deb = new Float64Array(n)
  let used = n
  let stopped = false
  let bodyPeak = 0
  let windowMax = 0
  let prevExc = 0
  for (let i = 0; i < n; i++) {
    const e = i < excLen ? exc[i] : 0
    const b = bankA.process(e) + bankB.process(prevExc)
    prevExc = e
    body[i] = b
    const ab = Math.abs(b)
    if (ab > bodyPeak) bodyPeak = ab
    if (ab > windowMax) windowMax = ab

    const env = noiseEnv.next()
    trans[i] = pulseHp.process(i < excLen ? pulse[i] : 0) + noiseHp.process(white.next()) * env * noiseW

    const de = debEnv.next() * (i < debAttack ? (i + 1) / debAttack : 1)
    deb[i] = debSmooth.process(debLp.process(debHp.process(crackle.next(debRate, sr)))) * de

    if ((i + 1) % STOP_WINDOW === 0 && i > excLen) {
      if (windowMax <= bodyPeak * STOP_GAIN && env <= STOP_GAIN && de <= STOP_GAIN) {
        used = i + 1
        stopped = true
        break
      }
      windowMax = 0
    }
  }

  // Layer balance, then the nominal peak.
  const transPeak = peakOf(trans, used)
  const debPeak = peakOf(deb, used)
  const gBody = bodyPeak > 1e-12 ? 1 / bodyPeak : 0
  const gTrans = transPeak > 1e-12 ? (TRANSIENT_LEVEL * vel * transJit) / transPeak : 0
  const gDeb = debPeak > 1e-12 ? (spec.debris.level * (0.4 + 0.6 * vel) * debJit) / debPeak : 0
  let mixPeak = 0
  for (let i = 0; i < used; i++) {
    const v = finiteOr(body[i] * gBody + trans[i] * gTrans + deb[i] * gDeb, 0)
    body[i] = v
    const a = Math.abs(v)
    if (a > mixPeak) mixPeak = a
  }
  if (!(mixPeak > 1e-12)) return 0
  const target = NOMINAL_PEAK * size.gain * lerp(0.6, 1, vel)
  const scale = target / mixPeak
  if (!stopped) {
    const fade = Math.min(used, Math.max(1, Math.round(FADE_SEC * sr)))
    for (let k = 0; k < fade; k++) {
      body[used - fade + k] *= 0.5 + 0.5 * Math.cos((Math.PI * (k + 1)) / fade)
    }
  }
  // Trim the tail below -80 dB of the peak.
  const floor = mixPeak * STOP_GAIN
  while (used > 1 && Math.abs(body[used - 1]) <= floor) used--
  for (let i = 0; i < used; i++) out[i] += clamp(body[i] * scale, -1, 1)
  return used
}
