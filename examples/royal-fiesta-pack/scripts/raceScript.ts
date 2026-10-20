// Deterministic 60 s Royal Fiesta race script (module fiesta-race, node): eight players, their
// paths and the timed event list that scripts/fiesta-race.ts renders through the offline scene.
//
// Everything derives from FIESTA_RACE.seed through AudioRng/hashSeed, so two imports give
// identical paths and events.
//
// Geometry: player 0 runs along -Z from the origin at about 8 m/s with a slight seeded sway.
// Players 1..7 run in lanes at x = ±2..8 m, each with a seeded speed (6.5..9.5 m/s) and a seeded
// start offset along Z. Everyone starts on 'cue.go' (3 s). An eliminated player stops at its
// elimination point.
//
// Listener: the listener follows player 0 at +2 m Z (behind) and +1.5 m Y, facing -Z (identity
// quat). The renderer applies this; the script only gives the player paths.
//
// Events (sorted by t, ties in insertion order):
//   - 'play': family id, optional player (positional at paths(t, player)); without player it is
//     listener-level.
//   - 'duck': duck = the preset duck kind, strength.
//   - 'bed': param/value of the 'crowd' bed (started by the renderer with its defaults).
//   - 'music': with style, music.play(style, { intensity: value }); otherwise music.set(param, value).
//
// Voice budget: no more than 2 'sting'-category sounds (crowd.*, sting.*) and 4 ui sounds ever
// overlap (VOICE_LIMITS in src/contract.ts), checked at build time with each family's maxSeconds
// stretched by the slowest micro-variation rate.
import { AudioRng, MICRO_VARIATION, VOICE_LIMITS, hashSeed, type Vec3 } from '@etcito/haa';

/** One scripted race event. */
export interface RaceEvent {
  t: number;
  kind: 'play' | 'duck' | 'bed' | 'music';
  family?: string;
  player?: number;
  gain?: number;
  duck?: string;
  strength?: number;
  param?: string;
  value?: number;
  style?: string;
}

/** The race seed. */
const SEED = hashSeed('fiesta-race', 1);
/** Race length (s), player count, the go time and the finish time (s). */
const SECONDS = 60;
const PLAYERS = 8;
const GO_T = 3;
const FINISH_T = 55;
/** Player 0 speed (m/s), its sway amplitude (m) and sway rate (Hz). */
const LEAD_SPEED = 8;
const SWAY_M = 0.35;
const SWAY_HZ = 0.27;
/** Lane x positions (m) the seven other players are dealt from, and their speed range (m/s). */
const LANES = [-8, -6, -4, -2, 2, 4, 6, 8];
const SPEED_MIN = 6.5;
const SPEED_MAX = 9.5;
/** Start offset range along Z (m). */
const START_OFFSET_M = 4;
/** Players farther than this from player 0 emit no action (m). */
const NEAR_M = 25;
/** Action interval per player (s) and obstacle-hit interval (s). */
const ACTION_MIN_S = 1.5;
const ACTION_MAX_S = 3;
const HIT_MIN_S = 4;
const HIT_MAX_S = 7;
/** Elimination times (s), and the swell release after an elimination or the finish (s). */
const ELIMINATION_T = [20, 35, 48];
const SWELL_RELEASE_S = 1.5;
/** Crowd excitement ramp: from 0.3 at 0 s to 0.9, one step every EXCITE_STEP_S. */
const EXCITE_FROM = 0.3;
const EXCITE_TO = 0.9;
const EXCITE_STEP_S = 4;
const EXCITE_END_T = 52;
/** Music intensity steps (time s, intensity), and the climax time (s). */
const INTENSITY_STEPS: readonly (readonly [number, number])[] = [[0, 0.4], [10, 0.5], [20, 0.6], [30, 0.7], [44, 0.8]];
const CLIMAX_T = 45;
/** Mid-race cheer window (s), clear of the elimination 'aww' shots. */
const MID_CHEER_FROM = 24;
const MID_CHEER_TO = 31;

/** maxSeconds of the voice-limited families the script plays (src/content/ui.ts, stings.ts,
 *  fiesta/crowd.ts); the build-time overlap check uses them. */
const LIMITED: Record<string, { category: 'ui' | 'sting'; seconds: number }> = {
  'cue.countdown': { category: 'ui', seconds: 0.3 },
  'cue.go': { category: 'ui', seconds: 0.45 },
  'sting.win': { category: 'sting', seconds: 1.6 },
  'crowd.cheer.small': { category: 'sting', seconds: 1.5 },
  'crowd.cheer.big': { category: 'sting', seconds: 3.0 },
  'crowd.aww': { category: 'sting', seconds: 1.8 }
};

