/**
 * Host test for the geometric acoustics of @etcito/haa/sfx: image-source reflections, the room
 * estimate (Sabine from the mean free path) and occlusion, against analytic planes and boxes.
 *
 *   node --test packages/haa/test/sfx-acoustics.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import type { RayCast, RayHit, Vec3 } from '../src/sfx/acoustics.ts';
import { earlyReflections, estimateRoom, occlusion, occlusionLevels, OCCLUDED_GAIN, SPEED_OF_SOUND_M_S, v3, WET_MAX } from '../src/sfx/acoustics.ts';
import { diffractionLevels, directPath, traceListener, voiceReverb } from '../src/sfx/raytrace.ts';
import { bankWeights } from '../src/sfx/spatial3d.ts';

interface Plane {
  id: number;
  n: Vec3;
  d: number;
  absorption: number;
  /** Optional finite extent: |x| and |z| limits of the hit point. */
  within?: (p: Vec3) => boolean;
}

/** Ray cast over infinite (or clipped) planes n·p = d, nearest hit first. */
function planes(list: Plane[]): RayCast {
  return (o, dir, maxDist) => {
    let best: RayHit | null = null;
    for (const pl of list) {
      const denom = pl.n.x * dir.x + pl.n.y * dir.y + pl.n.z * dir.z;
      if (Math.abs(denom) < 1e-9) continue;
      const t = (pl.d - (pl.n.x * o.x + pl.n.y * o.y + pl.n.z * o.z)) / denom;
      if (t <= 1e-6 || t > maxDist) continue;
      const point = v3(o.x + dir.x * t, o.y + dir.y * t, o.z + dir.z * t);
      if (pl.within && !pl.within(point)) continue;
      if (!best || t < best.toi) best = { toi: t, point, normal: pl.n, absorption: pl.absorption, id: pl.id };
    }
    return best;
  };
}

const floor: Plane = { id: 1, n: v3(0, 1, 0), d: 0, absorption: 0.1 };

test('a floor gives one reflection with the image-source path and delay', () => {
  const source = v3(0, 1, 0);
  const listener = v3(4, 1, 0);
  const refl = earlyReflections(source, listener, planes([floor]));
  assert.equal(refl.length, 1);
  const r = refl[0];
  assert.deepEqual(r.image, v3(0, -1, 0));
  assert.ok(Math.abs(r.point.x - 2) < 1e-9 && Math.abs(r.point.y) < 1e-9);
  const path = Math.hypot(4, 2);
  assert.ok(Math.abs(r.path - path) < 1e-9);
  assert.ok(Math.abs(r.delay - (path - 4) / SPEED_OF_SOUND_M_S) < 1e-12);
  assert.ok(Math.abs(r.gain - Math.sqrt(0.9)) < 1e-12);
});

test('a corner adds the two walls, shortest paths first; a listener behind a wall gets no echo from it', () => {
  const wallX: Plane = { id: 2, n: v3(-1, 0, 0), d: -10, absorption: 0.02 }; // x = 10
  const wallZ: Plane = { id: 3, n: v3(0, 0, 1), d: -10, absorption: 0.02 }; // z = -10
  const cast = planes([floor, wallX, wallZ]);
  const refl = earlyReflections(v3(8, 1, -8), v3(6, 1.6, -6), cast);
  assert.deepEqual(new Set(refl.map((r) => r.id)), new Set([1, 2, 3]));
  for (let i = 1; i < refl.length; i++) assert.ok(refl[i].path >= refl[i - 1].path);
  // Source in front of the x = 10 wall, listener beyond it: that wall reflects nothing to it.
  const behind = earlyReflections(v3(8, 1, 0), v3(12, 1, 0), planes([wallX]));
  assert.equal(behind.length, 0);
});

