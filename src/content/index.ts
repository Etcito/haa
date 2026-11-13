// The built-in content arrays (module content-core). DSP-safe. registerBuiltins() registers
// exactly these arrays, and the worklet entry calls it, so the prebuilt bundle knows them.
// APPEND-ONLY for later milestones: they add generic entries to these arrays (M3 the styles and
// MUSIC_BED, M5 BEACON_BED), never remove or rename one. Game or product content does not go
// here: it lives in its own pack and registers itself (see examples/).
import type { BedDef, FamilyDef, MixPreset, MusicStyle } from '../contract.ts'
import { BASE_PRESET } from '../mix/presets/base.ts'
import { IMPACT_FAMILIES } from './impacts.ts'
import { STING_FAMILIES } from './stings.ts'
import { UI_FAMILIES } from './ui.ts'
import { WIND_BED, WIND_FAMILIES } from './wind.ts'
import { CALM_STYLE, UPBEAT_STYLE } from './styles.ts'
import { MUSIC_BED } from './musicBed.ts'
import { BEACON_BED } from './beacon.ts'

/** Every built-in one-shot family, sorted by id. */
export const BUILTIN_FAMILIES: readonly FamilyDef[] = [
  ...IMPACT_FAMILIES,
  ...UI_FAMILIES,
  ...STING_FAMILIES,
  ...WIND_FAMILIES,
].sort((a, b) =>
  a.id < b.id ? -1 : a.id > b.id ? 1 : 0,
)

/** Every built-in bed. */
export const BUILTIN_BEDS: readonly BedDef[] = [WIND_BED, MUSIC_BED, BEACON_BED]

/** Every built-in mix preset. */
export const BUILTIN_MIXES: readonly MixPreset[] = [BASE_PRESET]

/** Every built-in music style: the M3 genre-neutral UPBEAT_STYLE and CALM_STYLE. */
export const BUILTIN_STYLES: readonly MusicStyle[] = [UPBEAT_STYLE, CALM_STYLE]
