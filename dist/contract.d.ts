import type { AudioRng } from './dsp/rng.js';
/** Version of the frozen public contract. */
export declare const HAA_CONTRACT_VERSION = 0;
/** Offline render and bake rate (Hz). The worklet uses the context rate. */
export declare const AUDIO_RATE = 48000;
/** Host render block (frames). Processors never allocate in process(). */
export declare const BLOCK = 128;
/** Maximum listeners represented by the spatial block. */
export declare const MAX_LISTENERS = 4;
/** Audio buses in mix-input order. */
export type BusId = 'world' | 'foley' | 'effects' | 'ambience' | 'music' | 'ui';
/** Audio buses in mix-input order. */
export declare const BUSES: readonly BusId[];
/** Mix input index for each bus. */
export declare const BUS_INDEX: Record<BusId, number>;
/** Mix input index for the reverb return. */
export declare const REVERB_INPUT = 6;
/** Number of stereo inputs to the mix processor. */
export declare const MIX_INPUTS = 7;
/** Material labels for authored impact families. */
export type MaterialId = 'metal' | 'plastic' | 'wood' | 'concrete';
/** Material labels in stable index order. */
export declare const MATERIALS: readonly MaterialId[];
/** One-shot voice allocation categories. */
export type VoiceCategory = 'impact' | 'fx' | 'ui' | 'ambience' | 'sting';
/** Voice categories in stable order. */
export declare const VOICE_CATEGORIES: readonly VoiceCategory[];
/** Maximum simultaneous voices for each category. */
export declare const VOICE_LIMITS: Record<VoiceCategory, number>;
/** Equal to the save defaults; loudness targets are measured at these levels. */
export declare const DEFAULT_USER_VOLUMES: {
    master: number;
    music: number;
    sfx: number;
};
/** Minimum number of rendered variants held by a family. */
export declare const FAMILY_VARIANTS_MIN = 8;
/** Maximum number of rendered variants held by a family. */
export declare const FAMILY_VARIANTS_MAX = 32;
/** Maximum authored duration of a family variant (seconds). */
export declare const FAMILY_MAX_SECONDS = 10;
/** Zero samples separating variants in a packed strip. */
export declare const GAP_SAMPLES = 4800;
/** Minimum absolute sample value used to find a sound onset. */
export declare const ONSET_THRESHOLD = 0.001;
/** Worklet processor names. */
export declare const PROCESSOR_NAMES: {
    readonly bed: "haa-bed";
    readonly sampler: "haa-sampler";
    readonly mix: "haa-mix";
};
/** Maximum sampler voices. */
export declare const SAMPLER_VOICES = 64;
/** Measured safety bounds for rendered audio. */
export declare const SAFETY: {
    readonly maxSamplePeakDb: -1;
    readonly maxAbsDc: 0.001;
    readonly minRmsDb: -60;
};
/** Peak ceiling applied while normalizing family variants (dBFS). */
export declare const NORMALIZE_PEAK_CAP_DB = -1.05;
/** Maximum allowed difference from a one-shot loudness target (LU). */
export declare const LOUDNESS_TOLERANCE_LU = 1;
/** Default reverb send for spatial sources. */
export declare const REVERB_SEND_BASE = 0.08;
/** Per-play seeded rate and gain variation bounds. */
export declare const MICRO_VARIATION: {
    readonly rate: 0.03;
    readonly gainDb: 1;
};
/** Three-dimensional position, direction or velocity in meters or meters per second. */
export interface Vec3 {
    x: number;
    y: number;
    z: number;
}
/** Quaternion orientation. */
export interface Quat {
    x: number;
    y: number;
    z: number;
    w: number;
}
/** Listener position, orientation and optional velocity. */
export interface Listener {
    pos: Vec3;
    quat: Quat;
    vel?: Vec3;
}
/** Source transform and optional range and occlusion. */
export interface SourcePos {
    pos: Vec3;
    vel?: Vec3;
    range?: number;
    occlusion?: number;
}
/** Returns source occlusion from 0 (open) to 1 (fully occluded) for a listener. */
export type OcclusionFn = (src: Vec3, listener: number) => number;
/** Per-source spatial parameters and its Doppler playback ratio. */
export interface SpatialResult {
    sp: Float32Array;
    rate: number;
}
/** Evaluates a source against listeners into a reusable spatial result. */
export interface SpatialModel {
    evaluate(listeners: readonly Listener[], src: SourcePos | null, occlusion: OcclusionFn | null, out: SpatialResult): SpatialResult;
}
/** Spatial parameter indices: count, send, then per-listener gain, pan and lowpass arrays. */
export declare const SP: {
    readonly COUNT: 0;
    readonly SEND: 1;
    readonly GAIN: 2;
    readonly PAN: 6;
    readonly LOWPASS: 10;
};
/** Number of floats in the spatial parameter block. */
export declare const SP_LENGTH: number;
/** Index of listener l's gain in a spatial parameter block. */
export declare function spGain(l: number): number;
/** Index of listener l's pan in a spatial parameter block. */
export declare function spPan(l: number): number;
/** Index of listener l's lowpass frequency in a spatial parameter block. */
export declare function spLowpass(l: number): number;
/** Creates a cleared spatial block with neutral Doppler rate. */
export declare function newSpatialResult(): SpatialResult;
/** Mono family renderer. It adds samples to a cleared output and returns frames written. */
export interface FamilyDef {
    id: string;
    bus: BusId;
    category: VoiceCategory;
    variants: number;
    maxSeconds: number;
    seed: number;
    render(out: Float32Array, sr: number, rng: AudioRng, v: number): number;
}
/** Family definition with a seed supplied by the registry when omitted. */
export type FamilySpec = Omit<FamilyDef, 'seed'> & {
    seed?: number;
};
/** Stereo bed processor. It adds to the outputs and reads parameters at p[offset..]. */
export interface BedVoice {
    process(p: Float32Array, offset: number, l: Float32Array, r: Float32Array, n: number): void;
    reset(): void;
}
/** Definition and parameter layout for a procedural bed. */
export interface BedDef {
    id: string;
    bus: BusId;
    params: readonly string[];
    defaults: readonly number[];
    unsmoothed?: readonly string[];
    positional: boolean;
    create(sr: number, seed: number): BedVoice;
}
/** Genre-neutral music arrangement data. */
export interface MusicStyle {
    id: string;
    root: number;
    scale: readonly number[];
    bpm: number;
    palette: {
        pad: number;
        bass: number;
        lead: number;
        pluck: number;
        drums: number;
        perc: number;
    };
    swing: number;
    progressions: readonly (readonly number[])[];
}
/** Reverb parameters; size is 0..1, decay and predelay are seconds, damping is Hz. */
export interface ReverbParams {
    size: number;
    decay: number;
    damping: number;
    predelay: number;
    returnDb: number;
}
/** Compressor settings for a mix preset. */
export interface CompPreset {
    thresholdDb: number;
    ratio: number;
    kneeDb: number;
    attackMs: number;
    releaseMs: number;
    makeupDb: number;
}
/** Three-band equalizer settings for a mix preset. */
export interface EqPreset {
    lowShelfHz: number;
    lowShelfDb: number;
    peakHz: number;
    peakDb: number;
    peakQ: number;
    highShelfHz: number;
    highShelfDb: number;
}
/** Level, equalization and compression settings for one bus. */
export interface BusPreset {
    levelDb: number;
    eq: EqPreset;
    comp: CompPreset;
}
/** Music and ambience attenuation envelope for a duck trigger. */
export interface DuckPreset {
    musicDb: number;
    ambienceDb: number;
    attackMs: number;
    holdMs: number;
    releaseMs: number;
}
/** Full bus, master, ducking, loudness and reverb mix configuration. */
export interface MixPreset {
    id: string;
    priority: number;
    buses: Record<BusId, BusPreset>;
    master: {
        trimDb: number;
        glue: CompPreset;
        limiter: {
            ceilingDbtp: number;
            lookaheadMs: number;
            releaseMs: number;
        };
    };
    ducks: Record<string, DuckPreset>;
    loudness: Record<VoiceCategory, number>;
    sceneLufs: number;
    familyTrimDb: Record<string, number>;
    bedTrimDb: Record<string, number>;
    reverb: ReverbParams;
}
/** One baked family's metadata and strip location in a bank manifest. */
export interface BankFamilyEntry {
    id: string;
    file: string;
    bus: BusId;
    category: VoiceCategory;
    variants: {
        start: number;
        length: number;
    }[];
    onset0: number;
    pcmHash: string;
    mp3Sha256: string;
}
/** Versioned description of a baked audio bank and its encoded family files. */
export interface BankManifest {
    version: 1;
    sampleRate: 48000;
    kbps: 128;
    encoder: 'lamejs 1.2.1';
    preset: string;
    families: BankFamilyEntry[];
}
/** Messages sent to the sampler processor. */
export type SamplerMsg = {
    t: 'load';
    id: string;
    variants: Float32Array[];
    rate: number;
} | {
    t: 'play';
    voice: number;
    id: string;
    variant: number;
    bus: number;
    gain: number;
    rate: number;
    sp: Float32Array;
} | {
    t: 'upd';
    voice: number;
    rate: number;
    sp: Float32Array;
} | {
    t: 'stop';
    voice: number;
    fade: number;
} | {
    t: 'stopAll';
    fade: number;
};
/** Messages sent to each bed processor. */
export type BedMsg = {
    t: 'p';
    v: Float32Array;
} | {
    t: 'sp';
    rate: number;
    sp: Float32Array;
} | {
    t: 'end';
    fade: number;
};
/** Messages sent to the mix processor. */
export type MixMsg = {
    t: 'preset';
    p: MixPreset;
} | {
    t: 'user';
    master: number;
    music: number;
    sfx: number;
} | {
    t: 'duck';
    kind: string;
    strength: number;
} | {
    t: 'reverb';
    p: ReverbParams;
} | {
    t: 'muffle';
    v: number;
};
/** Per-play overrides; gain is linear (default 1) and pitch is in semitones (default 0). */
export interface PlayOptions {
    pos?: Vec3;
    vel?: Vec3;
    gain?: number;
    pitch?: number;
    bus?: BusId;
    variant?: number;
    range?: number;
    occlusion?: number;
    listener?: number;
}
/** Handle for a playing one-shot voice. */
export interface Shot {
    readonly voice: number;
    setPos(pos: Vec3, vel?: Vec3): void;
    stop(fadeSec?: number): void;
}
/** Handle for a running procedural bed. */
export interface BedHandle {
    readonly id: string;
    set(values: ArrayLike<number>): void;
    setParam(name: string, value: number): void;
    setPos(pos: Vec3, vel?: Vec3): void;
    stop(fadeSec?: number): void;
}
/** Names of the music-bed parameters exposed through music controls. */
export type MusicParam = 'intensity' | 'phase' | 'climax' | 'resolved' | 'won' | 'sparkle' | 'tension' | 'level';
/** Controls for selecting and shaping procedural music. */
export interface MusicControl {
    readonly style: string | null;
    play(style: string, o?: {
        seed?: number;
        intensity?: number;
    }): void;
    crossfade(style: string, bars: number): void;
    set(name: MusicParam, v: number): void;
    stop(fadeSec?: number): void;
}
/** User-facing linear volume multipliers. */
export interface UserVolumes {
    master: number;
    music: number;
    sfx: number;
}
/** Configuration for creating a browser audio engine. */
export interface AudioOptions {
    manifestUrl: string;
    processorUrl: string;
    mix?: string | MixPreset;
    maxListeners?: 1 | 2 | 3 | 4;
    spatial?: SpatialModel;
    seed?: number;
    /** Family ids to load; default is every registered family, built-ins included. */
    families?: readonly string[];
}
/** Audio engine controls shared by browser and offline implementations. */
export interface AudioControls {
    play(id: string, o?: PlayOptions): Shot | null;
    startBed(id: string, o?: {
        seed?: number;
        pos?: Vec3;
        vel?: Vec3;
        values?: ArrayLike<number>;
    }): BedHandle | null;
    setListeners(ls: readonly Listener[]): void;
    setSpatial(m: SpatialModel | null): void;
    setOcclusion(fn: OcclusionFn | null): void;
    duck(kind: string, strength?: number): void;
    setVolumes(v: Partial<UserVolumes>): void;
    setReverb(p: ReverbParams): void;
    setMuffle(v: number): void;
    readonly music: MusicControl;
}
/** Browser audio engine controls, context and optional analysers. */
export interface HaaAudio extends AudioControls {
    readonly ctx: BaseAudioContext;
    readonly meters: {
        mix: AnalyserNode;
        left: AnalyserNode;
        right: AnalyserNode;
    } | null;
    ready(): Promise<void>;
    close(): Promise<void>;
}
/** Named stereo buffer returned by an offline render. */
export interface StereoRender {
    name: string;
    left: Float32Array;
    right: Float32Array;
    sampleRate: number;
}
/** Offline audio engine controls and deterministic render advancement. */
export interface OfflineAudio extends AudioControls {
    readonly time: number;
    readonly stats: {
        played: number;
        dropped: number;
        stolen: number;
    };
    advance(seconds: number): void;
    take(name: string): StereoRender;
}
