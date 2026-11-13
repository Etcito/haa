// HAA bed host: runs one BedVoice with smoothed params, an optional per-listener spatial
// stage (lowpass, equal-power pan, gain), a reverb send, a linear end fade and a
// non-finite guard. DSP-safe; runs in the 'haa-bed' processor and in the offline scene.
// Every buffer is preallocated at construction: process() never allocates.

import { BLOCK, MAX_LISTENERS, REVERB_SEND_BASE, SP, SP_LENGTH } from '../contract.ts'
import type { BedDef, BedVoice } from '../contract.ts'
import { OnePole } from '../dsp/filters.ts'
import { dbToGain, finiteOr } from '../dsp/math.ts'

/** Smoothing time constant of every param not listed in def.unsmoothed, in seconds. */
const PARAM_SMOOTH_SEC = 0.015
/** Listeners at or below this gain are skipped by the positional stage. */
const MIN_LISTENER_GAIN = 1e-5
/** Lowpass cutoff of an unfiltered listener, in Hz. */
const OPEN_LOWPASS_HZ = 20000

/** One procedural bed instance: params, spatial block, fade and guard around a BedVoice. */
export class BedHost {
  private readonly sr: number
  private readonly def: BedDef
  private readonly voice: BedVoice
  private readonly gain: number
  private readonly target: Float32Array
  private readonly current: Float32Array
  private readonly smooth: Uint8Array
  private readonly coef: number
  private readonly dopplerIndex: number
  private readonly sp = new Float32Array(SP_LENGTH)
  private readonly lowpass: OnePole[] = []
  private readonly tmpL = new Float32Array(BLOCK)
  private readonly tmpR = new Float32Array(BLOCK)
  private readonly tmpMono = new Float32Array(BLOCK)
  private rate = 1
  private fadeLeft = -1
  private fadeStep = 0
  private fadeGain = 1
  private ended = false

  /** Create the voice from def.create(sr, seed); trimDb applies to the voice output. */
  constructor(sr: number, def: BedDef, seed: number, trimDb = 0) {
    this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000
    this.def = def
    this.voice = def.create(this.sr, seed)
    this.gain = dbToGain(finiteOr(trimDb, 0))
    const n = def.params.length
    this.target = new Float32Array(n)
    this.current = new Float32Array(n)
    this.smooth = new Uint8Array(n)
    for (let i = 0; i < n; i++) {
      this.target[i] = this.current[i] = def.defaults[i]
      this.smooth[i] = def.unsmoothed?.includes(def.params[i]) ? 0 : 1
    }
    this.coef = Math.exp(-BLOCK / (PARAM_SMOOTH_SEC * this.sr))
    this.dopplerIndex = def.params.indexOf('doppler')
    for (let l = 0; l < MAX_LISTENERS; l++) this.lowpass.push(new OnePole())
    this.flatSpatial()
  }

  /** True once end() was called and its fade has completed; the output stays silent. */
  get done(): boolean {
    return this.ended
  }

  /** Set target values in def.params order; non-finite values are ignored. */
  setParams(v: ArrayLike<number>): void {
    const n = Math.min(v.length, this.target.length)
    for (let i = 0; i < n; i++) {
      const x = v[i]
      if (!Number.isFinite(x)) continue
      this.target[i] = x
      if (!this.smooth[i]) this.current[i] = x
    }
  }

  /** Copy the spatial block and the Doppler rate (written into a 'doppler' param when present). */
  setSpatial(rate: number, sp: Float32Array): void {
    const n = Math.min(sp.length, SP_LENGTH)
    for (let i = 0; i < n; i++) this.sp[i] = finiteOr(sp[i], 0)
    this.rate = finiteOr(rate, 1)
  }

  /** Ramp linearly to silence over fadeSec seconds, then report done. */
  end(fadeSec: number): void {
    if (this.ended || this.fadeLeft >= 0) return
    const frames = Math.max(0, Math.round(finiteOr(fadeSec, 0) * this.sr))
    if (frames === 0) { this.ended = true; return }
    this.fadeLeft = frames
    this.fadeStep = this.fadeGain / frames
  }

