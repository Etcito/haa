// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports, HAA buses (MIX_INPUTS, REVERB_INPUT), per-listener spatial rendering from the sp block replacing the origin pan/gain.
// The one-shot sampler host (DSP-safe). SAMPLER_VOICES preallocated slots read their variant with
// 4-point Hermite interpolation; each listener l < sp[COUNT] gets a one-pole lowpass, equal-power
// pan and gain into the voice's bus pair, and the summed stereo × sp[SEND] goes to REVERB_INPUT.
// process() OVERWRITES the MIX_INPUTS pairs and never allocates.

import { BLOCK, MAX_LISTENERS, MIX_INPUTS, REVERB_INPUT, SAMPLER_VOICES, SP, SP_LENGTH } from '../contract.ts'
import { TAU, clamp, finiteOr } from '../dsp/math.ts'

/** Fade (s) of the old voice when a play replaces an occupied slot. */
const REPLACE_FADE_SEC = 0.005
/** Smoothing time (s) of 'upd' values. */
const SMOOTH_SEC = 0.01
/** Listener gains at or below this are skipped. */
const MIN_GAIN = 1e-5
/** At or above this cutoff the one-pole is bypassed: at 20 kHz / 48 kHz its coefficient is
 *  about 0.073, which is not transparent within 1e-6, so 'open' means no filter. */
const LOWPASS_OPEN_HZ = 20000
const EMPTY = new Float32Array(0)

/** A play request (the fields of SamplerMsg's 'play' variant, without the tag). */
export interface SamplerPlay { voice: number; id: string; variant: number; bus: number; gain: number; rate: number; sp: Float32Array }

interface LoadedFamily { variants: Float32Array[]; rate: number }

/** One preallocated voice slot; every array is allocated at construction. */
interface Voice {
  active: boolean; data: Float32Array; baseStep: number; pos: number; gain: number; bus: number
  rate: number; rateTarget: number; sp: Float64Array; spTarget: Float64Array
  lpA: Float64Array; lpY: Float64Array; cL: Float64Array; cR: Float64Array; send: number
  fade: number; fadeInc: number
  pending: boolean; pData: Float32Array; pBaseStep: number; pGain: number; pBus: number; pRate: number; pSp: Float64Array
}

export class Sampler {
  private readonly sr: number
  private readonly voices: Voice[] = []
  private readonly families = new Map<string, LoadedFamily>()
  private activeCount = 0
  /** Scratch target coefficients for the voice being rendered. */
  private readonly tl = new Float64Array(MAX_LISTENERS)
  private readonly tr = new Float64Array(MAX_LISTENERS)

  constructor(sr: number) {
    this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000
    for (let i = 0; i < SAMPLER_VOICES; i++) {
      const f = () => new Float64Array(MAX_LISTENERS)
      this.voices.push({
        active: false, data: EMPTY, baseStep: 0, pos: 0, gain: 0, bus: 0,
        rate: 1, rateTarget: 1, sp: new Float64Array(SP_LENGTH), spTarget: new Float64Array(SP_LENGTH),
        lpA: f(), lpY: f(), cL: f(), cR: f(), send: 0, fade: 1, fadeInc: 0,
        pending: false, pData: EMPTY, pBaseStep: 0, pGain: 0, pBus: 0, pRate: 1, pSp: new Float64Array(SP_LENGTH)
      })
    }
  }

  /** Number of active voices (including ones fading out). */
  get playing(): number { return this.activeCount }

  /** Store a family's variants and their native sample rate, keeping the arrays by reference. */
  load(id: string, variants: Float32Array[], rate: number): void {
    if (!id || !Array.isArray(variants)) return
    this.families.set(id, { variants, rate: finiteOr(rate, this.sr) })
  }

  /** True when a family id has been loaded. */
  has(id: string): boolean { return this.families.has(id) }

  /** Number of loaded variants of a family (0 when unknown). */
  variantCount(id: string): number { return this.families.get(id)?.variants.length ?? 0 }

