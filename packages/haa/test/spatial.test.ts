/**
 * Host test for the spatial scene: falloff curve, pan sides, doppler clamp and the
 * reserved inputs, with fake Web Audio nodes and a fake player.
 *
 * Runs with Node's native type stripping:
 *   node --test packages/haa/test/spatial.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import type { HaaPlayer } from '../src/player.ts';
import type { HaaInput } from '../src/types.ts';
import { DEFAULT_FALLOFF, createSpatialScene, dopplerRatio, emitterGain, equalPowerPan } from '../src/spatial.ts';

/** A fake AudioNode that records its connections. */
class FakeNode {
  readonly outs = new Set<FakeNode>();
  connect(dest: FakeNode): FakeNode {
    this.outs.add(dest);
    return dest;
  }
  disconnect(dest?: FakeNode): void {
    if (dest) this.outs.delete(dest);
    else this.outs.clear();
  }
}
class FakeGain extends FakeNode {
  readonly gain = { value: 1 };
}
class FakePanner extends FakeNode {
  readonly pan = { value: 0 };
}

function fakeContext() {
  const ctx = {
    destination: new FakeNode(),
    gains: [] as FakeGain[],
    panners: [] as FakePanner[],
    createGain() {
      const g = new FakeGain();
      ctx.gains.push(g);
      return g;
    },
    createStereoPanner() {
      const p = new FakePanner();
      ctx.panners.push(p);
      return p;
    },
  };
  return ctx;
}

function input(name: string): HaaInput {
  return { name, kind: 'continuous', min: -100, max: 100, default: 0 };
}

function fakePlayer(names: string[]) {
  const sets: [string, number | string][] = [];
  const node = new FakeNode();
  const player = {
    node,
    info: { nodes: [], inputs: names.map(input), mode: 'live', duration: 1, loop: true },
    set(name: string, value: number | string) {
      sets.push([name, value]);
    },
  };
  return { player: player as unknown as HaaPlayer, node, sets };
}

function scene() {
  const ctx = fakeContext();
  const s = createSpatialScene(ctx as unknown as BaseAudioContext);
  s.setListener({ x: 0, y: 0, z: 0, fx: 0, fz: 1 });
  return { ctx, s };
}

test('gain is 1 at 1 m, 0.5 at 4 m and 0 at 61 m', () => {
  assert.equal(emitterGain(1, DEFAULT_FALLOFF), 1);
  assert.equal(emitterGain(4, DEFAULT_FALLOFF), 0.5);
  assert.equal(emitterGain(61, DEFAULT_FALLOFF), 0);

  const { ctx, s } = scene();
  const { player } = fakePlayer([]);
  const e = s.addEmitter('a', player);
  e.setGain(0.8);
  e.setPosition(0, 0, 4);
  s.update();
  assert.equal(ctx.gains[1].gain.value, 0.4);
  e.setPosition(0, 0, 61);
  s.update();
  assert.equal(ctx.gains[1].gain.value, 0);
});

test('an emitter to the right pans > 0, to the left < 0', () => {
  const { ctx, s } = scene();
  const e = s.addEmitter('a', fakePlayer([]).player);
  e.setPosition(3, 0, 1);
  s.update();
  assert.ok(ctx.panners[0].pan.value > 0);
  e.setPosition(-3, 0, 1);
  s.update();
  assert.ok(ctx.panners[0].pan.value < 0);
  s.setListener({ x: 0, y: 0, z: 0, fx: 0, fz: -1 });
  s.update();
  assert.ok(ctx.panners[0].pan.value > 0);

  const centre = equalPowerPan(0);
  assert.ok(Math.abs(centre.left - Math.SQRT1_2) < 1e-12);
  assert.ok(Math.abs(centre.right - Math.SQRT1_2) < 1e-12);
  assert.ok(Math.abs(equalPowerPan(1).right - 1) < 1e-12);
  assert.ok(Math.abs(equalPowerPan(5).left) < 1e-12);
});

test('doppler is c/(c - v), clamped at 2 and 0.5', () => {
  assert.equal(dopplerRatio(0), 1);
  assert.equal(dopplerRatio(300), 2);
  assert.equal(dopplerRatio(1000), 2);
  assert.equal(dopplerRatio(-1000), 0.5);
  assert.ok(Math.abs(dopplerRatio(34.3) - 343 / 308.7) < 1e-12);
});

test('reserved inputs are set only when declared', () => {
  const { s } = scene();
  const declared = fakePlayer(['_pan', '_doppler']);
  const bare = fakePlayer(['rpm']);
  const a = s.addEmitter('a', declared.player);
  const b = s.addEmitter('b', bare.player);
  for (const e of [a, b]) {
    e.setPosition(10, 0, 0);
    e.setVelocity(-1000, 0, 0);
  }
  s.update();
  const names = new Set(declared.sets.map(([name]) => name));
  assert.deepEqual([...names].sort(), ['_doppler', '_pan']);
  assert.deepEqual(declared.sets.at(-1), ['_doppler', 2]);
  assert.equal(bare.sets.length, 0);
});

test('addEmitter reroutes the player and dispose disconnects everything', () => {
  const { ctx, s } = scene();
  const { player, node } = fakePlayer([]);
  node.connect(ctx.destination);
  const out = ctx.gains[0];
  assert.ok(out.outs.has(ctx.destination));
  s.addEmitter('a', player);
  const [gain, panner] = [ctx.gains[1], ctx.panners[0]];
  assert.deepEqual([...node.outs], [gain]);
  assert.ok(gain.outs.has(panner) && panner.outs.has(out));
  s.dispose();
  assert.equal(node.outs.size + gain.outs.size + panner.outs.size + out.outs.size, 0);
});