  /** Reset the voice, the filters, the fade and the params to their defaults. */
  reset(): void {
    this.voice.reset()
    for (let l = 0; l < MAX_LISTENERS; l++) this.lowpass[l].reset()
    for (let i = 0; i < this.target.length; i++) this.target[i] = this.current[i] = this.def.defaults[i]
    this.fadeLeft = -1
    this.fadeGain = 1
    this.ended = false
  }

  /** OVERWRITE bus and send with n frames of the bed (silence once done). */
  process(bus: [Float32Array, Float32Array], send: [Float32Array, Float32Array], n: number): void {
    const [bl, br] = bus
    const [sl, sr] = send
    bl.fill(0, 0, n)
    br.fill(0, 0, n)
    sl.fill(0, 0, n)
    sr.fill(0, 0, n)
    for (let off = 0; off < n && !this.ended; off += BLOCK) {
      this.block(bl, br, off, Math.min(BLOCK, n - off))
    }
    const s = this.sp[SP.SEND]
    for (let i = 0; i < n; i++) {
      sl[i] = bl[i] * s
      sr[i] = br[i] * s
    }
  }

  private block(bl: Float32Array, br: Float32Array, off: number, len: number): void {
    if (this.dopplerIndex >= 0) {
      this.target[this.dopplerIndex] = this.rate
      if (!this.smooth[this.dopplerIndex]) this.current[this.dopplerIndex] = this.rate
    }
    const c = this.coef
    for (let i = 0; i < this.current.length; i++) {
      if (this.smooth[i]) this.current[i] = this.target[i] + (this.current[i] - this.target[i]) * c
    }
    const tl = this.tmpL
    const tr = this.tmpR
    tl.fill(0)
    tr.fill(0)
    this.voice.process(this.current, 0, tl, tr, len)
    for (let i = 0; i < len; i++) {
      if (!Number.isFinite(tl[i]) || !Number.isFinite(tr[i])) {
        this.voice.reset()
        for (let l = 0; l < MAX_LISTENERS; l++) this.lowpass[l].reset()
        return
      }
    }
    if (this.def.positional) this.spatialize(tl, tr, len)
    const g = this.gain
    for (let i = 0; i < len; i++) {
      let fg = 1
      if (this.fadeLeft >= 0) {
        fg = this.fadeGain
        this.fadeGain = Math.max(0, this.fadeGain - this.fadeStep)
        if (--this.fadeLeft <= 0) { this.ended = true; this.fadeGain = 0 }
      }
      bl[off + i] = tl[i] * g * fg
      br[off + i] = tr[i] * g * fg
      if (this.ended) break
    }
  }

  /** Mono (L+R)/2 through each listener's lowpass, equal-power pan and gain, summed in place. */
  private spatialize(tl: Float32Array, tr: Float32Array, len: number): void {
    const sp = this.sp
    const mono = this.tmpMono
    const count = Math.min(MAX_LISTENERS, Math.max(0, Math.floor(sp[SP.COUNT])))
    for (let i = 0; i < len; i++) mono[i] = (tl[i] + tr[i]) * 0.5
    tl.fill(0)
    tr.fill(0)
    for (let l = 0; l < count; l++) {
      const g = sp[SP.GAIN + l]
      if (!(g > MIN_LISTENER_GAIN)) continue
      const lp = this.lowpass[l]
      lp.setLowpass(sp[SP.LOWPASS + l], this.sr)
      const pan = Math.max(-1, Math.min(1, finiteOr(sp[SP.PAN + l], 0)))
      const a = ((pan + 1) * Math.PI) / 4
      const gl = Math.cos(a) * g
      const gr = Math.sin(a) * g
      for (let i = 0; i < len; i++) {
        const y = lp.process(mono[i])
        tl[i] += y * gl
        tr[i] += y * gr
      }
    }
  }

  /** FLAT spatial values for one listener: count 1, gain 1, pan 0, open lowpass, base send. */
  private flatSpatial(): void {
    this.sp[SP.COUNT] = 1
    this.sp[SP.SEND] = REVERB_SEND_BASE
    this.sp[SP.GAIN] = 1
    this.sp[SP.PAN] = 0
    this.sp[SP.LOWPASS] = OPEN_LOWPASS_HZ
    this.rate = 1
  }
}