test('room: open ground is dry, a corner faint, a closed box follows Sabine and is the wettest', () => {
  const open = estimateRoom(v3(0, 1.6, 0), planes([floor]));
  assert.ok(open.escape > 0.6, `escape ${open.escape}`);
  assert.equal(open.enclosure, 0);
  assert.equal(open.wet, 0);
  // Two concrete walls 3 m away and open sky: a diffuse field well under the direct sound.
  const corner = estimateRoom(v3(7, 1.6, -7), planes([
    floor,
    { id: 2, n: v3(-1, 0, 0), d: -10, absorption: 0.02, within: (p) => p.y < 4 },
    { id: 3, n: v3(0, 0, 1), d: -10, absorption: 0.02, within: (p) => p.y < 4 },
  ]));
  assert.ok(corner.wet > 0.05 && corner.wet < 0.3, JSON.stringify(corner));
  // A far wall (12 m) gives echoes, not a diffuse field.
  const far = estimateRoom(v3(0, 1.6, 0), planes([floor, { id: 4, n: v3(0, 0, 1), d: -12, absorption: 0.02 }]));
  assert.ok(far.wet < 0.02, JSON.stringify(far));

  const a = 0.05;
  const box = planes([
    { id: 1, n: v3(0, 1, 0), d: 0, absorption: a },
    { id: 2, n: v3(0, -1, 0), d: -4, absorption: a },
    { id: 3, n: v3(1, 0, 0), d: -3, absorption: a },
    { id: 4, n: v3(-1, 0, 0), d: -3, absorption: a },
    { id: 5, n: v3(0, 0, 1), d: -3, absorption: a },
    { id: 6, n: v3(0, 0, -1), d: -3, absorption: a },
  ]);
  const room = estimateRoom(v3(0, 2, 0), box);
  assert.equal(room.escape, 0);
  assert.ok(Math.abs(room.rt60 - (0.161 * room.meanFreePath) / (4 * a)) < 1e-9 || room.rt60 === 1.8);
  assert.equal(room.enclosure, 1);
  assert.equal(room.wet, WET_MAX);
});

test('occlusion: a wall between source and listener blocks all probes; the source surface itself never does', () => {
  const wall: Plane = { id: 9, n: v3(0, 0, 1), d: 5, absorption: 0.02, within: (p) => Math.abs(p.x) < 4 && p.y < 3 };
  const cast = planes([floor, wall]);
  const blocked = occlusion(v3(0, 1, 2), v3(0, 1.6, 9), cast);
  assert.equal(blocked.blocked, 1);
  assert.ok(Math.abs(blocked.gain - OCCLUDED_GAIN) < 1e-12);
  // Fully blocked is -7 dB broadband and -16 dB in the highs (two shelves) by default: a gentle
  // slope, not a low-pass; strength 0 turns occlusion off, 2 doubles every dB figure.
  assert.ok(Math.abs(20 * Math.log10(blocked.gain) + 7) < 0.05);
  assert.deepEqual(blocked.shelves, [-4, -5]);
  assert.deepEqual(occlusionLevels(1, 0), { blocked: 1, gain: 1, shelves: [0, 0] });
  const twice = occlusionLevels(1, 2);
  assert.ok(Math.abs(20 * Math.log10(twice.gain) + 14) < 0.1);
  assert.deepEqual(twice.shelves, [-8, -10]);
  // A source lying on the floor, heard from above: the floor under it is skipped.
  const onFloor = occlusion(v3(0, 0, 0), v3(3, 2, 0), planes([floor]));
  assert.equal(onFloor.blocked, 0);
});

// --- ray-traced acoustics (raytrace.ts) ------------------------------------------------------

/** A concrete hall x -6..6, z -4..4, 5 m high, a door 2 m wide and 2.6 m high in its x = 6 wall; open ground outside. */
function hall(): RayCast {
  const a = 0.02;
  const inX = (p: Vec3): boolean => Math.abs(p.x) <= 6 && p.y <= 5;
  const inZ = (p: Vec3): boolean => Math.abs(p.z) <= 4 && p.y <= 5;
  return planes([
    { id: 1, n: v3(0, 1, 0), d: 0, absorption: a },
    { id: 2, n: v3(0, -1, 0), d: -5, absorption: a, within: (p) => Math.abs(p.x) <= 6 && Math.abs(p.z) <= 4 },
    { id: 3, n: v3(0, 0, 1), d: -4, absorption: a, within: inX },
    { id: 4, n: v3(0, 0, -1), d: -4, absorption: a, within: inX },
    { id: 5, n: v3(1, 0, 0), d: -6, absorption: a, within: inZ },
    { id: 6, n: v3(-1, 0, 0), d: -6, absorption: a, within: (p) => inZ(p) && !(Math.abs(p.z) < 1 && p.y < 2.6) },
  ]);
}

