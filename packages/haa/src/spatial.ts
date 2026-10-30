// Spatial scene: distance falloff, equal-power pan and doppler for players placed in a
// world, plus the reserved patch inputs `_dist`, `_pan` and `_doppler` (docs/spatial.md).
// Browser-only at runtime (Web Audio nodes); the curve functions are pure.

import type { HaaPlayer } from './player.ts';

/** Distance falloff: full gain up to `ref` metres, silent beyond `max`. */
export interface Falloff {
  ref: number;
  max: number;
  rolloff: number;
}

export const DEFAULT_FALLOFF: Falloff = { ref: 2, max: 60, rolloff: 1 };

/** Metres per second. */
export const SPEED_OF_SOUND = 343;

/** Listener position and horizontal facing (fx, fz); +Z forward has +X on the right. */
export interface Listener {
  x: number;
  y: number;
  z: number;
  fx: number;
  fz: number;
}

export interface SpatialEmitter {
  readonly id: string;
  readonly player: HaaPlayer;
  setPosition(x: number, y: number, z: number): void;
  setVelocity(vx: number, vy: number, vz: number): void;
  setGain(g: number): void;
  dispose(): void;
}

export interface SpatialScene {
  readonly output: GainNode;
  setListener(l: Listener): void;
  addEmitter(id: string, player: HaaPlayer, falloff?: Falloff): SpatialEmitter;
  update(): void;
  dispose(): void;
}

/** 1 up to `ref`, then ref / (ref + rolloff × (dist − ref)), and 0 beyond `max`. */
export function emitterGain(dist: number, f: Falloff): number {
  if (!(dist > f.ref)) return 1;
  if (dist > f.max) return 0;
  return f.ref / (f.ref + f.rolloff * (dist - f.ref));
}

/** Equal-power channel gains for a pan in −1..1 (clamped). */
export function equalPowerPan(pan: number): { left: number; right: number } {
  const p = clamp(pan, -1, 1);
  const angle = ((p + 1) * Math.PI) / 4;
  return { left: Math.cos(angle), right: Math.sin(angle) };
}

/** c / (c − v) for a speed v toward the listener (m/s), clamped to 0.5..2. */
export function dopplerRatio(relSpeedTowardListener: number): number {
  const denom = SPEED_OF_SOUND - relSpeedTowardListener;
  if (!(denom > 0)) return 2;
  return clamp(SPEED_OF_SOUND / denom, 0.5, 2);
}

function clamp(v: number, lo: number, hi: number): number {
  return Number.isNaN(v) ? lo : Math.min(hi, Math.max(lo, v));
}

const RESERVED = ['_dist', '_pan', '_doppler'] as const;
type Reserved = (typeof RESERVED)[number];

/** Per-emitter state behind the public `SpatialEmitter`. */
interface EmitterState {
  emitter: SpatialEmitter;
  falloff: Falloff;
  gainNode: GainNode;
  panner: StereoPannerNode;
  pos: [number, number, number];
  vel: [number, number, number];
  gain: number;
  sent: Partial<Record<Reserved, number>>;
  disposed: boolean;
}

/** Disconnect a node, ignoring one that is already disconnected. */
function cut(node: AudioNode, dest?: AudioNode): void {
  try {
    if (dest) node.disconnect(dest);
    else node.disconnect();
  } catch {
    // Already disconnected.
  }
}

/**
 * Create a scene whose output gain feeds `output` (the context destination by default).
 * Each emitter routes its player through its own GainNode and StereoPannerNode.
 */
export function createSpatialScene(ctx: BaseAudioContext, output?: AudioNode): SpatialScene {
  const out = ctx.createGain();
  out.connect(output ?? ctx.destination);
  let listener: Listener = { x: 0, y: 0, z: 0, fx: 0, fz: 1 };
  const emitters = new Map<string, EmitterState>();

  function apply(s: EmitterState): void {
    const dx = s.pos[0] - listener.x;
    const dy = s.pos[1] - listener.y;
    const dz = s.pos[2] - listener.z;
    const dist = Math.hypot(dx, dy, dz);

    // Right vector of the facing (fz, −fx): facing +Z, +X is on the right.
    const flen = Math.hypot(listener.fx, listener.fz);
    const hlen = Math.hypot(dx, dz);
    let pan = 0;
    if (flen > 0 && hlen > 0) {
      pan = clamp(((dx / hlen) * listener.fz - (dz / hlen) * listener.fx) / flen, -1, 1);
    }

    // Speed toward the listener: the velocity against the emitter-to-listener direction.
    const toward = dist > 0 ? -(s.vel[0] * dx + s.vel[1] * dy + s.vel[2] * dz) / dist : 0;
    const doppler = dopplerRatio(toward);

    s.gainNode.gain.value = s.gain * emitterGain(dist, s.falloff);
    s.panner.pan.value = pan;

    const values: Record<Reserved, number> = { _dist: dist, _pan: pan, _doppler: doppler };
    const declared = s.emitter.player.info.inputs;
    for (const name of RESERVED) {
      if (!declared.some((input) => input.name === name)) continue;
      if (s.sent[name] === values[name]) continue;
      s.sent[name] = values[name];
      s.emitter.player.set(name, values[name]);
    }
  }

  function addEmitter(id: string, player: HaaPlayer, falloff: Falloff = DEFAULT_FALLOFF): SpatialEmitter {
    emitters.get(id)?.emitter.dispose();
    const gainNode = ctx.createGain();
    const panner = ctx.createStereoPanner();
    cut(player.node);
    player.node.connect(gainNode);
    gainNode.connect(panner);
    panner.connect(out);

    const emitter: SpatialEmitter = {
      id,
      player,
      setPosition(x, y, z) {
        state.pos = [x, y, z];
      },
      setVelocity(vx, vy, vz) {
        state.vel = [vx, vy, vz];
      },
      setGain(g) {
        state.gain = g;
      },
      dispose() {
        if (state.disposed) return;
        state.disposed = true;
        cut(player.node, gainNode);
        cut(gainNode);
        cut(panner);
        if (emitters.get(id) === state) emitters.delete(id);
      },
    };
    const state: EmitterState = {
      emitter,
      falloff,
      gainNode,
      panner,
      pos: [listener.x, listener.y, listener.z],
      vel: [0, 0, 0],
      gain: 1,
      sent: {},
      disposed: false,
    };
    emitters.set(id, state);
    apply(state);
    return emitter;
  }

  return {
    output: out,
    setListener(l) {
      listener = { ...l };
    },
    addEmitter,
    update() {
      for (const s of emitters.values()) apply(s);
    },
    dispose() {
      for (const s of [...emitters.values()]) s.emitter.dispose();
      cut(out);
    },
  };
}
