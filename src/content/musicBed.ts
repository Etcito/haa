// musicBed: the built-in adaptive music bed (module music, owner M3). DSP-safe: it imports only
// `import type` from the contract and MusicSynth from ../music/musicSynth.ts.
// It replaces the engine's music bed: the same listener-level bed on the music bus, with the
// generic MUSIC_PARAMS instead of the biome/race params. create() builds a MusicSynth that reads
// listStyles() at creation, so the 'style' param n plays the n-th registered style (by id).
import type { BedDef, BedVoice } from '../contract.ts'
import { MUSIC_PARAMS, MusicSynth } from '../music/musicSynth.ts'

/**
 * The music bed: one listener-level MusicSynth on the music bus. Defaults: style 0, intensity 0.5,
 * phase 1 (main), fadeBars 2, level 1. The discrete params are not smoothed by the host.
 */
export const MUSIC_BED: BedDef = {
  id: 'music',
  bus: 'music',
  params: MUSIC_PARAMS,
  defaults: [0, 0, 0, 0.5, 1, 0, 0, 0, 0, 0, 2, 1],
  unsmoothed: ['style', 'styleB', 'phase', 'climax', 'resolved', 'won', 'fadeBars'],
  positional: false,
  create: (sr: number, seed: number): BedVoice => new MusicSynth(sr, seed),
}
