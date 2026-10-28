// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports, biome ids replaced by generic zone ids
// ReverbZones: the per-zone reverb presets and absorption scales (module gen).
// DSP-safe: it runs unchanged in node, on the main thread and in AudioWorkletGlobalScope.
//
// REVERB_ZONES holds one ReverbParams per generic zone, 'underwater' included. The size/decay
// values are the authored identity of each zone (a small open field, a deep cavern); every field
// the list leaves out takes the base preset default (damping 6000 Hz, predelay 0.02 s,
// returnDb -14 dB), so a zone is only ever special where it matters.
//
// ABSORPTION_SCALE feeds the spatial model's absorptionScale: air absorption is divided by it,
// so a snowfield (0.6) and a dense forest (0.7) muffle distant sources sooner while the thin
// high air of the sky (1.4) carries them further. Every other zone is neutral at 1.
//
// blendReverb linearly interpolates every field (size, decay, damping, predelay, returnDb)
// into the caller's `out` and returns it. It clamps t to 0..1, treats NaN as 0 and never
// allocates, so a caller can blend zones in its frame loop without garbage.
import type { ReverbParams } from '../contract.ts'

/** A generic acoustic zone id, each with its own reverb preset. */
export type ReverbZoneId =
  | 'open'
  | 'canyon'
  | 'snowfield'
  | 'cavern'
  | 'water'
  | 'city'
  | 'sky'
  | 'forest'
  | 'room'
  | 'underwater'

/** A full reverb preset for one zone. */
function zone(size: number, decay: number, damping = 6000, predelay = 0.02, returnDb = -14): ReverbParams {
  return { size, decay, damping, predelay, returnDb }
}

/** One reverb preset per zone, the submerged zone included. */
export const REVERB_ZONES: Record<ReverbZoneId, ReverbParams> = {
  open: zone(0.3, 0.8),
  canyon: zone(0.8, 2.2, 6000, 0.06),
  snowfield: zone(0.5, 1.4, 3500),
  cavern: zone(0.9, 2.8),
  water: zone(0.6, 1.6),
  city: zone(0.7, 1.8),
  sky: zone(0.2, 0.6),
  forest: zone(0.5, 1.2, 3000),
  room: zone(0.4, 1),
  underwater: zone(0.6, 1.8, 1200)
}

/** Per-zone air absorption multiplier (0.6 snowfield / 0.7 forest / 1.4 sky / 1 elsewhere). */
export const ABSORPTION_SCALE: Record<Exclude<ReverbZoneId, 'underwater'>, number> = {
  open: 1,
  canyon: 1,
  snowfield: 0.6,
  cavern: 1,
  water: 1,
  city: 1,
  sky: 1.4,
  forest: 0.7,
  room: 1
}

/** Linear blend of a and b into out (t clamped to 0..1, NaN counts as 0). No allocation. */
export function blendReverb(a: ReverbParams, b: ReverbParams, t: number, out: ReverbParams): ReverbParams {
  const k = t >= 1 ? 1 : t > 0 ? t : 0
  out.size = a.size + (b.size - a.size) * k
  out.decay = a.decay + (b.decay - a.decay) * k
  out.damping = a.damping + (b.damping - a.damping) * k
  out.predelay = a.predelay + (b.predelay - a.predelay) * k
  out.returnDb = a.returnDb + (b.returnDb - a.returnDb) * k
  return out
}