const ACTIONS = ['fiesta.jump', 'fiesta.dive', 'fiesta.grab'];
const HITS = ['fiesta.wobble', 'fiesta.squash'];

/** Per-player path parameters. */
interface Runner { x: number; z0: number; speed: number; swayPhase: number; stopT: number }

/** Rounds a time to the millisecond so the event list prints cleanly. */
function ms(t: number): number {
  return Math.round(t * 1000) / 1000;
}

function makeRunners(rng: AudioRng): Runner[] {
  const lanes = LANES.slice();
  for (let i = lanes.length - 1; i > 0; i--) {
    const j = rng.int(0, i);
    const tmp = lanes[i];
    lanes[i] = lanes[j];
    lanes[j] = tmp;
  }
  const runners: Runner[] = [{ x: 0, z0: 0, speed: LEAD_SPEED, swayPhase: rng.range(0, 2 * Math.PI), stopT: Infinity }];
  for (let p = 1; p < PLAYERS; p++) {
    runners.push({
      x: lanes[p - 1],
      z0: rng.range(-START_OFFSET_M, START_OFFSET_M),
      speed: rng.range(SPEED_MIN, SPEED_MAX),
      swayPhase: rng.range(0, 2 * Math.PI),
      stopT: Infinity
    });
  }
  return runners;
}

/** Writes player's position at time t into out and returns it; allocates nothing. */
function pathOf(runners: readonly Runner[], t: number, player: number, out: Vec3): Vec3 {
  const r = runners[player];
  if (r === undefined) {
    out.x = 0;
    out.y = 0;
    out.z = 0;
    return out;
  }
  const tc = Math.min(Math.max(t, 0), r.stopT);
  const run = Math.max(0, tc - GO_T);
  const sway = player === 0 ? SWAY_M : SWAY_M * 0.6;
  out.x = r.x + sway * Math.sin(2 * Math.PI * SWAY_HZ * tc + r.swayPhase);
  out.y = 0;
  out.z = r.z0 - r.speed * run;
  return out;
}

function distanceToLead(runners: readonly Runner[], t: number, player: number, a: Vec3, b: Vec3): number {
  pathOf(runners, t, 0, a);
  pathOf(runners, t, player, b);
  return Math.hypot(b.x - a.x, b.y - a.y, b.z - a.z);
}

/** Throws when more than VOICE_LIMITS[category] ui or sting sounds would overlap. */
function checkVoiceBudget(events: readonly RaceEvent[]): void {
  const stretch = 1 / (1 - MICRO_VARIATION.rate);
  const spans: { category: 'ui' | 'sting'; from: number; to: number }[] = [];
  for (const e of events) {
    if (e.kind !== 'play' || e.family === undefined) continue;
    const lim = LIMITED[e.family];
    if (lim === undefined) continue;
    spans.push({ category: lim.category, from: e.t, to: e.t + lim.seconds * stretch });
  }
  for (const s of spans) {
    const live = spans.filter((o) => o.category === s.category && o.from <= s.from && o.to > s.from).length;
    if (live > VOICE_LIMITS[s.category]) {
      throw new Error(`[fiesta-race] ${live} ${s.category} sounds overlap at ${s.from} s (limit ${VOICE_LIMITS[s.category]})`);
    }
  }
}

