import type { MixPreset, OfflineAudio, SpatialModel, UserVolumes } from '../contract.js';
/** Creates a deterministic offline graph; families render synchronously on first play. */
export declare function createOfflineAudio(o?: {
    sampleRate?: number;
    mix?: string | MixPreset;
    maxListeners?: 1 | 2 | 3 | 4;
    spatial?: SpatialModel;
    seed?: number;
    volumes?: Partial<UserVolumes>;
}): OfflineAudio;
