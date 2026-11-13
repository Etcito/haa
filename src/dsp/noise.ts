// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports, explicit fields instead of parameter properties
// DSP-safe noise generators. Every generator draws only from its own AudioRng, so two
// instances with different seeds never share a stream. Sample-rate agnostic (sr passed in).
// No allocation after construction. Runs unchanged in node, on the main thread and in
// AudioWorkletGlobalScope.

import { clamp } from './math.ts'
import type { AudioRng } from './rng.ts'

/** Uniform white noise in [-1, 1). */
export class WhiteNoise {
  private readonly rng: AudioRng

  constructor(rng: AudioRng) {
    this.rng = rng
  }

  next(): number {
    return this.rng.next() * 2 - 1
  }

  reset(): void {
    // Stateless beyond the rng; nothing to reset.
  }
}

/** Fixed output scale that brings the Kellet refined pink filter close to unit peak. */
const PINK_SCALE = 0.11

/**
 * Pink noise, Paul Kellet's refined (economy) filter. The output is scaled to about
 * unit peak and clamped for safety.
 */
export class PinkNoise {
  private b0 = 0
  private b1 = 0
  private b2 = 0
  private b3 = 0
  private b4 = 0
  private b5 = 0
  private b6 = 0

  private readonly rng: AudioRng

  constructor(rng: AudioRng) {
    this.rng = rng
  }

  next(): number {
    const w = this.rng.next() * 2 - 1
    this.b0 = 0.99886 * this.b0 + w * 0.0555179
    this.b1 = 0.99332 * this.b1 + w * 0.0750759
    this.b2 = 0.969 * this.b2 + w * 0.153852
    this.b3 = 0.8665 * this.b3 + w * 0.3104856
    this.b4 = 0.55 * this.b4 + w * 0.5329522
    this.b5 = -0.7616 * this.b5 - w * 0.016898
    const pink = this.b0 + this.b1 + this.b2 + this.b3 + this.b4 + this.b5 + this.b6 + w * 0.5362
    this.b6 = w * 0.115926
    return clamp(pink * PINK_SCALE, -1, 1)
  }

  reset(): void {
    this.b0 = 0
    this.b1 = 0
    this.b2 = 0
    this.b3 = 0
    this.b4 = 0
    this.b5 = 0
    this.b6 = 0
  }
}

/** Leak of the brown-noise integrator. */
const BROWN_LEAK = 0.02
/** Output scale that normalizes the leaky integrator close to unit peak. */
const BROWN_SCALE = 3.5

/** Brown noise: a leaky integrator (leak 0.02), normalized and clamped. */
export class BrownNoise {
  private last = 0

  private readonly rng: AudioRng

  constructor(rng: AudioRng) {
    this.rng = rng
  }

  next(): number {
    const w = this.rng.next() * 2 - 1
    this.last = (this.last + BROWN_LEAK * w) / (1 + BROWN_LEAK)
    return clamp(this.last * BROWN_SCALE, -1, 1)
  }

  reset(): void {
    this.last = 0
  }
}

/**
 * Sparse Poisson crackle: each sample is an impulse with probability rate/sr, with a
 * random amplitude and a random sign; otherwise 0.
 */
export class Crackle {
  private readonly rng: AudioRng

  constructor(rng: AudioRng) {
    this.rng = rng
  }

  next(ratePerSec: number, sr: number): number {
    if (!(ratePerSec > 0) || !(sr > 0)) return 0
    const p = ratePerSec / sr
    if (this.rng.next() >= (p > 1 ? 1 : p)) return 0
    const amp = this.rng.next()
    return this.rng.next() < 0.5 ? -amp : amp
  }

  reset(): void {
    // Stateless beyond the rng; nothing to reset.
  }
}

/**
 * Velvet noise: exactly one +/-1 impulse per period of sr/density samples, placed at a
 * random position inside that period.
 */
export class VelvetNoise {
  private remaining = 0
  private at = 0
  private sign = 1

  private readonly rng: AudioRng

  constructor(rng: AudioRng) {
    this.rng = rng
  }

  next(density: number, sr: number): number {
    if (!(density > 0) || !(sr > 0)) return 0
    if (this.remaining <= 0) {
      const period = Math.max(1, Math.round(sr / density))
      this.remaining = period
      this.at = Math.floor(this.rng.next() * period)
      this.sign = this.rng.next() < 0.5 ? -1 : 1
    }
    const emit = this.at <= 0
    this.at -= 1
    this.remaining -= 1
    return emit ? this.sign : 0
  }

  reset(): void {
    this.remaining = 0
    this.at = 0
    this.sign = 1
  }
}
