// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe modal synthesis: a bank of 2-pole resonators driven by an excitation signal.
// Sample-rate agnostic (sr passed in). No allocation after construction; reset() clears
// the resonator states. Runs unchanged in node, on the main thread and in AudioWorkletGlobalScope.

import { TAU, finiteOr } from './math.ts'

export interface Mode {
  hz: number
  t60: number
  gain: number
}

/**
 * Sum of 2-pole resonators y[n] = 2 r cos(w) y[n-1] - r^2 y[n-2] + gain sin(w) x[n],
 * so a unit impulse rings as gain r^n sin((n + 1) w). Pole radius r = 10^(-3 / (t60 sr)).
 * Modes at or above 0.45 sr (or with a non-positive frequency) are skipped.
 */
export class ModalBank {
  readonly maxModes: number
  private readonly sr: number
  private readonly b: Float64Array
  private readonly a1: Float64Array
  private readonly a2: Float64Array
  private readonly y1: Float64Array
  private readonly y2: Float64Array
  private count = 0

  constructor(maxModes: number, sr: number) {
    this.maxModes = Math.max(0, Math.floor(finiteOr(maxModes, 0)))
    this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000
    const n = Math.max(1, this.maxModes)
    this.b = new Float64Array(n)
    this.a1 = new Float64Array(n)
    this.a2 = new Float64Array(n)
    this.y1 = new Float64Array(n)
    this.y2 = new Float64Array(n)
  }

  /**
   * Loads up to maxModes modes, scaling every frequency by freqScale and every t60 by
   * decayScale. Resonator states are kept so modes can be retuned while ringing; a mode
   * that becomes skipped is silenced.
   */
  setModes(modes: readonly Mode[], freqScale = 1, decayScale = 1): void {
    const sr = this.sr
    const nyq = 0.45 * sr
    const fs = finiteOr(freqScale, 1)
    const ds = finiteOr(decayScale, 1)
    const n = modes.length < this.maxModes ? modes.length : this.maxModes
    for (let i = 0; i < n; i++) {
      const m = modes[i]
      const hz = m.hz * fs
      const t60 = m.t60 * ds
      if (!(hz > 0) || !(hz < nyq) || !(t60 > 0) || !Number.isFinite(t60)) {
        this.silence(i)
        continue
      }
      const w = (TAU * hz) / sr
      const r = Math.pow(10, -3 / (t60 * sr))
      this.a1[i] = 2 * r * Math.cos(w)
      this.a2[i] = -r * r
      this.b[i] = finiteOr(m.gain, 0) * Math.sin(w)
    }
    for (let i = n; i < this.count; i++) this.silence(i)
    this.count = n
  }

  private silence(i: number): void {
    this.b[i] = 0
    this.a1[i] = 0
    this.a2[i] = 0
    this.y1[i] = 0
    this.y2[i] = 0
  }

  process(excitation: number): number {
    const b = this.b
    const a1 = this.a1
    const a2 = this.a2
    const y1 = this.y1
    const y2 = this.y2
    let sum = 0
    for (let i = 0; i < this.count; i++) {
      const y = a1[i] * y1[i] + a2[i] * y2[i] + b[i] * excitation
      y2[i] = y1[i]
      y1[i] = y
      sum += y
    }
    return sum
  }

  /** Sum of the squared resonator states (both delay taps of every mode). */
  energy(): number {
    let e = 0
    for (let i = 0; i < this.count; i++) e += this.y1[i] * this.y1[i] + this.y2[i] * this.y2[i]
    return e
  }

  reset(): void {
    this.y1.fill(0)
    this.y2.fill(0)
  }
}
