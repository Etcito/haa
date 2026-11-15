/**
 * Host test for the Rapier adapter of @etcito/haa/sfx, on a fake world shaped like Rapier 0.21:
 * an impact's inputs come from the pre-step velocities, resting contacts and quick repeats stay
 * silent, a hard enough hit breaks a breakable material, and both sides of a pair ring.
 *
 *   node --test packages/haa/test/sfx-rapier.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import type { RBody, RCollider, RManifold, RVec, RWorld } from '../src/sfx/rapier.ts';
import { createRapierSfx } from '../src/sfx/rapier.ts';
import type { SfxEvent } from '../src/sfx/contract.ts';
import { breaksOn, defineMaterial, energyFromJoules, resolveInteraction, sizeFromMass } from '../src/sfx/contract.ts';

const v = (x: number, y: number, z: number): RVec => ({ x, y, z });

class Body implements RBody {
  lin = v(0, 0, 0);
  ang = v(0, 0, 0);
  pos = v(0, 0, 0);
  readonly handle: number;
  private readonly m: number;
  private readonly fixed: boolean;
  constructor(handle: number, m: number, fixed = false) {
    this.handle = handle;
    this.m = m;
    this.fixed = fixed;
  }
  mass() { return this.fixed ? 0 : this.m; }
  linvel() { return this.lin; }
  angvel() { return this.ang; }
  translation() { return this.pos; }
  isFixed() { return this.fixed; }
  isDynamic() { return !this.fixed; }
  isSleeping() { return false; }
}

class Collider implements RCollider {
  readonly handle: number;
  private readonly body: Body;
  constructor(handle: number, body: Body) {
    this.handle = handle;
    this.body = body;
  }
  parent() { return this.body; }
  translation() { return this.body.pos; }
  mass() { return this.body.mass(); }
}

/** A world with one contact pair whose manifold the test sets. */
function world(colliders: Collider[]) {
  const contact = { normal: v(0, 1, 0), point: v(0, 0, 0), impulse: 0, depth: 0, solver: 1 };
  const w: RWorld = {
    getCollider: (h) => colliders.find((c) => c.handle === h) ?? null,
    forEachActiveRigidBody: (f) => colliders.forEach((c) => !c.parent().isFixed() && f(c.parent())),
    contactPair: (_c1, _c2, f) => {
      const m: RManifold = {
        normal: () => contact.normal,
        numContacts: () => 1,
        contactDist: () => -contact.depth,
        contactImpulse: () => contact.impulse,
        numSolverContacts: () => contact.solver,
        solverContactPoint: () => contact.point,
      };
      f(m, false);
    },
    castRayAndGetNormal: () => null,
  };
  return { w, contact };
}

const wood = defineMaterial({ name: 'wood', hardness: 0.55, sounds: { impact: 'wood-impact' } });
const glass = defineMaterial({ name: 'glass', hardness: 0.95, sounds: { impact: 'glass-impact', break: 'glass-break' }, breakEnergy: 2 });

function setup(strikerMat = glass, mass = 0.5) {
  const ground = new Body(1, 0, true);
  const bottle = new Body(2, mass);
  const cg = new Collider(10, ground);
  const cb = new Collider(20, bottle);
  const { w, contact } = world([cg, cb]);
  const events: SfxEvent[] = [];
  const broken: number[] = [];
  const mats = new Map([[10, wood], [20, strikerMat]]);
  const sfx = createRapierSfx({
    world: w,
    rapier: { Ray: class {} },
    materialOf: (c) => mats.get(c.handle) ?? null,
    emit: (e) => events.push(e),
    onBreak: (c) => broken.push(c.handle),
  });
  return { sfx, bottle, contact, events, broken };
}

