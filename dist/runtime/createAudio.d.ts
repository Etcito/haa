import type { AudioOptions, HaaAudio } from '../contract.js';
/**
 * Creates the browser audio engine on a context: mix, sampler and per-bed worklet nodes driven by a
 * Conductor. Rejects '[haa] worklet unavailable: <reason>' only when the worklet cannot be set up.
 */
export declare function createAudio(ctx: BaseAudioContext, o: AudioOptions): Promise<HaaAudio>;
