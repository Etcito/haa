// The sound-effect contract shared by game code, the physics adapters and the preset authors:
// the published inputs (all 0..1), the interaction kinds, the sound material slot and the
// unit conversions from physics (joules, kilograms, metres per second, metres) to 0..1.
// Pure and realm-neutral (docs/physics-sound.md).
import type { Vec3 } from './acoustics.ts';

/**
 * Published inputs every sfx preset may read through an `input` node; each one is 0..1. `density`
 * is the impact rate of a swarm (impacts per second / `DENSITY_FULL`); only cluster presets read it.
 */
export const SFX_INPUTS = ['energy', 'size', 'hardness', 'depth', 'speed', 'density'] as const;
export type SfxInputName = (typeof SFX_INPUTS)[number];
export type SfxInputs = Record<SfxInputName, number>;

/** What happened between two materials. `slide` and `roll` are continuous, the others one-shot. */
export type InteractionKind = 'impact' | 'slide' | 'roll' | 'step' | 'break';
export const INTERACTION_KINDS: readonly InteractionKind[] = ['impact', 'slide', 'roll', 'step', 'break'];

export function isContinuous(kind: InteractionKind): boolean {
  return kind === 'slide' || kind === 'roll';
}

/** One slot of a material: the preset it plays, a level trim and fixed input overrides. */
export interface SoundSlot {
  /** Preset name, e.g. `wood-crate-impact` (a file `patches/sfx/<name>.json`). */
  preset: string;
  /** Linear level trim of this slot, default 1. */
  gain?: number;
  /** Inputs forced for this slot, e.g. `{ size: 0.2 }` for a light tin can. */
  inputs?: Partial<SfxInputs>;
  /**
   * Swarm preset (impact slots of debris: pebbles, shards). When many impacts of this material fall
   * in the same short window and region, the first and loudest keep their own voices and the rest
   * drive ONE continuous voice of this preset through `density`, `energy`, `size` and `hardness`.
   */
  cluster?: SoundSlot | string;
}

/** A sound material: which preset each interaction plays, plus the physical hints. */
export interface SoundMaterial {
  name: string;
  /** 0 = soft (snow, cloth), 1 = hard (metal, stone). The other side receives it as `hardness`. */
  hardness: number;
  /** Acoustic absorption coefficient α of a surface made of it (Sabine, 0..1). Snow ≈ 0.6, concrete ≈ 0.02. */
  absorption: number;
  sounds: Partial<Record<InteractionKind, SoundSlot>>;
  /** Kinetic energy in joules above which an impact breaks a body made of it (needs a `break` slot). */
  breakEnergy?: number;
}

/** A material written loosely: slots may be bare preset names, hints default to 0.5 / 0.1. */
export interface SoundMaterialSpec {
  name: string;
  hardness?: number;
  absorption?: number;
  sounds?: Partial<Record<InteractionKind, SoundSlot | string>>;
  breakEnergy?: number;
}

export function defineMaterial(spec: SoundMaterialSpec): SoundMaterial {
  const sounds: Partial<Record<InteractionKind, SoundSlot>> = {};
  for (const kind of INTERACTION_KINDS) {
    const slot = spec.sounds?.[kind];
    if (slot) sounds[kind] = typeof slot === 'string' ? { preset: slot } : { ...slot };
    const cluster = sounds[kind]?.cluster;
    if (typeof cluster === 'string') sounds[kind]!.cluster = { preset: cluster };
  }
  return {
    name: spec.name,
    hardness: clamp01(spec.hardness ?? 0.5),
    absorption: clamp01(spec.absorption ?? 0.1),
    sounds,
    breakEnergy: spec.breakEnergy,
  };
}

// --- physics units to 0..1 -------------------------------------------------------------------

/** Kinetic energy range mapped log-linearly onto `energy`: a 1 g grain at 0.45 m/s .. 1 kJ. */
export const ENERGY_JOULES: readonly [number, number] = [1e-4, 1e3];
/** Mass range mapped log-linearly onto `size`: 1 g .. 1 t. */
export const SIZE_KG: readonly [number, number] = [1e-3, 1e3];
/** Tangential speed at which `speed` reaches 1 (m/s). */
export const SPEED_FULL = 10;
/** Penetration or sinking at which `depth` reaches 1 (m). */
export const DEPTH_FULL = 0.1;
/** Impact rate at which `density` reaches 1 (impacts per second). */
export const DENSITY_FULL = 200;

export function clamp01(v: number): number {
  return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
}

function logUnit(v: number, [lo, hi]: readonly [number, number]): number {
  if (!(v > 0)) return 0;
  return clamp01((Math.log10(v) - Math.log10(lo)) / (Math.log10(hi) - Math.log10(lo)));
}

