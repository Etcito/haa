/**
 * Host test for the interaction contract of @etcito/haa/sfx (src/sfx/contract.ts): which presets an
 * interaction plays and at what level.
 *
 *   node --test packages/haa/test/sfx-contract.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import { defineMaterial, resolveInteraction, sizeFromMass } from '../src/sfx/contract.ts';

test('the floor of an impact rings by the striker mass: full under a barrel, low under a grenade, out under a shard', () => {
  const concrete = defineMaterial({ name: 'concrete', hardness: 0.9, sounds: { impact: 'solid-impact' } });
  const metal = defineMaterial({ name: 'metal', hardness: 0.8, sounds: { impact: 'grenade-land' } });
  const floorGain = (kg: number) => {
    const r = resolveInteraction('impact', metal, concrete, { energy: 0.6, size: sizeFromMass(kg) });
    assert.equal(r[0].slot.preset, 'grenade-land', 'the striker always rings');
    assert.ok(Math.abs(r[0].gain - Math.SQRT1_2) < 1e-9 || r.length === 1);
    return r.find((s) => s.slot.preset === 'solid-impact')?.gain ?? 0;
  };
  assert.ok(Math.abs(floorGain(100) - Math.SQRT1_2) < 1e-9, 'a 100 kg striker: the floor at its full share');
  const grenade = 20 * Math.log10(floorGain(0.4) / Math.SQRT1_2);
  assert.ok(Math.abs(grenade - -24) < 0.1, `a 0.4 kg grenade: the floor ${grenade.toFixed(1)} dB under its full share`);
  assert.equal(floorGain(0.005), 0, 'a 5 g shard: the floor is left out');
});
