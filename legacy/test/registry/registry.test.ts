// Unit tests for the explicit registries (module registry): every define* validation error,
// seed default and freezing, duplicate rejection, id ordering, activeMix priority, trim helpers,
// bankSeconds and idempotent builtins. Synthetic specs only; no render runs here.
import { beforeEach, describe, expect, it } from 'vitest'
import type { BedDef, BusId, FamilySpec, MixPreset, MusicStyle, VoiceCategory } from '../../src/contract.ts'
import { hashSeed } from '../../src/dsp/rng.ts'
import { BUILTIN_BEDS, BUILTIN_FAMILIES, BUILTIN_MIXES, BUILTIN_STYLES } from '../../src/content/index.ts'
import { BASE_PRESET } from '../../src/mix/presets/base.ts'
import {
  activeMix, bankSeconds, bedTrimDb, clearRegistry, defineBed, defineFamily, defineMix, defineStyle,
  familyTrimDb, getBed, getFamily, getMix, getStyle, listBeds, listFamilies, listMixes, listStyles,
  registerBed, registerFamilies, registerFamily, registerMix, registerStyle, styleIndex,
} from '../../src/registry/registry.ts'
import { registerBuiltins } from '../../src/registry/builtins.ts'

/** A tiny family render: adds a few samples and returns the frames written. */
function render(out: Float32Array): number {
  const n = Math.min(4, out.length)
  for (let i = 0; i < n; i++) out[i] += 0.1 * (i + 1)
  return n
}

/** A bed voice that does nothing. */
const voice = { process(): void {}, reset(): void {} }

function family(o: Partial<FamilySpec> = {}): FamilySpec {
  return { id: 'test.one', bus: 'effects', category: 'fx', variants: 8, maxSeconds: 0.5, render, ...o }
}

function bed(o: Partial<BedDef> = {}): BedDef {
  return { id: 'test-bed', bus: 'ambience', params: ['level'], defaults: [1], positional: false, create: () => voice, ...o }
}

function mix(o: Partial<MixPreset> = {}): MixPreset {
  return { ...BASE_PRESET, id: 'test-mix', priority: 1, ...o }
}

function style(o: Partial<MusicStyle> = {}): MusicStyle {
  return {
    id: 'test-style', root: 60, scale: [0, 2, 4, 7, 9], bpm: 120,
    palette: { pad: 1, bass: 1, lead: 1, pluck: 1, drums: 1, perc: 1 },
    swing: 0, progressions: [[0, 2, 4, 0]], ...o,
  }
}

/** Copy of a record without one key, typed loosely to build invalid presets. */
function without(record: object, key: string): Record<string, unknown> {
  const copy = { ...record } as Record<string, unknown>
  delete copy[key]
  return copy
}

beforeEach(() => clearRegistry())

describe('defineFamily validation', () => {
  const cases: [string, Partial<FamilySpec>, string][] = [
    ['bad id', { id: 'Test.One' }, '[haa] bad family Test.One'],
    ['variants 7', { variants: 7 }, '[haa] bad family test.one'],
    ['variants 33', { variants: 33 }, '[haa] bad family test.one'],
    ['variants 8.5', { variants: 8.5 }, '[haa] bad family test.one'],
    ['unknown bus', { bus: 'nope' as BusId }, '[haa] bad family test.one'],
    ['unknown category', { category: 'nope' as VoiceCategory }, '[haa] bad family test.one'],
    ['maxSeconds 0', { maxSeconds: 0 }, '[haa] bad family test.one'],
    ['maxSeconds 11', { maxSeconds: 11 }, '[haa] bad family test.one'],
    ['maxSeconds NaN', { maxSeconds: NaN }, '[haa] bad family test.one'],
    ['render not a function', { render: 1 as unknown as FamilySpec['render'] }, '[haa] bad family test.one'],
  ]
  it.each(cases)('rejects %s', (_name, o, prefix) => {
    expect(() => defineFamily(family(o))).toThrow(`${prefix}: `)
  })
})

