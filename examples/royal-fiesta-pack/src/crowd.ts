// The Royal Fiesta crowd (module fiesta-content): the baked crowd one-shots and the live
// crowd bed. DSP-safe: renderCrowdShot and CrowdModel run unchanged in the node proof, the
// build-time bake, the in-browser fallback and the AudioWorklet (bed).
//
// CROWD_FAMILIES: bus 'ambience', category 'sting', 8 seeded variants, the default seed
// hashSeed('family', id). A variant draws its size, excitement and swell timing jitter from
// the family variant stream, then renderCrowdShot (same rng) seeds a fresh CrowdModel from
// it and writes a DC-blocked, faded, -6 dBFS-peak mono shot, so re-renders are bit-identical.
// CROWD_BED: the continuous crowd (size, excitement, swell, level), one CrowdModel per bed.
import { defineFamily, dsp, type AudioRng, type BedDef, type FamilyDef } from '@etcito/haa'
import { CROWD_PARAMS, CrowdModel, renderCrowdShot } from './crowdModel.ts'

/** Variants per crowd family (within FAMILY_VARIANTS_MIN..MAX). */
const VARIANTS = 8

/** A crowd shot recipe: nominal values, each jittered per variant from the rng. */
interface CrowdShot {
  /** Shot length and family maxSeconds (s). */
  seconds: number
  mood: 'cheer' | 'aww'
  /** Nominal crowd size 0..1, jittered by +/- sizeJitter (clamped). */
  size: number
  sizeJitter: number
  /** Nominal excitement 0..1, jittered by +/- 0.08 (clamped). */
  excitement: number
  /** Swell start and hold (s), each jittered by +/- 20%. */
  swellAt: number
  swellSeconds: number
}

/** Defines a crowd family rendering `shot` through renderCrowdShot. */
function crowdFamily(id: string, shot: CrowdShot): FamilyDef {
  return defineFamily({
    id,
    bus: 'ambience',
    category: 'sting',
    variants: VARIANTS,
    maxSeconds: shot.seconds,
    render: (out: Float32Array, sr: number, rng: AudioRng, _v: number): number => {
      // Jitter draws, in this order, from the variant stream.
      const size = dsp.clamp(shot.size + rng.range(-shot.sizeJitter, shot.sizeJitter), 0, 1)
      const excitement = dsp.clamp(shot.excitement + rng.range(-0.08, 0.08), 0, 1)
      const swellAt = shot.swellAt * rng.range(0.8, 1.2)
      const swellSeconds = shot.swellSeconds * rng.range(0.8, 1.2)
      return renderCrowdShot(out, { seconds: shot.seconds, excitement, swellAt, swellSeconds, size, mood: shot.mood }, sr, rng)
    },
  })
}

/** The crowd one-shots: a small cheer, a big cheer (full crowd, long swell) and an 'aww'. */
export const CROWD_FAMILIES: readonly FamilyDef[] = [
  crowdFamily('crowd.aww', { seconds: 1.8, mood: 'aww', size: 0.7, sizeJitter: 0.15, excitement: 0.35, swellAt: 0.1, swellSeconds: 0.8 }),
  crowdFamily('crowd.cheer.big', { seconds: 3.0, mood: 'cheer', size: 1, sizeJitter: 0.1, excitement: 0.9, swellAt: 0.2, swellSeconds: 1.8 }),
  crowdFamily('crowd.cheer.small', { seconds: 1.5, mood: 'cheer', size: 0.5, sizeJitter: 0.15, excitement: 0.6, swellAt: 0.15, swellSeconds: 0.5 }),
]

/** The live crowd bed (not positional): params CROWD_PARAMS, defaults size 0.6, excitement
 *  0.3, swell 0, level 1. */
export const CROWD_BED: BedDef = {
  id: 'crowd',
  bus: 'ambience',
  params: CROWD_PARAMS,
  defaults: [0.6, 0.3, 0, 1],
  positional: false,
  create: (sr: number, seed: number) => new CrowdModel(sr, seed),
}
