// Unit tests for the Royal Fiesta pack: the 9 family ids with their variants, lengths, buses and
// categories; the pack registered without displacing 'base'; the crowd bed registered and
// rendering 1 s of finite, non-silent stereo.
import { describe, expect, it } from 'vitest'
import {
  AudioRng, BLOCK, BUILTIN_FAMILIES, FAMILY_MAX_SECONDS, FAMILY_VARIANTS_MAX, FAMILY_VARIANTS_MIN,
  activeMix, defineMix, getBed, getMix, hashSeed, registerBuiltins, type FamilyDef,
} from '@etcito/haa'
import { CROWD_BED, FIESTA_PRESET, ROYAL_FIESTA_FAMILIES, registerRoyalFiesta } from '../src/index.ts'

const SR = 48000

/** id -> [maxSeconds, bus, category] */
const EXPECTED: Record<string, [number, string, string]> = {
  'fiesta.jump': [0.35, 'effects', 'fx'],
  'fiesta.dive': [0.5, 'effects', 'fx'],
  'fiesta.grab': [0.25, 'effects', 'fx'],
  'fiesta.eliminate': [1.2, 'effects', 'fx'],
  'fiesta.wobble': [0.6, 'effects', 'fx'],
  'fiesta.squash': [0.3, 'effects', 'fx'],
  'crowd.cheer.small': [1.5, 'ambience', 'sting'],
  'crowd.cheer.big': [3.0, 'ambience', 'sting'],
  'crowd.aww': [1.8, 'ambience', 'sting'],
}

describe('fiesta families', () => {
  for (const [id, [seconds, bus, category]] of Object.entries(EXPECTED)) {
    it(`${id} is in the pack with 8 variants, ${seconds} s, ${bus}/${category}`, () => {
      const def = ROYAL_FIESTA_FAMILIES.find((f) => f.id === id)
      expect(def).toBeDefined()
      expect(def!.variants).toBe(8)
      expect(def!.maxSeconds).toBe(seconds)
      expect(def!.bus).toBe(bus)
      expect(def!.category).toBe(category)
    })
  }

  it('is not built into the core', () => {
    const ids = new Set(BUILTIN_FAMILIES.map((f) => f.id))
    for (const id of Object.keys(EXPECTED)) expect(ids.has(id)).toBe(false)
  })
})

describe('fiesta registration', () => {
  it('keeps base active, registers the fiesta mix and the crowd bed, and is idempotent', () => {
    registerBuiltins()
    registerRoyalFiesta()
    registerRoyalFiesta()
    expect(activeMix().id).toBe('base')
    expect(getMix('fiesta')).toBeDefined()
    expect(() => defineMix(FIESTA_PRESET)).not.toThrow()
    expect(getBed('crowd')).toBeDefined()
  })
})

describe('crowd bed', () => {
  it('renders 1 s of finite, non-silent output in 128-frame blocks', () => {
    const voice = CROWD_BED.create(SR, 1)
    const p = Float32Array.from(CROWD_BED.defaults)
    const l = new Float32Array(BLOCK)
    const r = new Float32Array(BLOCK)
    let nonFinite = 0
    let sum = 0
    let count = 0
    for (let at = 0; at < SR; at += BLOCK) {
      const n = Math.min(BLOCK, SR - at)
      l.fill(0)
      r.fill(0)
      voice.process(p, 0, l, r, n)
      for (let i = 0; i < n; i++) {
        if (!Number.isFinite(l[i]) || !Number.isFinite(r[i])) nonFinite++
        else sum += l[i] * l[i] + r[i] * r[i]
      }
      count += 2 * n
    }
    expect(nonFinite).toBe(0)
    const rmsDb = 10 * Math.log10(sum / count)
    expect(rmsDb).toBeGreaterThan(-60)
  })
})

/** Renders variant v of def into a zeroed ceil(maxSeconds x SR) buffer. */
function renderVariant(def: FamilyDef, v: number): { out: Float32Array; n: number } {
  const out = new Float32Array(Math.ceil(def.maxSeconds * SR))
  const n = def.render(out, SR, new AudioRng(hashSeed(def.seed, def.id, v)), v)
  return { out, n }
}

describe('pack families (same checks the core runs on its built-ins)', () => {
  for (const def of ROYAL_FIESTA_FAMILIES) {
    describe(def.id, () => {
      it('has a variant count and length within the contract bounds', () => {
        expect(def.variants).toBeGreaterThanOrEqual(FAMILY_VARIANTS_MIN)
        expect(def.variants).toBeLessThanOrEqual(FAMILY_VARIANTS_MAX)
        expect(def.maxSeconds).toBeGreaterThan(0)
        expect(def.maxSeconds).toBeLessThanOrEqual(FAMILY_MAX_SECONDS)
      })

      it('renders variant 0 and variant 1 as different PCM', () => {
        const a = renderVariant(def, 0)
        const b = renderVariant(def, 1)
        expect(a.n).toBeGreaterThan(0)
        expect(a.n).toBeLessThanOrEqual(a.out.length)
        expect(b.n).toBeLessThanOrEqual(b.out.length)
        let differs = false
        for (let i = 0; i < a.out.length && !differs; i++) differs = a.out[i] !== b.out[i]
        expect(differs).toBe(true)
      })
    })
  }
})
