import { defineStyle } from "../registry/registry.js";
const MAJOR = [0, 2, 4, 5, 7, 9, 11];
const DORIAN = [0, 2, 3, 5, 7, 9, 10];
// upbeat: bright major, lead-led and driving. Climax: the 8-bar I-IV-V-iii drive.
// HAA tuning: lead, pluck, drums and shaker up and the pad down so the top end and the beat lead
// (brighter and more driving than calm); swing nearly straight for a tighter, pushing groove.
// HAA tuning: progressions lean on I-V-vi-IV and IV-V lifts; the climax one climbs and ends on V.
export const UPBEAT_STYLE = defineStyle({
    id: 'upbeat', root: 60, scale: MAJOR, bpm: 128,
    palette: { pad: 0.38, bass: 0.55, lead: 0.7, pluck: 0.65, drums: 0.7, perc: 0.5 }, swing: 0.03,
    progressions: [[0, 4, 5, 3], [0, 3, 4, 4], [5, 3, 0, 4], [0, 5, 3, 4], [0, 3, 4, 5, 0, 3, 1, 4]],
});
// calm: dorian, pad- and pluck-heavy with light drums and more swing. Climax: the 8-bar modal drift.
// HAA tuning: pad up, drums, lead and perc down (perc <= 0.3 keeps the shaker off) so the pad and
// its reverb carry a soft, spacious bed; more swing for a lazier, floating pulse.
// HAA tuning: progressions dwell on the dorian i-IV and i-bVII colours with slow root motion.
export const CALM_STYLE = defineStyle({
    id: 'calm', root: 57, scale: DORIAN, bpm: 96,
    palette: { pad: 0.8, bass: 0.35, lead: 0.35, pluck: 0.6, drums: 0.18, perc: 0.2 }, swing: 0.16,
    progressions: [[0, 3, 0, 3], [0, 6, 3, 0], [2, 6, 3, 0], [0, 2, 3, 6, 0, 3, 4, 0]],
});