test('an impact takes its energy and size from the pre-step velocity and the striker mass', () => {
  const { sfx, bottle, contact, events } = setup(glass, 0.5);
  bottle.lin = v(0, -2, 0); // 0.5 · 0.5 kg · 4 = 1 J, below the 2 J break
  sfx.beforeStep();
  bottle.lin = v(0, 0, 0); // the solver stopped it
  contact.impulse = 1;
  sfx.contactStarted(10, 20);
  sfx.afterStep(null, 1 / 60);
  const impacts = events.filter((e) => e.kind === 'impact');
  assert.equal(impacts.length, 1);
  const e = impacts[0];
  assert.equal(e.striker, glass);
  assert.equal(e.surface, wood);
  assert.ok(Math.abs(e.joules! - 1) < 1e-9);
  assert.ok(Math.abs(e.inputs.energy! - energyFromJoules(1)) < 1e-12);
  assert.ok(Math.abs(e.inputs.size! - sizeFromMass(0.5)) < 1e-12);
  // Both sides ring, each with the other's hardness.
  const sounds = resolveInteraction(e.kind, e.striker, e.surface, e.inputs, e.surfaceInputs);
  assert.deepEqual(sounds.map((s) => [s.slot.preset, s.inputs.hardness]), [['glass-impact', 0.55], ['wood-impact', 0.95]]);
});

test('an impact with no solver contact is placed at the moving body, not halfway to the fixed mesh origin', () => {
  // A level mesh (the vault) has its collider at the world origin: the middle of the two origins put a
  // bullet's hit on the vault's far wall 12 m away, outside the stone.
  const { sfx, bottle, contact, events } = setup(wood, 0.01);
  bottle.pos = v(23.8, 1.1, 16);
  bottle.lin = v(0, -120, 0);
  sfx.beforeStep();
  bottle.lin = v(0, 70, 0);
  contact.impulse = 1.9;
  contact.solver = 0;
  sfx.contactStarted(10, 20);
  sfx.afterStep(null, 1 / 60);
  const e = events.find((x) => x.kind === 'impact')!;
  assert.ok(e, 'the hit plays');
  const d = Math.hypot(e.position.x - 23.8, e.position.y - 1.1, e.position.z - 16);
  assert.ok(d < 0.1, `hit placed ${d.toFixed(2)} m from the bullet`);
});

test('a resting contact and a repeat within the cooldown stay silent', () => {
  const { sfx, bottle, contact, events } = setup(wood, 2);
  bottle.lin = v(0, -0.1, 0); // 0.1 m/s: resting
  sfx.beforeStep();
  contact.impulse = 0.03;
  sfx.contactStarted(10, 20);
  sfx.afterStep(null, 1 / 60);
  assert.equal(events.length, 0);
  // A real bounce inside the pair cooldown (60 ms) is dropped, after it it plays.
  bottle.lin = v(0, -3, 0);
  sfx.beforeStep();
  contact.impulse = 5;
  sfx.afterStep(null, 1 / 60);
  const first = events.length;
  assert.equal(first, 1);
  sfx.beforeStep();
  contact.impulse = 40;
  sfx.afterStep(null, 1 / 60);
  assert.equal(events.length, first);
});

test('a hit above breakEnergy emits a break of the breakable side and calls onBreak', () => {
  const { sfx, bottle, contact, events, broken } = setup(glass, 0.5);
  bottle.lin = v(0, -6, 0); // 9 J
  sfx.beforeStep();
  contact.impulse = 3;
  sfx.contactStarted(10, 20);
  sfx.afterStep(null, 1 / 60);
  const brk = events.find((e) => e.kind === 'break');
  assert.ok(brk);
  assert.equal(brk.striker, glass);
  assert.deepEqual(broken, [20]);
});

test('a soft surface takes the blow: the same hit lands on snow unbroken, breaks on concrete', () => {
  const snow = defineMaterial({ name: 'snow', hardness: 0.05, sounds: { impact: 'snow-impact' } });
  const concrete = defineMaterial({ name: 'concrete', hardness: 0.9, sounds: { impact: 'solid-impact' } });
  assert.equal(breaksOn(glass, snow, 50), false);
  assert.equal(breaksOn(glass, concrete, 2.5), true);
  // wood (0.55) raises the threshold x1.6: 2.5 J holds, 4 J breaks
  assert.equal(breaksOn(glass, wood, 2.5), false);
  assert.equal(breaksOn(glass, wood, 4), true);
});
