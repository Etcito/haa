/**
 * Host test for the face emitters of @etcito/haa/sfx (rain on the level's faces): exposure, flux
 * under a tilted rain and the emitter heard from under a roof.
 *
 *   node --test packages/haa/test/sfx-faces.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import type { RayCast, RayHit, Vec3 } from '../src/sfx/acoustics.ts';
import { v3 } from '../src/sfx/acoustics.ts';
import { defineMaterial, withBody } from '../src/sfx/contract.ts';
import type { FaceHost, SoundFace } from '../src/sfx/faces.ts';
import { createFaceSource } from '../src/sfx/faces.ts';

/** A roof slab y 3..3.2 over x, z in -2..2, and the ground y = 0. */
const cast: RayCast = (o, d, max) => {
  let best: RayHit | null = null;
  const tryPlane = (y: number, within: (p: Vec3) => boolean, id: number): void => {
    if (Math.abs(d.y) < 1e-9) return;
    const t = (y - o.y) / d.y;
    if (t <= 1e-6 || t > max) return;
    const p = v3(o.x + d.x * t, o.y + d.y * t, o.z + d.z * t);
    if (within(p) && (!best || t < best.toi)) best = { toi: t, point: p, normal: v3(0, Math.sign(-d.y), 0), absorption: 0.05, id };
  };
  const under = (p: Vec3): boolean => Math.abs(p.x) <= 2 && Math.abs(p.z) <= 2;
  tryPlane(3, under, 1);
  tryPlane(3.2, under, 1);
  tryPlane(0, () => true, 2);
  return best;
};

test('rain on faces: the roof shields the ground under it, a wall gets rain only when it tilts, and under the roof the roof plays through itself', () => {
  const tile = withBody(defineMaterial({ name: 'tile', hardness: 0.8 }), { shape: 'plate', dim: 0.3, thickness: 0.012 });
  const ground = withBody(defineMaterial({ name: 'concrete', hardness: 0.9 }), { shape: 'block', dim: 20, thickness: 0.5 });
  const faces: SoundFace[] = [
    { label: 'roof', center: v3(0, 3.2, 0), normal: v3(0, 1, 0), u: v3(2, 0, 0), v: v3(0, 0, 2), material: tile, thickness: 0.2 },
    { label: 'ground', center: v3(0, 0, 0), normal: v3(0, 1, 0), u: v3(10, 0, 0), v: v3(0, 0, 10), material: ground },
    { label: 'wall', center: v3(-2, 1.5, 0), normal: v3(-1, 0, 0), u: v3(0, 1.5, 0), v: v3(0, 0, 2), material: ground, thickness: 0.2 },
  ];
  let listener = v3(0, 1.6, 0);
  const played: Array<{ inputs: Record<string, number>; tx: readonly number[] }> = [];
  const host: FaceHost = {
    raycast: () => cast,
    listener: () => listener,
    loop: (_preset, _p, inputs) => {
      const rec = { inputs: { ...inputs } as Record<string, number>, tx: [0, 0, 0] as readonly number[] };
      played.push(rec);
      return { alive: true, set: (i) => Object.assign(rec.inputs, i), move: () => {}, setGain: () => {}, setTransmission: (db) => { rec.tx = db; }, stop: () => {} };
    },
  };
  const src = createFaceSource(host, faces, { preset: 'rain-on-face', direction: v3(0, -1, 0), intensity: 0.6, cell: 1 });
  src.tick(0.1);
  const byLabel = (l: string) => src.emitters.find((e) => e.face.label === l)!;
  // 20 x 20 m of ground minus the 4 x 4 m under the roof.
  assert.ok(Math.abs(byLabel('ground').area - 384) < 1, `ground ${byLabel('ground').area}`);
  assert.equal(byLabel('roof').area, 16);
  assert.equal(byLabel('wall').flux, 0);
  // Under the roof: the roof is heard from its underside, through the tiles' far-side loss.
  const roof = byLabel('roof');
  assert.ok(roof.under === 1 && roof.position.y < 3, JSON.stringify(roof.position));
  const roofVoice = played.find((p) => p.inputs.shape === 1 / 6);
  assert.ok(roofVoice && roofVoice.tx[1] > 10, JSON.stringify(roofVoice));
  // Rain driven toward +x at 40°: the wall facing -x receives sin(40°) of it.
  src.set({ direction: v3(Math.sin((40 * Math.PI) / 180), -Math.cos((40 * Math.PI) / 180), 0) });
  listener = v3(-6, 1.6, 0);
  src.tick(0.1);
  assert.ok(Math.abs(byLabel('wall').flux - Math.sin((40 * Math.PI) / 180)) < 1e-9, `wall ${byLabel('wall').flux}`);
  assert.ok(byLabel('wall').area > 0 && byLabel('roof').under === 0);
});

