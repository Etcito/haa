// The browser-safe public API of HAA (module repo). Every file it reaches is DSP-safe or
// browser-safe: no node: import. Node-only tools live in src/node.ts.
// APPEND-ONLY per milestone: each milestone appends the export lines its modules list and
// never removes or renames one.
// Name collisions: the contract types come from `export *`; the dsp and gen modules are
// namespaced (`dsp`, `gen`), and every other line names its exports explicitly, so no two
// sources export the same name (AudioRng and hashSeed are re-exported once, from dsp/rng.ts).
// M1
export * from "./contract.js";
export * as dsp from "./dsp/index.js";
export * as gen from "./gen/index.js";
export { AudioRng, hashSeed } from "./dsp/rng.js";
export { renderVariantRaw, renderFamilyVariants, renderFamilyVariantsAsync, oneShotLufs } from "./bank/render.js";
export { measureStereo, measureMono, safetyVerdict, OCTAVE_BANDS } from "./analysis/measure.js";
export { reportMarkdown, reportHtml } from "./analysis/report.js";
export { encodeWav, decodeWav } from "./offline/wav.js";
export { renderOffline, renderFamilyStrip, renderBedDemo } from "./offline/render.js";
export { MixChain } from "./mix/mixChain.js";
export { BASE_PRESET } from "./mix/presets/base.js";
export { BUILTIN_FAMILIES, BUILTIN_BEDS, BUILTIN_MIXES, BUILTIN_STYLES } from "./content/index.js";
// M2
export { defineFamily, defineBed, defineMix, defineStyle, registerFamily, registerFamilies, registerBed, registerMix, registerStyle, getFamily, listFamilies, getBed, listBeds, getMix, listMixes, activeMix, getStyle, listStyles, styleIndex, familyTrimDb, bedTrimDb, bankSeconds, clearRegistry, } from "./registry/registry.js";
export { registerBuiltins } from "./registry/builtins.js";
export { packStrip, pcmHash, alignVariants } from "./bank/pack.js";
// M3
export { Arranger } from "./music/arranger.js";
export { MusicSynth, MUSIC_PARAMS } from "./music/musicSynth.js";
export { UPBEAT_STYLE, CALM_STYLE } from "./content/styles.js";
export { MUSIC_BED } from "./content/musicBed.js";
// M4
export { createOfflineAudio } from "./offline/scene.js";
export { FLAT_SPATIAL } from "./runtime/flatSpatial.js";
export { createAudio } from "./runtime/createAudio.js";
// M5
export { createSpatial } from "./spatial/model.js";
export { distanceGain, spatializeOne, dopplerRatio, SPEED_OF_SOUND, DOPPLER_AMOUNT, REF_DISTANCE, DEFAULT_RANGE, OPEN_LOWPASS_HZ, REAR_LOWPASS_HZ, AIR_ABSORPTION_M, OCCLUDED_MIN_HZ, OCCLUDED_GAIN_DB, SPLIT_PAN_WIDTH, REVERB_SEND_DIST, REVERB_SEND_DIST_M, DOPPLER_MIN, DOPPLER_MAX, } from "./spatial/spatial.js";
