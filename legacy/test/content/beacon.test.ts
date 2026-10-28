// Unit tests for the beacon positional bed (module spatial-demo): a 2 s render is finite,
// non-silent, within the peak ceiling and continuous, and doppler 2 raises the spectral centroid.
import { describe, expect, it } from 'vitest'
import { BLOCK } from '../../src/contract.ts'
import { BEACON_BED } from '../../src/content/beacon.ts'
import { defineBed } from '../../src/registry/registry.ts'

const SR = 48000

/** Renders `seconds` of the beacon (seed 1) in BLOCK chunks with the given params. */
function render(values: readonly number[], seconds: number): { l: Float32Array; r: Float32Array } {
  const voice = BEACON_BED.create(SR, 1)
  const p = Float32Array.from(values)
  const n = Math.round(seconds * SR)
  const l = new Float32Array(n)
  const r = new Float32Array(n)
  for (let o = 0; o < n; o += BLOCK) {
    const len = Math.min(BLOCK, n - o)
    voice.process(p, 0, l.subarray(o, o + len), r.subarray(o, o + len), len)
  }
  return { l, r }
}

function rmsDb(x: Float32Array, from: number, to: number): number {
  let s = 0
  for (let i = from; i < to; i++) s += x[i] * x[i]
  return 10 * Math.log10(s / Math.max(1, to - from))
}

/** Magnitude-weighted mean frequency of a Hann-windowed plain DFT over x[start..start+len). */
function centroidHz(x: Float32Array, start: number, len: number): number {
  const v = new Float64Array(len)
  for (let i = 0; i < len; i++) v[i] = x[start + i] * (0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (len - 1)))
  const cos = new Float64Array(len)
  const sin = new Float64Array(len)
  for (let i = 0; i < len; i++) {
    cos[i] = Math.cos((2 * Math.PI * i) / len)
    sin[i] = Math.sin((2 * Math.PI * i) / len)
  }
  let num = 0
  let den = 0
  for (let k = 1; k < len / 2; k++) {
    let re = 0
    let im = 0
    for (let i = 0, j = 0; i < len; i++, j = (j + k) % len) {
      re += v[i] * cos[j]
      im -= v[i] * sin[j]
    }
    const mag = Math.hypot(re, im)
    num += mag * ((k * SR) / len)
    den += mag
  }
  return den > 0 ? num / den : 0
}

describe('BEACON_BED', () => {
  it('defines through defineBed', () => {
    expect(() => defineBed(BEACON_BED)).not.toThrow()
  })

  it('renders 2 s finite, non-silent, under -1 dBFS and continuous', () => {
    const { l, r } = render(BEACON_BED.defaults, 2)
    let peak = 0
    let bad = 0
    for (let i = 0; i < l.length; i++) {
      if (!Number.isFinite(l[i]) || r[i] !== l[i]) bad++
      peak = Math.max(peak, Math.abs(l[i]))
    }
    expect(bad).toBe(0)
    expect(rmsDb(l, 0, l.length)).toBeGreaterThanOrEqual(-60)
    expect(20 * Math.log10(peak)).toBeLessThanOrEqual(-1)
    const win = SR / 10
    let minWin = Infinity
    for (let o = 0; o + win <= l.length; o += win) minWin = Math.min(minWin, rmsDb(l, o, o + win))
    expect(minWin).toBeGreaterThanOrEqual(-60)
  })

  it('raises the spectral centroid at doppler 2', () => {
    const start = SR / 2
    const len = 4096
    const base = centroidHz(render([1, 4, 1], 1).l, start, len)
    const fast = centroidHz(render([1, 4, 2], 1).l, start, len)
    expect(fast).toBeGreaterThan(base)
  })
})
