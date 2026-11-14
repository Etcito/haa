// The sound-effect contract shared by game code, the physics adapters and the preset authors:
// the published inputs (all 0..1), the interaction kinds, the sound material slot and the
// unit conversions from physics (joules, kilograms, metres per second, metres) to 0..1.
// Pure and realm-neutral (docs/physics-sound.md).
import type { Vec3 } from './acoustics.ts';

/**
 * Published inputs every sfx preset may read through an `input` node; each one is 0..1. `density`
 * is the impact rate of a swarm (impacts per second / `DENSITY_FULL`); only cluster presets read it.
 * `shape`, `dim` and `thickness` describe the object that rings (its hand-chosen `SoundBody`);
 * `flux` and `area` describe a face under a distributed source (rain on a roof). A preset receives
 * only the inputs it declares.
 */
export const SFX_INPUTS = ['energy', 'size', 'hardness', 'depth', 'speed', 'density', 'shape', 'dim', 'thickness', 'flux', 'area', 'material'] as const;

/**
 * What an object is made of, for the generic presets that declare an enum input `material`
 * (`solid-impact`, `slide-scrape`, `roll-rumble`, `rain-surface`). In the contract the value is
 * index / 8 (`materialInput`); a preset that declares the enum receives the index. Sent only when
 * the material declares one: otherwise the preset keeps its own default.
 */
export const SOUND_MATERIALS = ['wood', 'metal', 'glass', 'stone', 'plastic', 'ceramic', 'concrete', 'tile', 'snow'] as const;
export type SoundMaterialKind = (typeof SOUND_MATERIALS)[number];

/** The contract value (0..1) of a material kind, e.g. for a slot's forced `inputs`. */
export function materialInput(kind: SoundMaterialKind): number {
  const i = SOUND_MATERIALS.indexOf(kind);
  return i < 0 ? Number.NaN : i / (SOUND_MATERIALS.length - 1);
}
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

/**
 * Resonator shapes an object can be declared with, chosen by hand (never computed from the mesh).
 * The `shape` input carries the index / (SHAPES.length − 1); a preset that declares `shape` with
 * min 0 and max 6 receives the index itself.
 *
 * - block:  a solid chunk (concrete slab, stone, brick): heavily damped, few low modes.
 * - plate:  a thick rigid plate (a tile, a wooden board, a manhole cover): plate modes, moderate ring.
 * - sheet:  a thin flexible sheet (corrugated metal roof, sheet steel, plywood skin): dense, long ring.
 * - bar:    a rod, beam or pipe: harmonic-ish bar modes (1 : 2.76 : 5.40 ...).
 * - box:    a hollow box (a crate, a cabinet): wall modes plus a cavity (Helmholtz) thump.
 * - shell:  a hollow curved body (a bucket, a can, a bell): ringing shell modes.
 * - bottle: a hollow body with a neck (a bottle, a jar): shell modes plus a cavity tone.
 */
export const SHAPES = ['block', 'plate', 'sheet', 'bar', 'box', 'shell', 'bottle'] as const;
export type ResonatorShape = (typeof SHAPES)[number];

/** Characteristic dimension range (largest extent) mapped log-linearly onto `dim`: 1 cm .. 10 m. */
export const DIM_METRES: readonly [number, number] = [0.01, 10];
/** Wall or plate thickness range mapped log-linearly onto `thickness`: 0.5 mm .. 0.5 m. */
export const THICKNESS_METRES: readonly [number, number] = [0.0005, 0.5];
/** Face area range mapped log-linearly onto `area`: 0.01 m² .. 10 000 m². */
export const AREA_M2: readonly [number, number] = [0.01, 1e4];

/**
 * The hand-chosen resonator of one object, next to its material: a hollow wooden box, a thick
 * concrete slab and a thin metal sheet hit with the same energy must not sound alike.
 */
export interface SoundBody {
  shape: ResonatorShape;
  /** Largest extent (m). */
  dim: number;
  /** Wall or plate thickness (m); a block's thickness is its dim. */
  thickness?: number;
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
  /** The object's resonator (see `withBody`): its presets receive `shape`, `dim` and `thickness`. */
  body?: SoundBody;
  /** What it is made of, sent as the `material` enum to the generic presets. */
  material?: SoundMaterialKind;
}

/** A material written loosely: slots may be bare preset names, hints default to 0.5 / 0.1. */
export interface SoundMaterialSpec {
  name: string;
  hardness?: number;
  absorption?: number;
  sounds?: Partial<Record<InteractionKind, SoundSlot | string>>;
  breakEnergy?: number;
  body?: SoundBody;
  material?: SoundMaterialKind;
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
    body: spec.body ? { ...spec.body } : undefined,
    material: spec.material,
  };
}

/**
 * One object's material: the shared material (its presets, hardness, absorption) with this object's
 * resonator. Return it from the adapter's `materialOf` for that object's colliders; the presets
 * stay the material's, only the body inputs change.
 */
export function withBody(material: SoundMaterial, body: SoundBody): SoundMaterial {
  return { ...material, body: { ...body } };
}

/** The 0..1 inputs of a body: `shape` (index / 6), `dim`, `thickness`. */
export function bodyInputs(body: SoundBody | undefined): Partial<SfxInputs> {
  if (!body) return {};
  const i = SHAPES.indexOf(body.shape);
  return {
    shape: i < 0 ? 0 : i / (SHAPES.length - 1),
    dim: logUnit(body.dim, DIM_METRES),
    thickness: logUnit(body.thickness ?? body.dim, THICKNESS_METRES),
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
export const areaFromSquareMetres = (m2: number): number => logUnit(m2, AREA_M2);

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
    shape: clamp01(partial?.shape ?? 0),
    dim: clamp01(partial?.dim ?? 0.4),
    thickness: clamp01(partial?.thickness ?? 0.5),
    flux: clamp01(partial?.flux ?? 1),
    area: clamp01(partial?.area ?? 0.4),
    // NaN when not given: the engine then leaves the preset's own default.
    material: Number.isFinite(partial?.material) ? clamp01(partial!.material!) : Number.NaN,
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
    // The preset of `m` rings `m`: it gets the other side's hardness and its own body.
    const made = m.material ? { material: materialInput(m.material) } : {};
    const merged = fullInputs({ ...base, ...own, hardness: other.hardness, ...bodyInputs(m.body), ...made, ...slot.inputs });
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
  /** Radius (m) of a large source (rain over an area, an explosion): it spreads around the listener up close. */
  extent?: number;
  /** Physical values behind the inputs, for logs. */
  joules?: number;
  normalSpeed?: number;
  tangentSpeed?: number;
}
