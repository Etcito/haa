// Royal Fiesta race renderer (module fiesta-race, node): drives FIESTA_RACE through the offline
// scene at a 60 Hz control rate and writes the 60 s soundtrack plus its event log.
//
// Usage: node --conditions=haa-source examples/royal-fiesta-pack/scripts/fiesta-race.ts [--listeners 1|2] [--out <dir>] (default examples/royal-fiesta-pack/out/m6)
//   writes <out>/fiesta-race.wav (fiesta-race-2l.wav with 2 listeners) and
//   <out>/fiesta-race.events.json (2-space indentation, fixed key order, no timestamps).
//
// Listener rig: listener 0 follows player 0 at +2 m Z (behind) and +1.5 m Y with the identity
// quat (facing -Z); with 2 listeners the second follows player 1 the same way. Listener and
// source velocities are finite differences of the scripted paths over one control step.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createOfflineAudio, createSpatial, getFamily, getStyle, registerBuiltins, type Listener, type MusicParam, type OfflineAudio, type StereoRender, type Vec3 } from '@etcito/haa';
import { writeWav } from '@etcito/haa/node';
import { CROWD_PARAMS, FIESTA_PRESET, registerRoyalFiesta } from '../src/index.ts';
import { FIESTA_RACE } from './raceScript.ts';
import type { RaceEvent } from './raceScript.ts';

/** Control rate (Hz) and the listener offset from its player (m). */
const CONTROL_HZ = 60;
const LISTENER_BACK_M = 2;
const LISTENER_UP_M = 1.5;
/** Music params settable through music.set. */
const MUSIC_PARAMS: readonly MusicParam[] = ['intensity', 'phase', 'climax', 'resolved', 'won', 'sparkle', 'tension', 'level'];
const IDENTITY = { x: 0, y: 0, z: 0, w: 1 } as const;

/** The result of one race render. */
export interface FiestaRaceResult {
  render: StereoRender;
  stats: OfflineAudio['stats'];
  scheduled: number;
}

const fail = (reason: string): never => { throw new Error(`[fiesta-race] ${reason}`); };

/** Throws `[fiesta-race] <reason>` on the first event that cannot resolve. */
function validate(events: readonly RaceEvent[]): void {
  registerBuiltins();
  registerRoyalFiesta();
  const bedParams: readonly string[] = CROWD_PARAMS;
  for (const e of events) {
    const at = `at ${e.t} s`;
    if (!Number.isFinite(e.t) || e.t < 0) fail(`bad event time ${e.t}`);
    switch (e.kind) {
      case 'play':
        if (e.family === undefined || !getFamily(e.family)) fail(`unregistered family ${e.family} ${at}`);
        if (e.player !== undefined && !(Number.isInteger(e.player) && e.player >= 0 && e.player < FIESTA_RACE.players)) fail(`bad player ${e.player} ${at}`);
        break;
      case 'duck':
        if (e.duck === undefined || !Object.hasOwn(FIESTA_PRESET.ducks, e.duck)) fail(`unknown duck kind ${e.duck} ${at}`);
        break;
      case 'bed':
        if (e.param === undefined || !bedParams.includes(e.param)) fail(`unknown bed param ${e.param} ${at}`);
        if (!Number.isFinite(e.value)) fail(`bad bed value ${e.value} ${at}`);
        break;
      case 'music':
        if (e.style !== undefined) {
          if (!getStyle(e.style)) fail(`unknown style ${e.style} ${at}`);
        } else if (e.param === undefined || !(MUSIC_PARAMS as readonly string[]).includes(e.param)) {
          fail(`unknown music param ${e.param} ${at}`);
        }
        break;
      default:
        fail(`unknown event kind ${String((e as { kind: unknown }).kind)} ${at}`);
    }
  }
}