  /** Start a voice in slot c.voice; an occupied slot is faded out over 5 ms first.
   *  Unknown ids, variants and slot numbers are ignored. */
  play(c: SamplerPlay): void {
    const fam = this.families.get(c.id)
    const v = this.voices[c.voice | 0]
    const data = fam?.variants[c.variant | 0]
    if (!fam || !v || !data || data.length === 0) return
    v.pData = data; v.pBaseStep = fam.rate / this.sr; v.pGain = Math.max(0, finiteOr(c.gain, 0))
    v.pBus = clamp(c.bus | 0, 0, MIX_INPUTS - 2); v.pRate = finiteOr(c.rate, 1); v.pending = true
    copySp(v.pSp, c.sp)
    if (v.active) v.fadeInc = Math.max(v.fadeInc, 1 / (REPLACE_FADE_SEC * this.sr))
    else this.start(v)
  }

  /** Set new rate and spatial targets for a voice; they are smoothed over 10 ms. */
  update(voice: number, rate: number, sp: Float32Array): void {
    const v = this.voices[voice | 0]
    if (!v) return
    if (v.pending) { v.pRate = finiteOr(rate, v.pRate); copySp(v.pSp, sp); return }
    if (!v.active) return
    v.rateTarget = finiteOr(rate, v.rateTarget)
    copySp(v.spTarget, sp)
  }

  /** Fade a voice out linearly over fade seconds (dropping a pending replacement); a
   *  non-positive fade stops it at once. */
  stop(voice: number, fade: number): void {
    const v = this.voices[voice | 0]
    if (!v || !v.active) return
    v.pending = false
    if (!(fade > 0)) { this.end(v); return }
    v.fadeInc = Math.max(v.fadeInc, 1 / (fade * this.sr))
  }

  /** Fade every active voice out over fade seconds. */
  stopAll(fade: number): void { for (let i = 0; i < SAMPLER_VOICES; i++) this.stop(i, fade) }

  /** True when the slot is free (finished, stopped or never used). */
  voiceDone(voice: number): boolean { return !this.voices[voice | 0]?.active }

  /** OVERWRITES the MIX_INPUTS stereo pairs, then ADDS every voice into its bus and send pairs. */
  process(out: readonly [Float32Array, Float32Array][], n: number): void {
    const len = Math.max(0, Math.min(n | 0, BLOCK))
    zero(out, len)
    if (len === 0) return
    const k = 1 - Math.exp(-len / (SMOOTH_SEC * this.sr))
    const send = out[REVERB_INPUT]
    const { tl, tr } = this
    for (let vi = 0; vi < SAMPLER_VOICES; vi++) {
      const v = this.voices[vi]
      if (!v.active) continue
      const pair = out[v.bus]
      if (!pair || !send) { this.end(v); continue }
      // Smooth the targets once per block; the output coefficients ramp linearly within it.
      v.rate += (v.rateTarget - v.rate) * k
      for (let j = 1; j < SP_LENGTH; j++) v.sp[j] += (v.spTarget[j] - v.sp[j]) * k
      v.sp[SP.COUNT] = v.spTarget[SP.COUNT]
      const count = this.coefs(v, tl, tr)
      const sendTo = clamp(finiteOr(v.sp[SP.SEND], 0), 0, 1)
      const step = v.baseStep * v.rate
      const data = v.data
      let bad = false
      for (let i = 0; i < len; i++) {
        if (!(v.pos < data.length) || !(step > 0)) { this.end(v); break }
        const t = (i + 1) / len
        const raw = read4(data, v.pos) * v.fade
        let sumL = 0, sumR = 0
        for (let l = 0; l < count; l++) {
          const gl = v.cL[l], gr = v.cR[l]
          if (gl === 0 && gr === 0 && tl[l] === 0 && tr[l] === 0) continue
          const a = v.lpA[l]
          const y = a === 0 ? raw : (1 - a) * raw + a * v.lpY[l]
          v.lpY[l] = y
          sumL += y * (gl + (tl[l] - gl) * t)
          sumR += y * (gr + (tr[l] - gr) * t)
        }
        if (!Number.isFinite(sumL) || !Number.isFinite(sumR)) { bad = true; break }
        pair[0][i] += sumL
        pair[1][i] += sumR
        const s = v.send + (sendTo - v.send) * t
        send[0][i] += sumL * s
        send[1][i] += sumR * s
        v.pos += step
        if (v.fadeInc > 0) {
          v.fade = Math.max(0, v.fade - v.fadeInc)
          if (v.fade <= 0) { this.end(v); break }
        }
      }
      if (bad) { v.pending = false; this.end(v); zero(out, len); return }
      for (let l = 0; l < MAX_LISTENERS; l++) { v.cL[l] = tl[l]; v.cR[l] = tr[l] }
      v.send = sendTo
    }
  }