describe('defineBed validation', () => {
  const cases: [string, Partial<BedDef>, string][] = [
    ['bad id', { id: 'Test_Bed' }, '[haa] bad bed Test_Bed'],
    ['empty params', { params: [], defaults: [] }, '[haa] bad bed test-bed'],
    ['duplicate params', { params: ['a', 'a'], defaults: [0, 0] }, '[haa] bad bed test-bed'],
    ['defaults length mismatch', { defaults: [1, 2] }, '[haa] bad bed test-bed'],
    ['NaN default', { defaults: [NaN] }, '[haa] bad bed test-bed'],
    ['unsmoothed not in params', { unsmoothed: ['other'] }, '[haa] bad bed test-bed'],
    ['create not a function', { create: null as unknown as BedDef['create'] }, '[haa] bad bed test-bed'],
  ]
  it.each(cases)('rejects %s', (_name, o, prefix) => {
    expect(() => defineBed(bed(o))).toThrow(`${prefix}: `)
  })
})

describe('defineMix validation', () => {
  const prefix = '[haa] bad mix test-mix: '
  it('rejects a missing bus', () => {
    const p = { ...mix(), buses: without(BASE_PRESET.buses, 'music') } as unknown as MixPreset
    expect(() => defineMix(p)).toThrow(prefix)
  })
  it('rejects ceilingDbtp -1', () => {
    const master = { ...BASE_PRESET.master, limiter: { ...BASE_PRESET.master.limiter, ceilingDbtp: -1 } }
    expect(() => defineMix(mix({ master }))).toThrow(prefix)
  })
  it('rejects a missing loudness category', () => {
    const p = { ...mix(), loudness: without(BASE_PRESET.loudness, 'sting') } as unknown as MixPreset
    expect(() => defineMix(p)).toThrow(prefix)
  })
  it('rejects a non-finite number', () => {
    expect(() => defineMix(mix({ sceneLufs: NaN }))).toThrow(prefix)
  })
})

describe('defineStyle validation', () => {
  const cases: [string, Partial<MusicStyle>][] = [
    ['scale not starting at 0', { scale: [2, 4, 7] }],
    ['scale not ascending', { scale: [0, 4, 2] }],
    ['scale out of 0..11', { scale: [0, 4, 12] }],
    ['bpm 39', { bpm: 39 }],
    ['bpm 241', { bpm: 241 }],
    ['swing 0.31', { swing: 0.31 }],
    ['a progression of length 5', { progressions: [[0, 1, 2, 3, 0]] }],
    ['an index >= scale.length', { progressions: [[0, 1, 2, 5]] }],
    ['no progression starting on 0', { progressions: [[1, 2, 3, 4], [2, 0, 1, 0]] }],
  ]
  it.each(cases)('rejects %s', (_name, o) => {
    expect(() => defineStyle(style(o))).toThrow('[haa] bad style test-style: ')
  })
})

describe('definitions', () => {
  it('defaults the family seed to hashSeed(family, id) and freezes a new object', () => {
    const spec = family()
    const def = defineFamily(spec)
    expect(def.seed).toBe(hashSeed('family', 'test.one'))
    expect(Object.isFrozen(def)).toBe(true)
    expect(def).not.toBe(spec)
    expect('seed' in spec).toBe(false)
    expect(defineFamily(family({ seed: 42 })).seed).toBe(42)
    const out = new Float32Array(8)
    expect(def.render(out, 48000, undefined as never, 0)).toBe(4)
  })

  it('freezes beds, mixes and styles', () => {
    expect(Object.isFrozen(defineBed(bed()))).toBe(true)
    expect(Object.isFrozen(defineMix(mix()))).toBe(true)
    expect(Object.isFrozen(defineStyle(style()))).toBe(true)
  })
})

describe('duplicates', () => {
  it('throws on a duplicate family', () => {
    registerFamily(family())
    expect(() => registerFamily(family())).toThrow('[haa] duplicate family test.one')
  })
  it('throws on a duplicate bed', () => {
    registerBed(bed())
    expect(() => registerBed(bed())).toThrow('[haa] duplicate bed test-bed')
  })
  it('throws on a duplicate mix', () => {
    registerMix(mix())
    expect(() => registerMix(mix())).toThrow('[haa] duplicate mix test-mix')
  })
  it('throws on a duplicate style', () => {
    registerStyle(style())
    expect(() => registerStyle(style())).toThrow('[haa] duplicate style test-style')
  })
})

