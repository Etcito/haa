// Royal Fiesta gameplay mix: the balance for the Royal Fiesta pack, selected explicitly with
// mix: 'fiesta'. Derived from BASE_PRESET. DSP-safe: pure data, no DOM, no Web Audio.
// priority -10 keeps activeMix() on 'base'.

import { BASE_PRESET, type BusId, type BusPreset, type DuckPreset, type MixPreset } from '@etcito/haa'

// Level overrides (dB) against the base preset; every other bus keeps its base level.
// HAA tuning: the race render sat at -31 LUFS with the base-derived levels (effects -5, ambience
// -13, music -13, ui -9); the bus compressors and the glue absorb most of a raise, so the buses
// gain +10..+13 dB to bring the 60 s race to the -16 LUFS scene target. effects stays 6..7 dB
// above music and ambience so the players' actions read over the music and the crowd; the
// per-1 s window DC of the low-frequency fiesta bodies caps how far effects can go.
const LEVEL_DB: Partial<Record<BusId, number>> = { effects: 7, ambience: -1, music: 0, ui: 4 }

// New bus objects: EQ and compressor are the base ones, only levelDb changes for three buses.
const buses = Object.fromEntries(
  (Object.keys(BASE_PRESET.buses) as BusId[]).map((id) => {
    const base = BASE_PRESET.buses[id]
    return [id, { ...base, levelDb: LEVEL_DB[id] ?? base.levelDb }]
  })
) as Record<BusId, BusPreset>

// The four base ducks plus the fiesta ones: music/ambience depth, then attack/hold/release in ms.
const ducks: Record<string, DuckPreset> = {
  ...BASE_PRESET.ducks,
  eliminate: { musicDb: -6, ambienceDb: -2, attackMs: 20, holdMs: 600, releaseMs: 900 },
  cheer: { musicDb: -4, ambienceDb: 0, attackMs: 40, holdMs: 1500, releaseMs: 1200 },
  qualify: { musicDb: -5, ambienceDb: -3, attackMs: 30, holdMs: 1000, releaseMs: 1000 }
}

/** The Royal Fiesta preset (priority -10): base balance with the buses raised for the race scene and the fiesta trims below. */
export const FIESTA_PRESET: MixPreset = {
  id: 'fiesta',
  priority: -10,
  buses,
  master: BASE_PRESET.master,
  ducks,
  loudness: { ...BASE_PRESET.loudness },
  sceneLufs: BASE_PRESET.sceneLufs,
  /** Per-family trim (dB), tuned in this file only. */
  familyTrimDb: {
    // HAA tuning: the fiesta actions are raised rather than the music and crowd lowered (Q9).
    // grab is the shortest and quietest body, so it gets the most; eliminate, squash and
    // wobble carry the most sub-100 Hz energy and stay lower to keep every 1 s window's DC
    // under 1e-3.
    'fiesta.jump': 13,
    'fiesta.dive': 13,
    'fiesta.grab': 16,
    'fiesta.eliminate': 9,
    'fiesta.wobble': 11,
    'fiesta.squash': 9,
    // HAA tuning: the crowd stings sit above the crowd bed so the 'aww' and the finish cheer read.
    'crowd.cheer.small': 8,
    'crowd.cheer.big': 8,
    'crowd.aww': 8
  },
  /** Per-bed trim (dB), tuned in this file only. */
  bedTrimDb: {
    // HAA tuning: music and crowd beds raised with their buses towards the -16 LUFS scene.
    music: 6,
    crowd: 7
  },
  reverb: { ...BASE_PRESET.reverb }
}
