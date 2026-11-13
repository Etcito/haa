// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe oscillators. These are internal building blocks: a raw sine/saw/square is
// NEVER routed to an output. Per the 'No raw oscillators' rule, tonal content must pass
// through an additive bank, an excitation-driven resonator or FM/AM modulation before it
// reaches a bus. Runs unchanged in node, on the main thread and in AudioWorkletGlobalScope.
// No allocation after construction.

import { TAU, finiteOr } from './math.ts'

/** A sample-rate locked phase accumulator whose next() returns the phase in [0, 1). */
export class Phasor {
  private inc = 0
  private ph = 0

  /** Set the frequency and sample rate; the current phase is preserved. */
  set(hz: number, sr: number): void {
    const rate = sr > 1 ? sr : 48000
    this.inc = finiteOr(hz, 0) / rate
  }

  get phase(): number {
    return this.ph
  }

  set phase(v: number) {
    this.ph = v - Math.floor(v)
    if (!Number.isFinite(this.ph)) this.ph = 0
  }

  /** The phase before advancing, in [0, 1). */
  next(): number {
    const p = this.ph
    this.ph += this.inc
    if (this.ph >= 1 || this.ph < 0) this.ph -= Math.floor(this.ph)
    return p
  }

  reset(): void {
    this.ph = 0
  }
}

/** Band limit of the additive bank: partials at or above this fraction of sr are skipped. */
const PARTIAL_LIMIT = 0.45

/**
 * Additive partial bank with one phase accumulator per partial. render() ADDS its
 * band-limited sum into the output buffer; partials with f0 * ratio >= 0.45 sr are
 * skipped. This is an internal building block, never routed raw to an output.
 */
export class AdditiveBank {
  private readonly ratios: Float32Array
  private readonly amps: Float32Array
  private readonly phases: Float32Array
  private readonly initial: Float32Array
  private readonly maxPartials: number
  private n = 0

  constructor(maxPartials: number) {
    this.maxPartials = Math.max(1, Math.floor(maxPartials))
    this.ratios = new Float32Array(this.maxPartials)
    this.amps = new Float32Array(this.maxPartials)
    this.phases = new Float32Array(this.maxPartials)
    this.initial = new Float32Array(this.maxPartials)
  }

  /** Number of active partials (highest configured index + 1). */
  get count(): number {
    return this.n
  }

  /** Configure partial i; the phase defaults to 0 and is also the reset phase. */
  setPartial(i: number, ratio: number, amp: number, phase?: number): void {
    if (!Number.isInteger(i) || i < 0 || i >= this.maxPartials) return
    this.ratios[i] = finiteOr(ratio, 0)
    this.amps[i] = finiteOr(amp, 0)
    const raw = phase === undefined ? 0 : finiteOr(phase, 0)
    const p0 = raw - Math.floor(raw)
    this.initial[i] = p0
    this.phases[i] = p0
    if (i + 1 > this.n) this.n = i + 1
  }

  /**
   * Add gain * sum(amp_i * sin(phase_i)) into out[0..n) for every partial below the
   * Nyquist guard. Phase accumulators advance by f0 * ratio / sr each sample.
   */
  render(f0: number, out: Float32Array, n: number, sr: number, gain: number): void {
    const rate = sr > 1 ? sr : 48000
    const limit = PARTIAL_LIMIT * rate
    const base = finiteOr(f0, 0)
    const g = finiteOr(gain, 0)
    const frames = n < out.length ? n : out.length
    for (let i = 0; i < this.n; i++) {
      const amp = this.amps[i] * g
      if (amp === 0) continue
      const fr = base * this.ratios[i]
      if (!(fr > 0) || fr >= limit) continue
      const inc = fr / rate
      let ph = this.phases[i]
      for (let s = 0; s < frames; s++) {
        out[s] += amp * Math.sin(TAU * ph)
        ph += inc
        if (ph >= 1) ph -= Math.floor(ph)
      }
      this.phases[i] = ph
    }
  }

  /** Reset every phase accumulator to its configured phase. */
  reset(): void {
    for (let i = 0; i < this.n; i++) this.phases[i] = this.initial[i]
  }
}