describe('ordering and lookups', () => {
  it('lists families by id whatever the registration order', () => {
    registerFamilies([family({ id: 'c.three' }), family({ id: 'a.one' }), family({ id: 'b.two' })])
    expect(listFamilies().map((f) => f.id)).toEqual(['a.one', 'b.two', 'c.three'])
    expect(getFamily('b.two')?.id).toBe('b.two')
    expect(getFamily('missing')).toBeUndefined()
  })

  it('lists beds, mixes and styles by id', () => {
    registerBed(bed({ id: 'bed-b' }))
    registerBed(bed({ id: 'bed-a' }))
    expect(listBeds().map((b) => b.id)).toEqual(['bed-a', 'bed-b'])
    expect(getBed('bed-a')?.id).toBe('bed-a')
    registerMix(mix({ id: 'mix-b' }))
    registerMix(mix({ id: 'mix-a' }))
    expect(listMixes().map((m) => m.id)).toEqual(['mix-a', 'mix-b'])
    expect(getMix('mix-b')?.id).toBe('mix-b')
    registerStyle(style({ id: 'style-c' }))
    registerStyle(style({ id: 'style-a' }))
    registerStyle(style({ id: 'style-b' }))
    expect(listStyles().map((s) => s.id)).toEqual(['style-a', 'style-b', 'style-c'])
    expect(getStyle('style-c')?.id).toBe('style-c')
  })

  it('styleIndex returns the list index, or -1', () => {
    registerStyle(style({ id: 'style-c' }))
    registerStyle(style({ id: 'style-a' }))
    expect(styleIndex('style-a')).toBe(0)
    expect(styleIndex('style-c')).toBe(1)
    expect(styleIndex('missing')).toBe(-1)
  })
})

describe('activeMix', () => {
  it('throws when no preset is registered', () => {
    expect(() => activeMix()).toThrow('[haa] no mix preset')
  })
  it('picks the highest priority', () => {
    registerMix(mix({ id: 'alpha', priority: 0 }))
    registerMix(mix({ id: 'zeta', priority: 3 }))
    expect(activeMix().id).toBe('zeta')
  })
  it('breaks ties by the lowest id', () => {
    registerMix(mix({ id: 'zeta', priority: 2 }))
    registerMix(mix({ id: 'beta', priority: 2 }))
    registerMix(mix({ id: 'alpha', priority: 1 }))
    expect(activeMix().id).toBe('beta')
  })
})

describe('helpers', () => {
  it('familyTrimDb and bedTrimDb return the trim, or 0 when absent', () => {
    const p = mix({ familyTrimDb: { 'test.one': -3 }, bedTrimDb: { wind: -2 } })
    expect(familyTrimDb(p, 'test.one')).toBe(-3)
    expect(familyTrimDb(p, 'missing')).toBe(0)
    expect(bedTrimDb(p, 'wind')).toBe(-2)
    expect(bedTrimDb(p, 'missing')).toBe(0)
  })

  it('bankSeconds sums variants x maxSeconds', () => {
    const a = defineFamily(family({ id: 'a.one', variants: 8, maxSeconds: 0.5 }))
    const b = defineFamily(family({ id: 'b.two', variants: 10, maxSeconds: 1.25 }))
    expect(bankSeconds([a, b])).toBe(16.5)
    expect(bankSeconds([])).toBe(0)
    registerFamilies([a, b])
    expect(bankSeconds()).toBe(16.5)
  })
})

describe('registerBuiltins', () => {
  function counts(): number[] {
    return [listFamilies().length, listBeds().length, listMixes().length, listStyles().length]
  }
  const expected = [BUILTIN_FAMILIES.length, BUILTIN_BEDS.length, BUILTIN_MIXES.length, BUILTIN_STYLES.length]

  it('registers every built-in of each kind', () => {
    registerBuiltins()
    expect(counts()).toEqual(expected)
    expect(activeMix().id).toBe('base')
  })

  it('is idempotent', () => {
    registerBuiltins()
    expect(() => registerBuiltins()).not.toThrow()
    expect(counts()).toEqual(expected)
  })

  it('registers everything again after clearRegistry', () => {
    registerBuiltins()
    clearRegistry()
    expect(counts()).toEqual([0, 0, 0, 0])
    registerBuiltins()
    expect(counts()).toEqual(expected)
  })
})
