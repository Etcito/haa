// The browser-safe public API of HAA (module repo). Every file it reaches is DSP-safe or
// browser-safe: no node: import. Node-only tools live in src/node.ts.
// APPEND-ONLY per milestone: each milestone appends the export lines its modules list and
// never removes or renames one.
// Name collisions: the contract types come from `export *`; the dsp and gen modules are
// namespaced (`dsp`, `gen`), and every other line names its exports explicitly, so no two
// sources export the same name (AudioRng and hashSeed are re-exported once, from dsp/rng.ts).

// M1
export * from './contract.ts'
export * as dsp from './dsp/index.ts'
export * as gen from './gen/index.ts'
export { AudioRng, hashSeed } from './dsp/rng.ts'
export { renderVariantRaw, renderFamilyVariants, renderFamilyVariantsAsync, oneShotLufs } from './bank/render.ts'
export { measureStereo, measureMono, safetyVerdict, OCTAVE_BANDS } from './analysis/measure.ts'
export type { Measure } from './analysis/measure.ts'
export { reportMarkdown, reportHtml } from './analysis/report.ts'
export type { ReportEntry, ReportCheck, Report } from './analysis/report.ts'
export { encodeWav, decodeWav } from './offline/wav.ts'
export { renderOffline, renderFamilyStrip, renderBedDemo } from './offline/render.ts'
export { MixChain } from './mix/mixChain.ts'
export { BASE_PRESET } from './mix/presets/base.ts'
export { BUILTIN_FAMILIES, BUILTIN_BEDS, BUILTIN_MIXES, BUILTIN_STYLES } from './content/index.ts'

// M2
export {
  defineFamily, defineBed, defineMix, defineStyle,
  registerFamily, registerFamilies, registerBed, registerMix, registerStyle,
  getFamily, listFamilies, getBed, listBeds, getMix, listMixes, activeMix,
  getStyle, listStyles, styleIndex, familyTrimDb, bedTrimDb, bankSeconds, clearRegistry,
} from './registry/registry.ts'
export { registerBuiltins } from './registry/builtins.ts'
export { packStrip, pcmHash, alignVariants } from './bank/pack.ts'

// M3
export { Arranger } from './music/arranger.ts'
export { MusicSynth, MUSIC_PARAMS } from './music/musicSynth.ts'
export { UPBEAT_STYLE, CALM_STYLE } from './content/styles.ts'
export { MUSIC_BED } from './content/musicBed.ts'

// M4
export { createOfflineAudio } from './offline/scene.ts'
export { FLAT_SPATIAL } from './runtime/flatSpatial.ts'
export { createAudio } from './runtime/createAudio.ts'

// M5
export { createSpatial } from './spatial/model.ts'
export {
  distanceGain, spatializeOne, dopplerRatio,
  SPEED_OF_SOUND, DOPPLER_AMOUNT, REF_DISTANCE, DEFAULT_RANGE, OPEN_LOWPASS_HZ, REAR_LOWPASS_HZ,
  AIR_ABSORPTION_M, OCCLUDED_MIN_HZ, OCCLUDED_GAIN_DB, SPLIT_PAN_WIDTH, REVERB_SEND_DIST,
  REVERB_SEND_DIST_M, DOPPLER_MIN, DOPPLER_MAX,
} from './spatial/spatial.ts'

