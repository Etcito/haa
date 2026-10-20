// Unit tests for the built-in families (Q5): unique ids, variant count, length cap, and
// variant 0 differing from variant 1. Iterates BUILTIN_FAMILIES, so later families are covered.
import { describe, expect, it } from 'vitest'
import { FAMILY_MAX_SECONDS, FAMILY_VARIANTS_MAX, FAMILY_VARIANTS_MIN } from '../../src/contract.ts'
import type { FamilyDef } from '../../src/contract.ts'
import { AudioRng, hashSeed } from '../../src/dsp/rng.ts'
import { BUILTIN_FAMILIES } from '../../src/content/index.ts'

const SR = 48000

/** Renders variant v of def into a zeroed ceil(maxSeconds x SR) buffer. */
function renderVariant(def: FamilyDef, v: number): { out: Float32Array; n: number } {
  const out = new Float32Array(Math.ceil(def.maxSeconds * SR))
  const n = def.render(out, SR, new AudioRng(hashSeed(def.seed, def.id, v)), v)
  return { out, n }
}

describe('built-in families', () => {
  it('has families with unique ids', () => {
    expect(BUILTIN_FAMILIES.length).toBeGreaterThan(0)
    const ids = BUILTIN_FAMILIES.map((f) => f.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  for (const def of BUILTIN_FAMILIES) {
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
