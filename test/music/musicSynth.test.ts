// MusicSynth and MUSIC_BED unit tests (module music, owner M3): the bed definition, the registered
// style order, a short safe render, seeded determinism (Q3), the clamped fadeBars crossfade and the
// empty-style-list error. Renders stay at 1 s so the file runs well under 5 s.
import { beforeAll, describe, expect, it } from 'vitest'
import { MUSIC_BED } from '../../src/content/musicBed.ts'
import { MUSIC_PARAMS, MusicSynth } from '../../src/music/musicSynth.ts'
import { defineBed, listStyles, styleIndex } from '../../src/registry/registry.ts'
import { registerBuiltins } from '../../src/registry/builtins.ts'
import { renderBedDemo } from '../../src/offline/render.ts'
import type { StereoRender } from '../../src/contract.ts'

const P = (name: (typeof MUSIC_PARAMS)[number]): number => MUSIC_PARAMS.indexOf(name)

function render(seed: number, script: (t: number, p: Float32Array) => void): StereoRender {
  return renderBedDemo(MUSIC_BED, { seconds: 1, seed, script })
}

function upbeat(t: number, p: Float32Array): void {
  void t
  p[P('intensity')] = 0.6
  p[P('phase')] = 1
  p[P('style')] = styleIndex('upbeat')
}

function allFinite(r: StereoRender): boolean {
  for (const ch of [r.left, r.right]) for (const x of ch) if (!Number.isFinite(x)) return false
  return true
}

function rmsDb(r: StereoRender): number {
  let s = 0
  for (let i = 0; i < r.left.length; i++) s += r.left[i] * r.left[i] + r.right[i] * r.right[i]
  return 10 * Math.log10(s / (2 * r.left.length) + 1e-30)
}

function same(a: StereoRender, b: StereoRender): boolean {
  if (a.left.length !== b.left.length) return false
  for (let i = 0; i < a.left.length; i++) {
    if (!Object.is(a.left[i], b.left[i]) || !Object.is(a.right[i], b.right[i])) return false
  }
  return true
}

describe('MusicSynth', () => {
  beforeAll(() => registerBuiltins())

  it('MUSIC_BED passes defineBed with one default per param', () => {
    expect(() => defineBed(MUSIC_BED)).not.toThrow()
    expect(MUSIC_BED.defaults.length).toBe(MUSIC_PARAMS.length)
  })

  it('registers the styles in id order (calm 0, upbeat 1)', () => {
    expect(listStyles().map((s) => s.id)).toEqual(['calm', 'upbeat'])
    expect(styleIndex('calm')).toBe(0)
    expect(styleIndex('upbeat')).toBe(1)
  })

  it('renders 1 s of upbeat music finite and not silent', () => {
    const r = render(1, upbeat)
    expect(allFinite(r)).toBe(true)
    expect(rmsDb(r)).toBeGreaterThan(-60)
  })

  it('is bit-identical for the same seed and differs for another seed', () => {
    const a = render(7, upbeat)
    expect(same(a, render(7, upbeat))).toBe(true)
    expect(same(a, render(8, upbeat))).toBe(false)
  })

  for (const fadeBars of [0, 40]) {
    it(`renders a style change with fadeBars ${fadeBars} finite (clamped to 1..16)`, () => {
      const r = render(3, (t, p) => {
        upbeat(t, p)
        p[P('fadeBars')] = fadeBars
        if (t >= 0.25) p[P('style')] = styleIndex('calm')
      })
      expect(allFinite(r)).toBe(true)
      expect(rmsDb(r)).toBeGreaterThan(-60)
    })
  }

  it('throws on an empty style list', () => {
    expect(() => new MusicSynth(48000, 1, [])).toThrow('[haa] no music style')
  })
})