test('rain on faces: on wet ground the ground is heard right under the listener, whatever the cell size', () => {
  const ground = withBody(defineMaterial({ name: 'concrete', hardness: 0.9 }), { shape: 'block', dim: 20, thickness: 0.5 });
  const faces: SoundFace[] = [{ label: 'ground', center: v3(0, 0, 0), normal: v3(0, 1, 0), u: v3(10, 0, 0), v: v3(0, 0, 10), material: ground }];
  const listener = v3(2.5, 1.6, 0.7);
  const host: FaceHost = {
    raycast: () => cast,
    listener: () => listener,
    loop: () => ({ alive: true, set: () => {}, move: () => {}, setGain: () => {}, setTransmission: () => {}, stop: () => {} }),
  };
  // 4 m cells: the cell centred under the roof (x, z in -2..2) is dry, but the listener stands just beside it.
  const src = createFaceSource(host, faces, { preset: 'rain-on-face', direction: v3(0, -1, 0), intensity: 0.6, cell: 4 });
  src.tick(0.1);
  const p = src.emitters[0].position;
  assert.ok(Math.abs(p.x - 2.5) < 1e-6 && Math.abs(p.z - 0.7) < 1e-6 && p.y < 0.1, JSON.stringify(p));
});

test('rain on faces: a face hidden behind a wall is ranked by what reaches the listener, so the roof above wins', () => {
  // The roof slab of the first test, plus a wall x = 3 (y 0..5): the metal plate beyond it is hidden.
  const walled: RayCast = (o, d, max) => {
    const hit = cast(o, d, max);
    if (Math.abs(d.x) > 1e-9) {
      const t = (3 - o.x) / d.x;
      const p = v3(o.x + d.x * t, o.y + d.y * t, o.z + d.z * t);
      if (t > 1e-6 && t <= max && p.y >= 0 && p.y <= 5 && (!hit || t < hit.toi)) return { toi: t, point: p, normal: v3(-Math.sign(d.x), 0, 0), absorption: 0.05, id: 3 };
    }
    return hit;
  };
  const wood = withBody(defineMaterial({ name: 'wood', hardness: 0.55 }), { shape: 'plate', dim: 0.2, thickness: 0.022 });
  const metal = withBody(defineMaterial({ name: 'metal', hardness: 1 }), { shape: 'sheet', dim: 4, thickness: 0.001 });
  const faces: SoundFace[] = [
    { label: 'roof', center: v3(0, 3.2, 0), normal: v3(0, 1, 0), u: v3(2, 0, 0), v: v3(0, 0, 2), material: wood, thickness: 0.2 },
    { label: 'plate', center: v3(6, 0.05, 0), normal: v3(0, 1, 0), u: v3(2, 0, 0), v: v3(0, 0, 2), material: metal },
  ];
  const host: FaceHost = {
    raycast: () => walled,
    listener: () => v3(0, 1.6, 0),
    loop: () => ({ alive: true, set: () => {}, move: () => {}, setGain: () => {}, setTransmission: () => {}, stop: () => {} }),
  };
  const src = createFaceSource(host, faces, { preset: 'rain-on-face', direction: v3(0, -1, 0), intensity: 0.6, maxVoices: 1 });
  src.tick(0.1);
  const by = (l: string) => src.emitters.find((e) => e.face.label === l)!;
  assert.ok(by('roof').playing && !by('plate').playing, `roof ${by('roof').score} plate ${by('plate').score}`);
});
