// styles: the two built-in genre-neutral demo styles (module music, owner M3). DSP-safe: it runs
// unchanged in node, on the main thread and in AudioWorkletGlobalScope. It imports only
// `import type` plus the registry's defineStyle, never the platform RNG, clocks or Web Audio.
//
// UPBEAT_STYLE and CALM_STYLE are the first entries of BUILTIN_STYLES: the music bed task appends
// them to the content-core array in src/content/index.ts, so this file stays append-only content.
// The order in that array defines the numeric 'style' and 'styleB' music bed params: the index in
// listStyles() names the style of param value n. Each style is wrapped in defineStyle, so an
// invalid authored value throws '[haa] bad style <id>: <reason>' at load.
//
// root, scale and bpm follow the shared contract. Palettes are linear 0..1 levels for pad, bass,
// lead, pluck, drums and perc; swing is 0..0.3. Progressions list chord roots as 0-based
// scale-degree indices; each is 4 or 8 long and every index is < scale.length. Every style has at
// least one progression starting on degree 0, and its 8-long progression is the climax one.
import type { MusicStyle } from '../contract.ts'
import { defineStyle } from '../registry/registry.ts'

const MAJOR: readonly number[] = [0, 2, 4, 5, 7, 9, 11]
const DORIAN: readonly number[] = [0, 2, 3, 5, 7, 9, 10]

// upbeat: bright major, lead-led and driving. Climax: the 8-bar I-IV-V-iii drive.
// HAA tuning: lead, pluck, drums and shaker up and the pad down so the top end and the beat lead
// (brighter and more driving than calm); swing nearly straight for a tighter, pushing groove.
// HAA tuning: progressions lean on I-V-vi-IV and IV-V lifts; the climax one climbs and ends on V.
export const UPBEAT_STYLE: MusicStyle = defineStyle({
  id: 'upbeat', root: 60, scale: MAJOR, bpm: 128,
  palette: { pad: 0.38, bass: 0.55, lead: 0.7, pluck: 0.65, drums: 0.7, perc: 0.5 }, swing: 0.03,
  progressions: [[0, 4, 5, 3], [0, 3, 4, 4], [5, 3, 0, 4], [0, 5, 3, 4], [0, 3, 4, 5, 0, 3, 1, 4]],
})

// calm: dorian, pad- and pluck-heavy with light drums and more swing. Climax: the 8-bar modal drift.
// HAA tuning: pad up, drums, lead and perc down (perc <= 0.3 keeps the shaker off) so the pad and
// its reverb carry a soft, spacious bed; more swing for a lazier, floating pulse.
// HAA tuning: progressions dwell on the dorian i-IV and i-bVII colours with slow root motion.
export const CALM_STYLE: MusicStyle = defineStyle({
  id: 'calm', root: 57, scale: DORIAN, bpm: 96,
  palette: { pad: 0.8, bass: 0.35, lead: 0.35, pluck: 0.6, drums: 0.18, perc: 0.2 }, swing: 0.16,
  progressions: [[0, 3, 0, 3], [0, 6, 3, 0], [2, 6, 3, 0], [0, 2, 3, 6, 0, 3, 4, 0]],
})
