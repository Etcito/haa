// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: renamed buses, generic duck kinds, loudness targets added
// Base mix preset: the neutral default balance, the fallback when no higher-priority preset
// is registered. DSP-safe: pure data, no runtime imports, no DOM, no Web Audio.
// limiter.ceilingDbtp stays at or below -1.2 dBTP in every preset.

import type { BusId, BusPreset, CompPreset, DuckPreset, EqPreset, MixPreset, VoiceCategory } from '../../contract.ts'

// Neutral EQ: shelves at 120 Hz / 8 kHz and the peak at 1 kHz, all 0 dB, Q 0.7.
function eq(lowShelfDb: number, peakHz: number, peakDb: number, peakQ: number, highShelfDb: number): EqPreset {
  return {
    lowShelfHz: 120,
    lowShelfDb,
    peakHz,
    peakDb,
    peakQ,
    highShelfHz: 8000,
    highShelfDb
  }
}

const NEUTRAL = eq(0, 1000, 0, 0.7, 0)
// World (engine) keeps its low shelf boost and high shelf cut; music cuts the 2.5 kHz peak.
const WORLD_EQ = eq(1.5, 1000, 0, 0.7, -2)
const MUSIC_EQ = eq(0, 2500, -2, 0.8, 0)

// Every bus compressor: -18 dB, ratio 2.5, knee 6, 150 ms release, no makeup.
function comp(attackMs: number): CompPreset {
  return { thresholdDb: -18, ratio: 2.5, kneeDb: 6, attackMs, releaseMs: 150, makeupDb: 0 }
}

// Bus mapping from the engine: engine → world, tires → foley; the other buses keep their names.
const buses: Record<BusId, BusPreset> = {
  world: { levelDb: -8, eq: WORLD_EQ, comp: comp(10) },
  foley: { levelDb: -11, eq: NEUTRAL, comp: comp(10) },
  effects: { levelDb: -6, eq: NEUTRAL, comp: comp(3) },
  ambience: { levelDb: -15, eq: NEUTRAL, comp: comp(10) },
  music: { levelDb: -12, eq: MUSIC_EQ, comp: comp(10) },
  ui: { levelDb: -9, eq: NEUTRAL, comp: comp(3) }
}

// music/ambience depth, then attack/hold/release in ms.
const ducks: Record<string, DuckPreset> = {
  impact: { musicDb: -5, ambienceDb: -4, attackMs: 10, holdMs: 300, releaseMs: 600 },
  fx: { musicDb: -4, ambienceDb: -3, attackMs: 20, holdMs: 250, releaseMs: 500 },
  cue: { musicDb: -6, ambienceDb: -4, attackMs: 40, holdMs: 1200, releaseMs: 800 },
  sting: { musicDb: -8, ambienceDb: -6, attackMs: 60, holdMs: 3000, releaseMs: 1500 }
}

// One-shot loudness targets per category (integrated LUFS). These are lead decisions: they
// are never changed to pass a check.
const loudness: Record<VoiceCategory, number> = {
  impact: -21,
  fx: -20,
  ui: -24,
  ambience: -26,
  sting: -18
}

/** The base preset (priority 0): the default balance of every bus, duck and loudness target. */
export const BASE_PRESET: MixPreset = {
  id: 'base',
  priority: 0,
  buses,
  master: {
    trimDb: 0,
    glue: { thresholdDb: -12, ratio: 1.8, kneeDb: 8, attackMs: 20, releaseMs: 200, makeupDb: 0 },
    limiter: { ceilingDbtp: -1.5, lookaheadMs: 5, releaseMs: 80 }
  },
  ducks,
  loudness,
  // Scene integrated loudness target (LUFS), also a lead decision.
  sceneLufs: -16,
  familyTrimDb: {},
  bedTrimDb: {},
  reverb: { size: 0.5, decay: 1.2, damping: 6000, predelay: 0.02, returnDb: -14 }
}
