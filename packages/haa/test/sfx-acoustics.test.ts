/**
 * Host test for the geometric acoustics of @etcito/haa/sfx: image-source reflections, the room
 * estimate (Sabine from the mean free path) and occlusion, against analytic planes and boxes.
 *
 *   node --test packages/haa/test/sfx-acoustics.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import type { RayCast, RayHit, Vec3 } from '../src/sfx/acoustics.ts';
import { earlyReflections, estimateRoom, occlusion, OCCLUDED_GAIN, SPEED_OF_SOUND_M_S, v3 } from '../src/sfx/acoustics.ts';

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

test('room: open ground is short and dry, a closed box follows Sabine from the mean free path', () => {
  const open = estimateRoom(v3(0, 1.6, 0), planes([floor]));
  assert.ok(open.escape > 0.6, `escape ${open.escape}`);
  assert.ok(open.wet < 0.15 && open.rt60 <= 0.2, JSON.stringify(open));

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
  assert.ok(room.wet > 0.9);
});

test('occlusion: a wall between source and listener blocks all probes; the source surface itself never does', () => {
  const wall: Plane = { id: 9, n: v3(0, 0, 1), d: 5, absorption: 0.02, within: (p) => Math.abs(p.x) < 4 && p.y < 3 };
  const cast = planes([floor, wall]);
  const blocked = occlusion(v3(0, 1, 2), v3(0, 1.6, 9), cast);
  assert.equal(blocked.blocked, 1);
  assert.ok(Math.abs(blocked.gain - OCCLUDED_GAIN) < 1e-12 && blocked.cutoff < 1000);
  // A source lying on the floor, heard from above: the floor under it is skipped.
  const onFloor = occlusion(v3(0, 0, 0), v3(3, 2, 0), planes([floor]));
  assert.equal(onFloor.blocked, 0);
});
