// Unit tests for frozen HAA contract values and spatial parameter layout.
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import {
  AUDIO_RATE,
  BLOCK,
  BUSES,
  BUS_INDEX,
  DEFAULT_USER_VOLUMES,
  FAMILY_MAX_SECONDS,
  FAMILY_VARIANTS_MAX,
  FAMILY_VARIANTS_MIN,
  GAP_SAMPLES,
  LOUDNESS_TOLERANCE_LU,
  MAX_LISTENERS,
  MICRO_VARIATION,
  MIX_INPUTS,
  NORMALIZE_PEAK_CAP_DB,
  ONSET_THRESHOLD,
  PROCESSOR_NAMES,
  REVERB_INPUT,
  REVERB_SEND_BASE,
  SAFETY,
  SAMPLER_VOICES,
  SP,
  SP_LENGTH,
  VOICE_CATEGORIES,
  VOICE_LIMITS,
  newSpatialResult,
  spGain,
  spLowpass,
  spPan,
} from '../src/contract.ts'

describe('contract constants', () => {
  it('pins the shared constant table', () => {
    expect(AUDIO_RATE).toBe(48000)
    expect(BLOCK).toBe(128)
    expect(MAX_LISTENERS).toBe(4)
    expect(BUSES).toEqual(['world', 'foley', 'effects', 'ambience', 'music', 'ui'])
    expect(BUS_INDEX).toEqual({ world: 0, foley: 1, effects: 2, ambience: 3, music: 4, ui: 5 })
    expect(REVERB_INPUT).toBe(6)
    expect(MIX_INPUTS).toBe(7)
    expect(VOICE_CATEGORIES).toEqual(['impact', 'fx', 'ui', 'ambience', 'sting'])
    expect(VOICE_LIMITS).toEqual({ impact: 12, fx: 16, ui: 4, ambience: 8, sting: 2 })
    expect(DEFAULT_USER_VOLUMES).toEqual({ master: 0.8, music: 0.6, sfx: 0.8 })
    expect(FAMILY_VARIANTS_MIN).toBe(8)
    expect(FAMILY_VARIANTS_MAX).toBe(32)
    expect(FAMILY_MAX_SECONDS).toBe(10)
    expect(GAP_SAMPLES).toBe(4800)
    expect(ONSET_THRESHOLD).toBe(1e-3)
    expect(PROCESSOR_NAMES).toEqual({ bed: 'haa-bed', sampler: 'haa-sampler', mix: 'haa-mix' })
    expect(SAMPLER_VOICES).toBe(64)
    expect(SAFETY).toEqual({ maxSamplePeakDb: -1, maxAbsDc: 1e-3, minRmsDb: -60 })
    expect(NORMALIZE_PEAK_CAP_DB).toBe(-1.05)
    expect(LOUDNESS_TOLERANCE_LU).toBe(1)
    expect(REVERB_SEND_BASE).toBe(0.08)
    expect(MICRO_VARIATION).toEqual({ rate: 0.03, gainDb: 1 })
  })
})

describe('contract spatial block', () => {
  it('pins the length and offsets', () => {
    expect(SP_LENGTH).toBe(2 + 3 * MAX_LISTENERS)
    expect(SP_LENGTH).toBe(14)
    expect(SP).toEqual({ COUNT: 0, SEND: 1, GAIN: 2, PAN: 6, LOWPASS: 10 })
    expect(spGain(3)).toBe(5)
    expect(spPan(0)).toBe(6)
    expect(spLowpass(3)).toBe(13)
  })

  it('creates an all-zero block at neutral rate', () => {
    const result = newSpatialResult()
    expect(result.sp).toHaveLength(14)
    expect(Array.from(result.sp)).toEqual(Array(14).fill(0))
    expect(result.rate).toBe(1)
  })
})

describe('contract DSP safety', () => {
  it('does not construct an AudioContext', () => {
    const source = readFileSync(resolve(import.meta.dirname, '../src/contract.ts'), 'utf8')
    expect(source).not.toMatch(/new\s+\w*AudioContext\b/)
  })
})