function buildEvents(runners: Runner[], seed: number): RaceEvent[] {
  const events: RaceEvent[] = [];
  const a: Vec3 = { x: 0, y: 0, z: 0 };
  const b: Vec3 = { x: 0, y: 0, z: 0 };
  const near = (t: number, p: number): boolean => p === 0 || distanceToLead(runners, t, p, a, b) <= NEAR_M;

  // Countdown, go and the start cheer.
  for (const t of [0, 1, 2]) events.push({ t, kind: 'play', family: 'cue.countdown' });
  events.push({ t: GO_T, kind: 'play', family: 'cue.go' });
  events.push({ t: GO_T, kind: 'play', family: 'crowd.cheer.small' });
  events.push({ t: GO_T, kind: 'duck', duck: 'cheer', strength: 1 });

  // Music: upbeat from 0, intensity steps, climax.
  for (const [t, v] of INTENSITY_STEPS) {
    if (t === 0) events.push({ t, kind: 'music', style: 'upbeat', param: 'intensity', value: v });
    else events.push({ t, kind: 'music', param: 'intensity', value: v });
  }
  events.push({ t: CLIMAX_T, kind: 'music', param: 'climax', value: 1 });

  // Crowd bed excitement ramp.
  for (let t = 0; t <= EXCITE_END_T; t += EXCITE_STEP_S) {
    const v = EXCITE_FROM + (EXCITE_TO - EXCITE_FROM) * (t / EXCITE_END_T);
    events.push({ t, kind: 'bed', param: 'excitement', value: Math.round(v * 1000) / 1000 });
  }

  // Eliminations on distinct players 1..7, nearby ones preferred; they stop where they fall.
  const elimRng = new AudioRng(hashSeed(seed, 'eliminations'));
  const out = new Set<number>();
  for (const t of ELIMINATION_T) {
    const free: number[] = [];
    for (let p = 1; p < PLAYERS; p++) if (!out.has(p)) free.push(p);
    const close = free.filter((p) => near(t, p));
    const pool = close.length > 0 ? close : free;
    const p = pool[elimRng.int(0, pool.length - 1)];
    out.add(p);
    runners[p].stopT = t;
    events.push({ t, kind: 'play', family: 'fiesta.eliminate', player: p });
    events.push({ t, kind: 'duck', duck: 'eliminate', strength: 1 });
    events.push({ t, kind: 'play', family: 'crowd.aww' });
    events.push({ t, kind: 'bed', param: 'swell', value: 1 });
    events.push({ t: t + SWELL_RELEASE_S, kind: 'bed', param: 'swell', value: 0 });
  }

  // Mid-race cheer.
  const cheerRng = new AudioRng(hashSeed(seed, 'cheer'));
  events.push({ t: ms(cheerRng.range(MID_CHEER_FROM, MID_CHEER_TO)), kind: 'play', family: 'crowd.cheer.small' });

  // Per-player jumps, dives and grabs while near player 0 and still running.
  for (let p = 0; p < PLAYERS; p++) {
    const rng = new AudioRng(hashSeed(seed, 'actions', p));
    let t = GO_T + rng.range(0.5, ACTION_MAX_S);
    while (t < FINISH_T && t < runners[p].stopT) {
      const family = ACTIONS[rng.int(0, ACTIONS.length - 1)];
      if (near(t, p)) events.push({ t: ms(t), kind: 'play', family, player: p });
      t += rng.range(ACTION_MIN_S, ACTION_MAX_S);
    }
  }

  // Obstacle hits on a nearby running player.
  const hitRng = new AudioRng(hashSeed(seed, 'hits'));
  for (let t = GO_T + hitRng.range(HIT_MIN_S, HIT_MAX_S); t < FINISH_T; t += hitRng.range(HIT_MIN_S, HIT_MAX_S)) {
    const pool: number[] = [];
    for (let p = 0; p < PLAYERS; p++) if (t < runners[p].stopT && near(t, p)) pool.push(p);
    const p = pool[hitRng.int(0, pool.length - 1)];
    const family = HITS[hitRng.int(0, HITS.length - 1)];
    events.push({ t: ms(t), kind: 'play', family, player: p });
  }

  // Player 0 qualifies.
  events.push({ t: FINISH_T, kind: 'play', family: 'sting.win' });
  events.push({ t: FINISH_T, kind: 'play', family: 'crowd.cheer.big' });
  events.push({ t: FINISH_T, kind: 'duck', duck: 'qualify', strength: 1 });
  events.push({ t: FINISH_T, kind: 'bed', param: 'swell', value: 1 });
  events.push({ t: FINISH_T + SWELL_RELEASE_S, kind: 'bed', param: 'swell', value: 0 });

  events.sort((x, y) => x.t - y.t);
  checkVoiceBudget(events);
  return events;
}

const RUNNERS = makeRunners(new AudioRng(hashSeed(SEED, 'runners')));
const EVENTS: readonly RaceEvent[] = Object.freeze(buildEvents(RUNNERS, SEED).map((e) => Object.freeze(e)));

/** The scripted race: its length, player count, seed, player paths and sorted events. */
export const FIESTA_RACE: {
  seconds: 60;
  players: 8;
  seed: number;
  paths: (t: number, player: number, out: Vec3) => Vec3;
  events: readonly RaceEvent[];
} = {
  seconds: SECONDS,
  players: PLAYERS,
  seed: SEED,
  paths: (t: number, player: number, out: Vec3): Vec3 => pathOf(RUNNERS, t, player, out),
  events: EVENTS
};
