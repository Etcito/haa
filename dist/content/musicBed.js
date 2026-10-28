import { MUSIC_PARAMS, MusicSynth } from "../music/musicSynth.js";
/**
 * The music bed: one listener-level MusicSynth on the music bus. Defaults: style 0, intensity 0.5,
 * phase 1 (main), fadeBars 2, level 1. The discrete params are not smoothed by the host.
 */
export const MUSIC_BED = {
    id: 'music',
    bus: 'music',
    params: MUSIC_PARAMS,
    defaults: [0, 0, 0, 0.5, 1, 0, 0, 0, 0, 0, 2, 1],
    unsmoothed: ['style', 'styleB', 'phase', 'climax', 'resolved', 'won', 'fadeBars'],
    positional: false,
    create: (sr, seed) => new MusicSynth(sr, seed),
};