/** Renders the scripted race for 1 or 2 listeners. */
export function renderFiestaRace(listeners: 1 | 2): FiestaRaceResult {
  const race = FIESTA_RACE;
  validate(race.events);
  const audio = createOfflineAudio({ mix: 'fiesta', spatial: createSpatial(), maxListeners: listeners });
  const crowd = audio.startBed('crowd') ?? fail('crowd bed did not start');

  const dt = 1 / CONTROL_HZ;
  const steps = Math.round(race.seconds * CONTROL_HZ);
  const cur: Vec3 = { x: 0, y: 0, z: 0 };
  const prev: Vec3 = { x: 0, y: 0, z: 0 };
  const rig: Listener[] = Array.from({ length: listeners }, () => ({
    pos: { x: 0, y: 0, z: 0 }, quat: { ...IDENTITY }, vel: { x: 0, y: 0, z: 0 },
  }));
  /** Position and finite-difference velocity of a player at t. */
  const sample = (t: number, player: number, pos: Vec3, vel: Vec3): void => {
    race.paths(t, player, cur);
    race.paths(Math.max(0, t - dt), player, prev);
    pos.x = cur.x; pos.y = cur.y; pos.z = cur.z;
    const k = t > 0 ? 1 / Math.min(dt, t) : 0;
    vel.x = (cur.x - prev.x) * k; vel.y = (cur.y - prev.y) * k; vel.z = (cur.z - prev.z) * k;
  };

  let next = 0;
  for (let step = 0; step < steps; step++) {
    const t = step / CONTROL_HZ;
    for (let l = 0; l < listeners; l++) {
      const ls = rig[l];
      const vel = ls.vel as Vec3;
      sample(t, l, ls.pos, vel);
      ls.pos.y += LISTENER_UP_M;
      ls.pos.z += LISTENER_BACK_M;
    }
    audio.setListeners(rig);
    while (next < race.events.length && race.events[next].t <= t + 1e-9) {
      const e = race.events[next++];
      switch (e.kind) {
        case 'play':
          if (e.player === undefined) {
            audio.play(e.family as string, { gain: e.gain });
          } else {
            const pos = { x: 0, y: 0, z: 0 };
            const vel = { x: 0, y: 0, z: 0 };
            sample(t, e.player, pos, vel);
            audio.play(e.family as string, { pos, vel, gain: e.gain });
          }
          break;
        case 'duck': audio.duck(e.duck as string, e.strength); break;
        case 'bed': crowd.setParam(e.param as string, e.value as number); break;
        case 'music':
          if (e.style !== undefined) audio.music.play(e.style, { intensity: e.value });
          else audio.music.set(e.param as MusicParam, e.value as number);
          break;
      }
    }
    audio.advance(dt);
  }
  const s = audio.stats;
  return {
    render: audio.take('fiesta-race'),
    stats: { played: s.played, dropped: s.dropped, stolen: s.stolen },
    scheduled: race.events.filter((e) => e.kind === 'play').length,
  };
}

/** One event in the log's fixed key order, without absent keys. */
function logEvent(e: RaceEvent): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const key of ['t', 'kind', 'family', 'player', 'gain', 'duck', 'strength', 'param', 'value', 'style'] as const) {
    if (e[key] !== undefined) out[key] = e[key];
  }
  return out;
}

function parseArgs(argv: readonly string[]): { listeners: 1 | 2; out: string } {
  let listeners: 1 | 2 = 1;
  let out = fileURLToPath(new URL('../out/m6', import.meta.url));
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const value = argv[i + 1];
    if (arg === '--listeners') {
      if (value !== '1' && value !== '2') fail(`--listeners must be 1 or 2, got ${value}`);
      listeners = value === '2' ? 2 : 1;
      i++;
    } else if (arg === '--out') {
      if (value === undefined || value.startsWith('--')) fail('--out needs a directory');
      out = value;
      i++;
    } else {
      fail(`unknown argument ${arg}`);
    }
  }
  return { listeners, out };
}

function main(): void {
  const { listeners, out } = parseArgs(process.argv.slice(2));
  const { render, stats, scheduled } = renderFiestaRace(listeners);
  const wav = join(out, listeners === 2 ? 'fiesta-race-2l.wav' : 'fiesta-race.wav');
  const log = join(out, 'fiesta-race.events.json');
  writeWav(wav, render);
  mkdirSync(out, { recursive: true });
  const doc = {
    name: 'fiesta-race',
    listeners,
    seconds: FIESTA_RACE.seconds,
    players: FIESTA_RACE.players,
    seed: FIESTA_RACE.seed,
    controlHz: CONTROL_HZ,
    scheduled,
    stats: { played: stats.played, dropped: stats.dropped, stolen: stats.stolen },
    events: FIESTA_RACE.events.map(logEvent),
  };
  writeFileSync(log, `${JSON.stringify(doc, null, 2)}\n`);
  console.log(`fiesta-race listeners ${listeners}: played ${stats.played} dropped ${stats.dropped} stolen ${stats.stolen} scheduled ${scheduled} wav ${wav} events ${log}`);
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(msg.startsWith('[fiesta-race] ') ? msg : `[fiesta-race] ${msg}`);
    process.exit(1);
  }
}
