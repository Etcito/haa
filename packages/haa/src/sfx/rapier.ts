// Rapier adapter: turns contacts of a Rapier world (@dimforge/rapier3d-compat or the
// deterministic build, 0.21 API) into sound events. Typed structurally, so this package never
// imports Rapier. Feed it either an EventQueue (`afterStep(queue)`) or contact starts and stops
// you already have (`contactStarted`/`contactStopped`: a polling game, or the Rust engine's
// `ege.contact` events).
//
// Physics -> inputs:
//   energy   0.5 · m_eff · v_n²  (pre-step relative normal speed at the contact point), log-scaled
//   size     the striker's mass, log-scaled (1 g .. 1 t)
//   hardness the other material's hardness (contract.resolveInteraction)
//   depth    penetration of the deepest contact point (/ 10 cm) × (1 − surface hardness)
//   speed    tangential slip speed at the contact (/ 10 m/s)
import type { RayCast, RayHit, Vec3 } from './acoustics.ts';
import { add, cross, dot, length, scale, sub, v3 } from './acoustics.ts';
import type { InteractionKind, SfxEvent, SoundMaterial } from './contract.ts';
import { depthFromMetres, energyFromJoules, sizeFromMass, speedFromVelocity } from './contract.ts';

// --- the part of Rapier this adapter touches -------------------------------------------------

export interface RVec {
  x: number;
  y: number;
  z: number;
}
export interface RBody {
  handle: number;
  mass(): number;
  linvel(): RVec;
  angvel(): RVec;
  translation(): RVec;
  worldCom?(): RVec;
  isFixed(): boolean;
  isDynamic(): boolean;
  isSleeping(): boolean;
}
export interface RCollider {
  handle: number;
  parent(): RBody | null;
  translation(): RVec;
  mass(): number;
}
export interface RManifold {
  normal(): RVec;
  numContacts(): number;
  contactDist(i: number): number;
  contactImpulse(i: number): number;
  numSolverContacts(): number;
  solverContactPoint(i: number): RVec | null;
}
export interface RWorld {
  getCollider(handle: number): RCollider | null;
  forEachActiveRigidBody(f: (body: RBody) => void): void;
  contactPair(c1: RCollider, c2: RCollider, f: (m: RManifold, flipped: boolean) => void): void;
  castRayAndGetNormal(
    ray: unknown,
    maxToi: number,
    solid: boolean,
    filterFlags?: number,
    filterGroups?: number,
    filterExcludeCollider?: RCollider,
    filterExcludeRigidBody?: RBody,
    filterPredicate?: (collider: RCollider) => boolean,
  ): { collider: RCollider; timeOfImpact: number; normal: RVec } | null;
}
export interface REventQueue {
  drainCollisionEvents(f: (h1: number, h2: number, started: boolean) => void): void;
  drainContactForceEvents(f: (e: { collider1(): number; collider2(): number; maxForceMagnitude(): number }) => void): void;
}
/** The Rapier module (for `new Ray(origin, dir)`). */
export interface RModule {
  Ray: new (origin: RVec, dir: RVec) => unknown;
}

// --- options ---------------------------------------------------------------------------------

export interface RapierSfxOptions {
  world: RWorld;
  rapier: RModule;
  /** The sound material of a collider, or null for a silent one. */
  materialOf(collider: RCollider): SoundMaterial | null;
  /** Receives every event (wire it to `SfxEngine.handle`). */
  emit(event: SfxEvent): void;
  /** Called when an impact exceeds a material's `breakEnergy` (spawn the shards here). */
  onBreak?(collider: RCollider, event: SfxEvent): void;
  /** Relative normal speed below which a contact makes no impact (m/s). Default 0.35. */
  minImpactSpeed?: number;
  /** Minimum time between two impacts of the same pair (s). Default 0.06. */
  pairCooldown?: number;
  /** Minimum time between two impacts of the same collider with anything (s). Default 0.02. */
  colliderCooldown?: number;
  /** Tangential slip above which a continuous slide/roll runs (m/s). Default 0.15. */
  minSlideSpeed?: number;
}

interface Snapshot {
  lin: RVec;
  ang: RVec;
  com: RVec;
}

interface Pair {
  key: string;
  c1: number;
  c2: number;
  emaImpulse: number;
  lastImpact: number;
  continuous: InteractionKind | null;
  quietSince: number;
}

const pairKey = (a: number, b: number): string => (a < b ? `${a}:${b}` : `${b}:${a}`);