test('diffuse field: the hall is wettest at its centre, less in front of the door, less in the doorway, faint outside; it comes from the hall', () => {
  const cast = hall();
  const at = (x: number) => {
    const lis = v3(x, 1.6, 0.3);
    const r = voiceReverb(traceListener(lis, cast, { rays: 256, bounces: 6 }), v3(x - 1.5, 0.05, 0), cast);
    return { db: 10 * Math.log10(r.energy), rt: r.rt60, dirX: r.direction.x };
  };
  const centre = at(0);
  const nearDoor = at(5);
  const doorway = at(6.2);
  const outside = at(10);
  assert.ok(centre.db > nearDoor.db + 1.5, JSON.stringify({ centre, nearDoor }));
  assert.ok(nearDoor.db > doorway.db + 2, JSON.stringify({ nearDoor, doorway }));
  assert.ok(doorway.db > outside.db + 6, JSON.stringify({ doorway, outside }));
  // A concrete hall rings for seconds; heard from the doorway its field arrives from inside (-x).
  assert.ok(centre.rt > 1.5, JSON.stringify(centre));
  assert.ok(doorway.dirX < -0.1, JSON.stringify(doorway));
});

test('direct path: a source sliding behind a wall edge loses level gradually, and arrives around the edge', () => {
  // A wall z = 0, x <= 0, 3 m high; listener at z = 4; the source moves along z = -2 from x = 4 to x = -4.
  const wall: Plane = { id: 9, n: v3(0, 0, 1), d: 0, absorption: 0.02, within: (p) => p.x <= 0 && p.y <= 3 };
  const cast = planes([floor, wall]);
  const lis = v3(0.5, 1.6, 4);
  const trace = traceListener(lis, cast, { rays: 256, bounces: 2 });
  let prev: number | null = null;
  let maxStep = 0;
  let last = 0;
  for (let x = 4; x >= -4; x -= 0.25) {
    const path = directPath(v3(x, 1, -2), lis, trace, cast);
    const db = diffractionLevels(path).bandsDb[1];
    if (prev !== null) maxStep = Math.max(maxStep, Math.abs(db - prev));
    prev = db;
    last = db;
    if (x <= -3) {
      // Hidden: it arrives around the edge or over the top, never through the wall.
      assert.ok(path.visible === 0 && path.found && path.excess > 0.2, JSON.stringify(path));
      assert.ok(path.via.y > 2.9 || path.via.x > -0.1, JSON.stringify(path.via));
    }
  }
  assert.ok(last < -6, `hidden loss ${last}`);
  assert.ok(maxStep < 3, `largest step between positions 0.25 m apart: ${maxStep} dB`);
});

test('a footstep on the floor gets no floor echo merged with it; reverb weights glide and keep their power', () => {
  const refl = earlyReflections(v3(0, 0.03, 0), v3(0.3, 1.6, 0.4), planes([floor]), { minDelay: 0.0015 });
  assert.equal(refl.length, 0);
  let prev = bankWeights(0.2);
  for (let rt = 0.2; rt <= 2.5; rt *= 1.02) {
    const w = bankWeights(rt);
    assert.ok(Math.abs(w.reduce((a, x) => a + x * x, 0) - 1) < 1e-9);
    const step = Math.max(...w.map((x, i) => Math.abs(x - prev[i])));
    assert.ok(step < 0.15, `weights jump ${step} at ${rt}`);
    prev = w;
  }
});
