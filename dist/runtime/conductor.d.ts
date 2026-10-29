import type { AudioControls, BedDef, BedHandle, BedMsg, Listener, MixMsg, MixPreset, MusicControl, OcclusionFn, PlayOptions, ReverbParams, SamplerMsg, Shot, SpatialModel, UserVolumes } from '../contract.js';
/** Transport supplied by the browser or offline host. */
export interface ConductorBackend {
    sampler(m: SamplerMsg): void;
    mix(m: MixMsg): void;
    startBed(handle: number, def: BedDef, seed: number, trimDb: number, values: Float32Array): boolean;
    bed(handle: number, m: BedMsg): void;
}
type Options = {
    preset: MixPreset;
    maxListeners: number;
    seed: number;
    spatial: SpatialModel | null;
    backend: ConductorBackend;
    now: () => number;
    loaded: (id: string) => number;
    variantLength: (id: string, v: number) => number;
    loadRate: number;
};
type BedOptions = NonNullable<Parameters<AudioControls['startBed']>[1]>;
/** Shared control brain: seeded variation, voice admission, spatial updates and music. */
export declare class Conductor implements AudioControls {
    #private;
    /** A single managed music bed. */
    readonly music: MusicControl;
    /** Uses the supplied clock and transport in any realm. */
    constructor(o: Options);
    /** Successful plays and admission failures, including unloaded families. */
    get stats(): {
        played: number;
        dropped: number;
        stolen: number;
    };
    /** Selects a variant, applies variation and spatial gain, then admits a voice. */
    play(id: string, o?: PlayOptions): Shot | null;
    /** Starts an independently seeded bed with defaults overridden by supplied values. */
    startBed(id: string, o?: BedOptions): BedHandle | null;
    /** Caps listeners and refreshes live positional sources after expiry. */
    setListeners(ls: readonly Listener[]): void;
    /** Replaces the spatial model and immediately re-evaluates sources. */
    setSpatial(m: SpatialModel | null): void;
    /** Replaces the occlusion callback and immediately re-evaluates sources. */
    setOcclusion(fn: OcclusionFn | null): void;
    /** Triggers a named preset duck; unknown kinds are ignored. */
    duck(kind: string, strength?: number): void;
    /** Merges and clamps user volume multipliers. */
    setVolumes(v: Partial<UserVolumes>): void;
    /** Changes the mix reverb. */
    setReverb(p: ReverbParams): void;
    /** Changes the mix muffle amount. */
    setMuffle(v: number): void;
}
export {};
