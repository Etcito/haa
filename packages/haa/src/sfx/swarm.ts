// Swarms of small impacts (40 pebbles, the shards of a bottle): one voice per tiny impact does not
// scale, so the first and the loudest impacts of a region keep their own voices and the rest feed
// ONE continuous voice of a cluster preset, driven by the impact rate (`density`) and the recent
// power-mean energy, mean size and mean hardness of the impacts it absorbed. Pure bookkeeping:
// the engine owns the voices (docs/physics-sound.md, "Swarms").
import type { Vec3 } from './acoustics.ts';
import { length, sub } from './acoustics.ts';
import { DENSITY_FULL } from './contract.ts';

export interface SwarmOptions {
  /** Impacts per region and window that keep their own voice. Default 3. */
  individual?: number;
  /** Extra own voices for an impact louder (by 0.05 energy) than every own voice of the window. Default 2. */
  standouts?: number;
  /** Window (s) over which the own voices of a region are counted. Default 0.3. */
  window?: number;
  /** Radius (m) of a region. Default 4. */
  radius?: number;
  /**
   * Time constant (s) of the impact-rate estimate and of the means. Default 0.08. The rate and the
   * power mean share it, so the grains emitted carry, on average, exactly the summed power of the
   * impacts: rate(t)·mean(t) = (1/τ)·Σ pᵢ·e^(−(t − tᵢ)/τ), whose integral is Σ pᵢ. (Means faster
   * than the rate left the bursts about 2 dB under their impacts.)
   */
  tau?: number;
  /** Time (s) after the last clustered impact before the cluster voice stops. Default 0.15. */
  hold?: number;
}

export interface Swarm {
  id: number;
  /** Cluster preset name: impacts of different presets that share it share the swarm. */
  key: string;
  center: Vec3;
  /** Last impact (own voice or clustered). */
  last: number;
  /** Own voices in the window: time and energy. */
  recent: Array<{ t: number; energy: number }>;
  /** Clustered impacts so far, and the last one's time. */
  clustered: number;
  lastClustered: number;
  rate: number;
  rateAt: number;
  /** Decaying sums (weight, power 10^(energy·48/10), size, hardness) of the clustered impacts, and their last update. */
  w: number;
  pow: number;
  meanAt: number;
  size: number;
  hardness: number;
}

/**
 * The energy is averaged as power on the 48 dB axis of the impact presets' energy-to-level maps:
 * an arithmetic mean of overlapping loud and soft impacts lost up to 4 dB of their summed level.
 */
const DB_PER_ENERGY = 48;

export interface SwarmInputs {
  energy: number;
  size: number;
  hardness: number;
  density: number;
}


export class SwarmTracker {
  readonly swarms: Swarm[] = [];
  private readonly o: Required<SwarmOptions>;
  private nextId = 1;

  constructor(o: SwarmOptions = {}) {
    this.o = { individual: 3, standouts: 2, window: 0.3, radius: 4, tau: 0.08, hold: 0.15, ...o };
  }

  get hold(): number {
    return this.o.hold;
  }

  /** Where an impact goes: its own voice, or the cluster voice of `swarm`. */
  route(key: string, at: Vec3, inputs: { energy: number; size: number; hardness: number }, now: number): { own: true } | { own: false; swarm: Swarm } {
    const o = this.o;
    let swarm = this.swarms.find((s) => s.key === key && now - s.last <= o.window + o.hold && length(sub(at, s.center)) <= o.radius);
    if (!swarm) {
      swarm = {
        id: this.nextId++, key, center: { ...at }, last: now, recent: [], clustered: 0, lastClustered: -Infinity,
        rate: 0, rateAt: now, w: 0, pow: 0, meanAt: now, size: 0, hardness: 0,
      };
      this.swarms.push(swarm);
    }
    swarm.recent = swarm.recent.filter((r) => now - r.t <= o.window);
    swarm.last = now;
    const k = 0.25;
    swarm.center = { x: swarm.center.x + (at.x - swarm.center.x) * k, y: swarm.center.y + (at.y - swarm.center.y) * k, z: swarm.center.z + (at.z - swarm.center.z) * k };
    const loudest = swarm.recent.reduce((a, r) => Math.max(a, r.energy), -Infinity);
    const own = swarm.recent.length < o.individual
      || (swarm.recent.length < o.individual + o.standouts && inputs.energy > loudest + 0.05);
    if (own) {
      swarm.recent.push({ t: now, energy: inputs.energy });
      return { own: true };
    }
    const decay = Math.exp(-(now - swarm.rateAt) / o.tau);
    swarm.rate = swarm.rate * decay + 1 / o.tau;
    swarm.rateAt = now;
    const fade = Math.exp(-(now - swarm.meanAt) / o.tau);
    swarm.w = swarm.w * fade + 1;
    swarm.pow = swarm.pow * fade + 10 ** ((inputs.energy * DB_PER_ENERGY) / 10);
    swarm.size = swarm.size * fade + inputs.size;
    swarm.hardness = swarm.hardness * fade + inputs.hardness;
    swarm.meanAt = now;
    swarm.clustered++;
    swarm.lastClustered = now;
    return { own: false, swarm };
  }

  /** The cluster voice's inputs at `now`: the rate decays between impacts, the means do not. */
  inputs(s: Swarm, now: number): SwarmInputs {
    const rate = s.rate * Math.exp(-Math.max(0, now - s.rateAt) / this.o.tau);
    const w = Math.max(1e-9, s.w);
    return {
      energy: Math.min(1, Math.max(0, (10 * Math.log10(Math.max(1e-12, s.pow / w))) / DB_PER_ENERGY)),
      size: s.size / w,
      hardness: s.hardness / w,
      density: Math.min(1, rate / DENSITY_FULL),
    };
  }

  /** True when the cluster voice of `s` should stop (no clustered impact for `hold`). */
  quiet(s: Swarm, now: number): boolean {
    return s.clustered > 0 && now - s.lastClustered > this.o.hold;
  }

  /** Forget the swarms idle for longer than window + hold; returns them. */
  prune(now: number): Swarm[] {
    const gone: Swarm[] = [];
    for (let i = this.swarms.length - 1; i >= 0; i--) {
      if (now - this.swarms[i].last > this.o.window + this.o.hold) gone.push(...this.swarms.splice(i, 1));
    }
    return gone;
  }
}