export interface RapierSfx {
  /** Snapshot the velocities of awake bodies; call right before `world.step()`. */
  beforeStep(): void;
  /** Drain `queue` (if any) and emit this step's events; call right after `world.step()`. */
  afterStep(queue: REventQueue | null, dt: number): void;
  contactStarted(h1: number, h2: number): void;
  contactStopped(h1: number, h2: number): void;
  /** A ray cast over the colliders, for acoustics: hits carry the material's absorption. */
  rayCast(filter?: (collider: RCollider) => boolean): RayCast;
  /** Material under a point (ray straight down), e.g. for footsteps. */
  surfaceBelow(p: Vec3, maxDist?: number): { material: SoundMaterial; point: Vec3; collider: RCollider } | null;
  readonly activePairs: number;
}

export function createRapierSfx(o: RapierSfxOptions): RapierSfx {
  const minImpact = o.minImpactSpeed ?? 0.35;
  const pairCooldown = o.pairCooldown ?? 0.06;
  const colliderCooldown = o.colliderCooldown ?? 0.02;
  const minSlide = o.minSlideSpeed ?? 0.15;
  const snapshots = new Map<number, Snapshot>();
  const pairs = new Map<string, Pair>();
  const started: Array<[number, number]> = [];
  const stopped: Array<[number, number]> = [];
  const forced = new Set<string>();
  const lastByCollider = new Map<number, number>();
  let time = 0;

  function velocityAt(body: RBody | null, p: RVec): Vec3 {
    if (!body || body.isFixed()) return v3(0, 0, 0);
    const s = snapshots.get(body.handle);
    const lin = s?.lin ?? (body.isSleeping() ? v3(0, 0, 0) : body.linvel());
    const ang = s?.ang ?? (body.isSleeping() ? v3(0, 0, 0) : body.angvel());
    const com = s?.com ?? (body.worldCom?.() ?? body.translation());
    return add(lin, cross(ang, sub(p, com)));
  }

  function currentVelocityAt(body: RBody | null, p: RVec): Vec3 {
    if (!body || body.isFixed() || body.isSleeping()) return v3(0, 0, 0);
    return add(body.linvel(), cross(body.angvel(), sub(p, body.worldCom?.() ?? body.translation())));
  }

  /** The deepest contact of a pair: world point, normal from c1 to c2, depth, total impulse. */
  function contact(c1: RCollider, c2: RCollider): { point: Vec3; normal: Vec3; depth: number; impulse: number } | null {
    let best: { point: Vec3; normal: Vec3; depth: number; impulse: number } | null = null;
    o.world.contactPair(c1, c2, (m, flipped) => {
      const n = m.normal();
      const normal = flipped ? v3(-n.x, -n.y, -n.z) : v3(n.x, n.y, n.z);
      let impulse = 0;
      let depth = 0;
      for (let i = 0; i < m.numContacts(); i++) {
        impulse += m.contactImpulse(i);
        depth = Math.max(depth, -m.contactDist(i));
      }
      let point: Vec3 | null = null;
      const k = m.numSolverContacts();
      if (k > 0) {
        let sx = 0;
        let sy = 0;
        let sz = 0;
        for (let i = 0; i < k; i++) {
          const p = m.solverContactPoint(i);
          if (p) {
            sx += p.x;
            sy += p.y;
            sz += p.z;
          }
        }
        point = v3(sx / k, sy / k, sz / k);
      }
      if (!point) {
        const a = c1.translation();
        const b = c2.translation();
        point = scale(add(a, b), 0.5);
      }
      if (!best || impulse > best.impulse) best = { point, normal, depth, impulse };
    });
    return best;
  }

  function massOf(c: RCollider): number {
    const body = c.parent();
    if (!body || body.isFixed()) return Infinity;
    return body.mass() || c.mass() || 1;
  }

  function cooled(pair: Pair, c1: number, c2: number): boolean {
    if (time - pair.lastImpact < pairCooldown) return false;
    if (time - (lastByCollider.get(c1) ?? -1) < colliderCooldown) return false;
    if (time - (lastByCollider.get(c2) ?? -1) < colliderCooldown) return false;
    return true;
  }

  /** Emit an impact (and a break) for a pair if it hits hard enough; true when it did. */
  function tryImpact(pair: Pair, c1: RCollider, c2: RCollider, m1: SoundMaterial, m2: SoundMaterial, requireSpike: boolean): boolean {
    const hit = contact(c1, c2);
    if (!hit) return false;
    const spike = hit.impulse > Math.max(1e-4, pair.emaImpulse * 4);
    pair.emaImpulse = pair.emaImpulse * 0.8 + hit.impulse * 0.2;
    if (requireSpike && !spike) return false;
    const b1 = c1.parent();
    const b2 = c2.parent();
    const rel = sub(velocityAt(b1, hit.point), velocityAt(b2, hit.point));
    const vn = Math.abs(dot(rel, hit.normal));
    if (vn < minImpact || !cooled(pair, c1.handle, c2.handle)) return false;
    const vt = length(sub(rel, scale(hit.normal, dot(rel, hit.normal))));
    const ma = massOf(c1);
    const mb = massOf(c2);
    const mEff = !Number.isFinite(ma) ? mb : !Number.isFinite(mb) ? ma : (ma * mb) / (ma + mb);
    const joules = 0.5 * mEff * vn * vn;
    // The striker is the faster side at the contact; a fixed body is always the surface.
    const s1 = length(velocityAt(b1, hit.point));
    const s2 = length(velocityAt(b2, hit.point));
    const firstStrikes = !Number.isFinite(mb) || (Number.isFinite(ma) && s1 >= s2);
    const [striker, surface, sc, fc] = firstStrikes ? [m1, m2, c1, c2] : [m2, m1, c2, c1];
    const strikerMass = massOf(sc);
    const surfaceMass = massOf(fc);
    const event: SfxEvent = {
      kind: 'impact',
      position: hit.point,
      striker,
      surface,
      inputs: {
        energy: energyFromJoules(joules),
        size: sizeFromMass(Number.isFinite(strikerMass) ? strikerMass : 1000),
        // A rigid solver's penetration is sinking only into a soft surface (snow, mud).
        depth: depthFromMetres(hit.depth) * (1 - surface.hardness),
        speed: speedFromVelocity(vt),
      },
      // A fixed surface rings over the area the striker excites; a loose one by its own mass.
      surfaceInputs: Number.isFinite(surfaceMass) ? { size: sizeFromMass(surfaceMass) } : undefined,
      joules,
      normalSpeed: vn,
      tangentSpeed: vt,
    };
    pair.lastImpact = time;
    lastByCollider.set(c1.handle, time);
    lastByCollider.set(c2.handle, time);
    o.emit(event);
    for (const [c, m, other] of [[c1, m1, m2], [c2, m2, m1]] as const) {
      if (m.sounds.break && m.breakEnergy !== undefined && joules >= m.breakEnergy) {
        const brk: SfxEvent = { ...event, kind: 'break', striker: m, surface: other, surfaceInputs: undefined };
        o.emit(brk);
        o.onBreak?.(c, brk);
      }
    }
    return true;
  }

  /** Slide/roll of a pair in contact: start, update or end its continuous voice. */
  function continuous(pair: Pair, c1: RCollider, c2: RCollider, m1: SoundMaterial, m2: SoundMaterial): void {
    const wants = m1.sounds.slide || m1.sounds.roll || m2.sounds.slide || m2.sounds.roll;
    if (!wants) return;
    const hit = contact(c1, c2);
    let kind: InteractionKind | null = null;
    let speed = 0;
    let striker = m1;
    let surface = m2;
    let force = 0;
    if (hit) {
      const b1 = c1.parent();
      const b2 = c2.parent();
      const rel = sub(currentVelocityAt(b1, hit.point), currentVelocityAt(b2, hit.point));
      const slip = length(sub(rel, scale(hit.normal, dot(rel, hit.normal))));
      const moving = b1 && !b1.isFixed() ? b1 : b2;
      const center = moving && !moving.isSleeping() ? length(moving.linvel()) : 0;
      if (moving === b2) [striker, surface] = [m2, m1];
      const rollSlot = striker.sounds.roll || surface.sounds.roll;
      const slideSlot = surface.sounds.slide || striker.sounds.slide;
      if (rollSlot && center > minSlide && slip < center * 0.3) {
        kind = 'roll';
        speed = center;
      } else if (slideSlot && slip > minSlide) {
        kind = 'slide';
        speed = slip;
      }
      force = hit.impulse;
    }
    if (kind) pair.quietSince = time;
    // Hold a running voice through 150 ms of stillness (a box catching on an edge).
    if (!kind && pair.continuous && time - pair.quietSince < 0.15) return;
    if (pair.continuous && kind !== pair.continuous) {
      endContinuous(pair, hit?.point ?? c1.translation());
    }
    if (!kind || !hit) return;
    const strikerCollider = striker === m1 ? c1 : c2;
    const mass = massOf(strikerCollider);
    const event: SfxEvent = {
      kind,
      key: `${kind}:${pair.key}`,
      phase: pair.continuous ? 'update' : 'start',
      position: hit.point,
      striker,
      surface,
      inputs: {
        speed: speedFromVelocity(speed),
        // Normal impulse per step as a pressing force, on the same log axis as impacts.
        energy: energyFromJoules(Math.max(1e-6, force * speed)),
        size: sizeFromMass(Number.isFinite(mass) ? mass : 1000),
        depth: depthFromMetres(hit.depth) * (1 - surface.hardness),
      },
      tangentSpeed: speed,
    };
    pair.continuous = kind;
    o.emit(event);
  }

  function endContinuous(pair: Pair, at: RVec): void {
    if (!pair.continuous) return;
    const c1 = o.world.getCollider(pair.c1);
    const c2 = o.world.getCollider(pair.c2);
    const m1 = c1 ? o.materialOf(c1) : null;
    const m2 = c2 ? o.materialOf(c2) : null;
    if (m1 && m2) {
      o.emit({
        kind: pair.continuous,
        key: `${pair.continuous}:${pair.key}`,
        phase: 'end',
        position: v3(at.x, at.y, at.z),
        striker: m1,
        surface: m2,
        inputs: {},
      });
    }
    pair.continuous = null;
  }

  function step(h1: number, h2: number, isNew: boolean): void {
    const key = pairKey(h1, h2);
    const pair = pairs.get(key);
    if (!pair) return;
    const c1 = o.world.getCollider(h1);
    const c2 = o.world.getCollider(h2);
    if (!c1 || !c2) return;
    const m1 = o.materialOf(c1);
    const m2 = o.materialOf(c2);
    if (!m1 || !m2) return;
    const b1 = c1.parent();
    const b2 = c2.parent();
    const awake = (b: RBody | null): boolean => !!b && b.isDynamic() && !b.isSleeping();
    if (!isNew && !awake(b1) && !awake(b2) && !forced.has(key)) {
      if (pair.continuous) endContinuous(pair, c1.translation());
      return;
    }
    const impacted = tryImpact(pair, c1, c2, m1, m2, !isNew);
    if (!impacted) continuous(pair, c1, c2, m1, m2);
  }

  return {
    beforeStep() {
      snapshots.clear();
      o.world.forEachActiveRigidBody((b) => {
        if (!b.isDynamic()) return;
        snapshots.set(b.handle, { lin: b.linvel(), ang: b.angvel(), com: b.worldCom?.() ?? b.translation() });
      });
    },
    afterStep(queue, dt) {
      time += dt;
      queue?.drainCollisionEvents((h1, h2, isStart) => (isStart ? started : stopped).push([h1, h2]));
      queue?.drainContactForceEvents((e) => forced.add(pairKey(e.collider1(), e.collider2())));
      const fresh = new Set<string>();
      for (const [h1, h2] of started.splice(0)) {
        const key = pairKey(h1, h2);
        if (!pairs.has(key)) {
          pairs.set(key, { key, c1: h1, c2: h2, emaImpulse: 0, lastImpact: -1, continuous: null, quietSince: time });
        }
        fresh.add(key);
        step(h1, h2, true);
      }
      for (const pair of pairs.values()) if (!fresh.has(pair.key)) step(pair.c1, pair.c2, false);
      for (const [h1, h2] of stopped.splice(0)) {
        const key = pairKey(h1, h2);
        const pair = pairs.get(key);
        if (!pair) continue;
        const c = o.world.getCollider(h1);
        endContinuous(pair, c ? c.translation() : v3(0, 0, 0));
        pairs.delete(key);
      }
      forced.clear();
    },
    contactStarted(h1, h2) {
      started.push([h1, h2]);
    },
    contactStopped(h1, h2) {
      stopped.push([h1, h2]);
    },
    rayCast(filter) {
      return (origin: Vec3, dir: Vec3, maxDist: number): RayHit | null => {
        const ray = new o.rapier.Ray(origin, dir);
        const hit = o.world.castRayAndGetNormal(ray, maxDist, true, undefined, undefined, undefined, undefined, filter);
        if (!hit) return null;
        const m = o.materialOf(hit.collider);
        return {
          toi: hit.timeOfImpact,
          point: add(origin, scale(dir, hit.timeOfImpact)),
          normal: v3(hit.normal.x, hit.normal.y, hit.normal.z),
          absorption: m?.absorption ?? 0.1,
          id: hit.collider.handle,
        };
      };
    },
    surfaceBelow(p, maxDist = 3) {
      const ray = new o.rapier.Ray(p, v3(0, -1, 0));
      const hit = o.world.castRayAndGetNormal(ray, maxDist, true);
      if (!hit) return null;
      const m = o.materialOf(hit.collider);
      return m ? { material: m, point: v3(p.x, p.y - hit.timeOfImpact, p.z), collider: hit.collider } : null;
    },
    get activePairs() {
      return pairs.size;
    },
  };
}