export const energyFromJoules = (joules: number): number => logUnit(joules, ENERGY_JOULES);
export const sizeFromMass = (kg: number): number => logUnit(kg, SIZE_KG);
export const speedFromVelocity = (mps: number): number => clamp01(mps / SPEED_FULL);
export const depthFromMetres = (m: number): number => clamp01(m / DEPTH_FULL);

/** Inverse of `energyFromJoules`, for game code that thinks in joules. */
export function joulesFromEnergy(energy: number): number {
  const [lo, hi] = ENERGY_JOULES;
  return 10 ** (Math.log10(lo) + clamp01(energy) * (Math.log10(hi) - Math.log10(lo)));
}

/**
 * Level used when a preset does not read `energy` itself (the stand-ins): the energy axis spans
 * seven decades of joules, mapped onto 40 dB, so 1 → 0 dB, 0.5 → −20 dB, 0 → −40 dB.
 */
export function fallbackGain(energy: number): number {
  return 10 ** (-2 * (1 - clamp01(energy)));
}

export function fullInputs(partial: Partial<SfxInputs> | undefined): SfxInputs {
  return {
    energy: clamp01(partial?.energy ?? 0.5),
    size: clamp01(partial?.size ?? 0.5),
    hardness: clamp01(partial?.hardness ?? 0.5),
    depth: clamp01(partial?.depth ?? 0),
    speed: clamp01(partial?.speed ?? 0),
    density: clamp01(partial?.density ?? 0),
  };
}

// --- pair resolution -------------------------------------------------------------------------

/** One preset to play for an interaction, with the inputs it receives. */
export interface ResolvedSound {
  material: SoundMaterial;
  kind: InteractionKind;
  slot: SoundSlot;
  inputs: SfxInputs;
  /** Level of this voice in the mix of the pair (both sides of an impact share the loudness). */
  gain: number;
}

/**
 * The presets an interaction plays. `striker` is the moving object, `surface` the one it hits.
 * - impact: both sides ring; each receives the other's hardness; one preset once (−3 dB each when two play).
 * - slide: the surface's slot (scraping on gravel), else the striker's.
 * - roll: the striker's slot (a rolling barrel), else the surface's.
 * - step: the ground's slot only.
 * - break: the breaking object's (the striker's) slot only.
 */
export function resolveInteraction(
  kind: InteractionKind,
  striker: SoundMaterial,
  surface: SoundMaterial,
  inputs: Partial<SfxInputs>,
  surfaceInputs?: Partial<SfxInputs>,
): ResolvedSound[] {
  const base = fullInputs(inputs);
  const pick = (m: SoundMaterial, other: SoundMaterial): ResolvedSound | null => {
    const slot = m.sounds[kind];
    if (!slot) return null;
    const own = m === surface && m !== striker ? surfaceInputs : undefined;
    const merged = fullInputs({ ...base, ...own, hardness: other.hardness, ...slot.inputs });
    return { material: m, kind, slot, inputs: merged, gain: slot.gain ?? 1 };
  };
  switch (kind) {
    case 'impact': {
      const a = pick(striker, surface);
      const b = pick(surface, striker);
      if (a && b && a.slot.preset === b.slot.preset) return [a];
      const both = [a, b].filter((x): x is ResolvedSound => x !== null);
      if (both.length === 2) for (const s of both) s.gain *= Math.SQRT1_2;
      return both;
    }
    case 'slide': {
      const s = pick(surface, striker) ?? pick(striker, surface);
      return s ? [s] : [];
    }
    case 'roll': {
      const s = pick(striker, surface) ?? pick(surface, striker);
      return s ? [s] : [];
    }
    case 'step': {
      const s = pick(surface, striker);
      return s ? [s] : [];
    }
    case 'break': {
      const s = pick(striker, surface);
      return s ? [s] : [];
    }
  }
}

// --- events ----------------------------------------------------------------------------------

/** A sound event, from a physics adapter or from game code. */
export interface SfxEvent {
  kind: InteractionKind;
  position: Vec3;
  /** The moving object (the bottle, the boot, the bullet). */
  striker: SoundMaterial;
  /** What it hits (the floor, the snow, the metal plate). */
  surface: SoundMaterial;
  /** Inputs of the striker's voice, shared by the surface's voice unless `surfaceInputs` overrides them. */
  inputs: Partial<SfxInputs>;
  surfaceInputs?: Partial<SfxInputs>;
  /** Identity of a continuous interaction (slide/roll): `start`, then `update`s, then `end`. */
  key?: string;
  phase?: 'start' | 'update' | 'end';
  /** Physical values behind the inputs, for logs. */
  joules?: number;
  normalSpeed?: number;
  tangentSpeed?: number;
}