  /** Per-listener target pan × gain coefficients and lowpass coefficients from v.sp; returns N. */
  private coefs(v: Voice, tl: Float64Array, tr: Float64Array): number {
    const count = clamp(v.sp[SP.COUNT] | 0, 0, MAX_LISTENERS)
    for (let l = 0; l < MAX_LISTENERS; l++) {
      const g = l < count ? v.gain * Math.max(0, finiteOr(v.sp[SP.GAIN + l], 0)) : 0
      const a = ((clamp(finiteOr(v.sp[SP.PAN + l], 0), -1, 1) + 1) * Math.PI) / 4
      tl[l] = g > MIN_GAIN ? g * Math.cos(a) : 0
      tr[l] = g > MIN_GAIN ? g * Math.sin(a) : 0
      const hz = finiteOr(v.sp[SP.LOWPASS + l], LOWPASS_OPEN_HZ)
      v.lpA[l] = hz >= LOWPASS_OPEN_HZ ? 0 : Math.exp((-TAU * clamp(hz, 10, 0.49 * this.sr)) / this.sr)
    }
    return count
  }

  /** Start the slot's pending play with its coefficients already at their targets. */
  private start(v: Voice): void {
    if (!v.active) this.activeCount++
    v.active = true
    v.pending = false
    v.data = v.pData; v.baseStep = v.pBaseStep; v.gain = v.pGain; v.bus = v.pBus
    v.rate = v.rateTarget = v.pRate
    v.sp.set(v.pSp); v.spTarget.set(v.pSp)
    v.pos = 0; v.fade = 1; v.fadeInc = 0
    v.lpY.fill(0)
    this.coefs(v, v.cL, v.cR)
    v.send = clamp(finiteOr(v.sp[SP.SEND], 0), 0, 1)
  }

  /** End the slot's voice; a pending replacement starts at once (it sounds from the next block). */
  private end(v: Voice): void {
    if (v.active) { v.active = false; this.activeCount-- }
    v.data = EMPTY
    if (v.pending) this.start(v)
  }
}

/** Copy a spatial block into a slot's array; missing or non-finite entries become 0. */
function copySp(dst: Float64Array, src: Float32Array): void {
  for (let j = 0; j < SP_LENGTH; j++) dst[j] = finiteOr(src?.[j] ?? 0, 0)
}

function zero(out: readonly [Float32Array, Float32Array][], len: number): void {
  for (let b = 0; b < MIX_INPUTS && b < out.length; b++) {
    out[b][0].fill(0, 0, len)
    out[b][1].fill(0, 0, len)
  }
}

/** 4-point (Catmull-Rom) Hermite read at a fractional position, indices clamped to the ends. */
function read4(data: Float32Array, pos: number): number {
  const len = data.length
  const p = pos < 0 ? 0 : pos > len - 1 ? len - 1 : pos
  const i = Math.floor(p)
  const t = p - i
  const i0 = i > 0 ? i - 1 : 0
  const i2 = i + 1 < len ? i + 1 : len - 1
  const i3 = i + 2 < len ? i + 2 : len - 1
  const xm1 = data[i0], x0 = data[i], x1 = data[i2], x2 = data[i3]
  const c = (x1 - xm1) * 0.5
  const w = c + x0 - x1
  const a = w + x0 - x1 + (x2 - x0) * 0.5
  const b = w + a
  return ((a * t - b) * t + c) * t + x0
}
