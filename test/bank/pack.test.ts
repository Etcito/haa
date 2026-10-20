import { describe, expect, it } from 'vitest'
import { GAP_SAMPLES } from '../../src/contract.ts'
import type { BankFamilyEntry } from '../../src/contract.ts'
import { alignVariants, packStrip, pcmHash } from '../../src/bank/pack.ts'

function burst(len: number, lead: number): Float32Array {
  const v = new Float32Array(len)
  for (let i = lead; i < len; i++) v[i] = 0.8 * Math.exp(-(i - lead) / 40) * Math.cos((i - lead) * 0.3)
  return v
}

describe('packStrip', () => {
  it('lays variants out with gaps', () => {
    const a = new Float32Array(100)
    const b = new Float32Array(50)
    a[3] = 0.5
    b[0] = 0.5
    const { strip, entries, onset0 } = packStrip([a, b])
    expect(strip.length).toBe(150 + 2 * GAP_SAMPLES)
    expect(entries).toEqual([{ start: 0, length: 100 }, { start: 100 + GAP_SAMPLES, length: 50 }])
    expect(onset0).toBe(3)
    expect(strip.subarray(100, 100 + GAP_SAMPLES).every((x) => x === 0)).toBe(true)
    expect(strip.subarray(150 + GAP_SAMPLES).every((x) => x === 0)).toBe(true)
  })
  it('silent strip has onset0 0', () => {
    expect(packStrip([new Float32Array(10)]).onset0).toBe(0)
  })
})

describe('pcmHash', () => {
  it('is stable and sensitive', () => {
    const s = Float32Array.from([0, 0.25, -0.5, 1])
    expect(pcmHash(s)).toMatch(/^[0-9a-f]{8}$/)
    expect(pcmHash(s)).toBe('3a8b59eb')
    const t = s.slice()
    t[2] = -0.5000001
    expect(pcmHash(t)).not.toBe(pcmHash(s))
  })
})

describe('alignVariants', () => {
  const variants = [burst(3000, 5), burst(2000, 40), burst(2500, 17)]
  const { strip, entries, onset0 } = packStrip(variants)
  const entry: BankFamilyEntry = {
    id: 'x', file: 'x.mp3', bus: 'world', category: 'fx', variants: entries, onset0, pcmHash: pcmHash(strip), mp3Sha256: '',
  }
  const rate = 44100
  const k = rate / 48000
  const delay = 7
  const n = Math.floor(strip.length * k)
  const dec = new Float32Array(n + delay)
  for (let i = 0; i < n; i++) {
    const p = i / k
    const j = Math.floor(p)
    const f = p - j
    dec[i + delay] = strip[j] * (1 - f) + (strip[j + 1] ?? 0) * f
  }
  it('recovers onsets within 1 sample', () => {
    const out = alignVariants(dec, entry, rate)
    expect(out.length).toBe(3)
    const leads = [5, 40, 17]
    out.forEach((v, i) => {
      let first = -1
      for (let s = 0; s < v.length; s++) if (Math.abs(v[s]) > 1e-3) { first = s; break }
      const expected = (entries[i].start + leads[i]) * k - Math.round(entries[i].start * k)
      expect(Math.abs(first - expected)).toBeLessThanOrEqual(1)
    })
  })
  it('zero-length when beyond decoded', () => {
    const e = { ...entry, variants: [...entries, { start: 10_000_000, length: 100 }] }
    const out = alignVariants(dec, e, rate)
    expect(out[3].length).toBe(0)
  })
})
