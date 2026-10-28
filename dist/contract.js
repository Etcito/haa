// --- Constants ---------------------------------------------------------------------------
/** Version of the frozen public contract. */
export const HAA_CONTRACT_VERSION = 0;
/** Offline render and bake rate (Hz). The worklet uses the context rate. */
export const AUDIO_RATE = 48000;
/** Host render block (frames). Processors never allocate in process(). */
export const BLOCK = 128;
/** Maximum listeners represented by the spatial block. */
export const MAX_LISTENERS = 4;
/** Audio buses in mix-input order. */
export const BUSES = ['world', 'foley', 'effects', 'ambience', 'music', 'ui'];
/** Mix input index for each bus. */
export const BUS_INDEX = {
    world: 0, foley: 1, effects: 2, ambience: 3, music: 4, ui: 5
};
/** Mix input index for the reverb return. */
export const REVERB_INPUT = 6;
/** Number of stereo inputs to the mix processor. */
export const MIX_INPUTS = 7;
/** Material labels in stable index order. */
export const MATERIALS = ['metal', 'plastic', 'wood', 'concrete'];
/** Voice categories in stable order. */
export const VOICE_CATEGORIES = ['impact', 'fx', 'ui', 'ambience', 'sting'];
/** Maximum simultaneous voices for each category. */
export const VOICE_LIMITS = {
    impact: 12, fx: 16, ui: 4, ambience: 8, sting: 2
};
/** Equal to the save defaults; loudness targets are measured at these levels. */
export const DEFAULT_USER_VOLUMES = { master: 0.8, music: 0.6, sfx: 0.8 };
/** Minimum number of rendered variants held by a family. */
export const FAMILY_VARIANTS_MIN = 8;
/** Maximum number of rendered variants held by a family. */
export const FAMILY_VARIANTS_MAX = 32;
/** Maximum authored duration of a family variant (seconds). */
export const FAMILY_MAX_SECONDS = 10;
/** Zero samples separating variants in a packed strip. */
export const GAP_SAMPLES = 4800;
/** Minimum absolute sample value used to find a sound onset. */
export const ONSET_THRESHOLD = 1e-3;
/** Worklet processor names. */
export const PROCESSOR_NAMES = { bed: 'haa-bed', sampler: 'haa-sampler', mix: 'haa-mix' };
/** Maximum sampler voices. */
export const SAMPLER_VOICES = 64;
/** Measured safety bounds for rendered audio. */
export const SAFETY = { maxSamplePeakDb: -1, maxAbsDc: 1e-3, minRmsDb: -60 };
/** Peak ceiling applied while normalizing family variants (dBFS). */
export const NORMALIZE_PEAK_CAP_DB = -1.05;
/** Maximum allowed difference from a one-shot loudness target (LU). */
export const LOUDNESS_TOLERANCE_LU = 1;
/** Default reverb send for spatial sources. */
export const REVERB_SEND_BASE = 0.08;
/** Per-play seeded rate and gain variation bounds. */
export const MICRO_VARIATION = { rate: 0.03, gainDb: 1 };
/** Spatial parameter indices: count, send, then per-listener gain, pan and lowpass arrays. */
export const SP = { COUNT: 0, SEND: 1, GAIN: 2, PAN: 6, LOWPASS: 10 };
/** Number of floats in the spatial parameter block. */
export const SP_LENGTH = 2 + 3 * MAX_LISTENERS;
/** Index of listener l's gain in a spatial parameter block. */
export function spGain(l) { return SP.GAIN + l; }
/** Index of listener l's pan in a spatial parameter block. */
export function spPan(l) { return SP.PAN + l; }
/** Index of listener l's lowpass frequency in a spatial parameter block. */
export function spLowpass(l) { return SP.LOWPASS + l; }
/** Creates a cleared spatial block with neutral Doppler rate. */
export function newSpatialResult() { return { sp: new Float32Array(SP_LENGTH), rate: 1 }; }
