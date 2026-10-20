// The Royal Fiesta one-shot families (module fiesta-content). DSP-safe: the same fxKit/uiKit
// layers run in the node proof, the build-time bake and the in-browser fallback.
//
// Every family is bus 'effects', category 'fx', 8 seeded variants, the default seed
// hashSeed('family', id) (filled in by defineFamily). A variant first draws a jitter record
// (pitch, timing, layer balance, decay) from the family variant stream, then its recipe mixes
// at least three layers into the zeroed buffer through a scratch buffer, with the same rng.
// The shared finish DC-blocks the whole written length, peak-normalizes to -6 dBFS and ends
// with a short cosine fade, so a re-render from the same seed is bit-identical and no variant
// ends on a click. Recipes keep body energy after the onset (moderate crest factor), so the
// bank's -1.05 dBFS peak cap does not stop the loudness normalization short of the fx target.
import { defineFamily, dsp, gen, type AudioRng, type FamilyDef } from '@etcito/haa'

/** Variants per fiesta family (within FAMILY_VARIANTS_MIN..MAX). */
const VARIANTS = 8
/** Nominal peak of a rendered variant (dBFS). */
const PEAK_DB = -6
/** The cosine fade that ends every variant at its written length (s). */
const TAIL_FADE = 0.012

/** Per-variant seeded jitter; each recipe applies it to its layers. */
interface Jitter {
  /** Pitch ratio of every tonal layer and sweep, 1 +/- 6%. */
  pitch: number
  /** Onset offset ratio of the later layers, 1 +/- 15%. */
  timing: number
  /** Level ratio of the secondary layers (noise vs tone balance), 1 +/- 25%. */
  balance: number
  /** Decay and length ratio, 1 +/- 12%. */
  decay: number
}

/** A family recipe: mixes its layers into out (length maxSeconds x sr) through mix(). */
type Recipe = (out: Float32Array, sr: number, rng: AudioRng, j: Jitter) => void

/** Draws the jitter record, in field order, from the variant stream. */
function drawJitter(rng: AudioRng): Jitter {
  return {
    pitch: 1 + rng.range(-0.06, 0.06),
    timing: 1 + rng.range(-0.15, 0.15),
    balance: 1 + rng.range(-0.25, 0.25),
    decay: 1 + rng.range(-0.12, 0.12),
  }
}

/** Renders one layer into a zeroed scratch buffer of out's length, then adds scratch x gain
 *  into out. The layer writes from its own start; returns its end index. */
function mix(out: Float32Array, gain: number, layer: (scratch: Float32Array) => number): number {
  const scratch = new Float32Array(out.length)
  const end = Math.min(out.length, Math.max(0, layer(scratch)))
  for (let i = 0; i < end; i++) out[i] += scratch[i] * gain
  return end
}

/** HAA tuning: removes the layers' DC offset (sub glides, modal thuds) over the whole written
 *  length, before the peak normalization, so the normalized variants stay under |DC| 1e-3. */
function dcBlock(out: Float32Array): void {
  const dc = new dsp.DcBlocker()
  for (let i = 0; i < out.length; i++) out[i] = dc.process(out[i])
}

/** A cosine fade over the last TAIL_FADE seconds of out (no click at the end, Q8). */
function tailFade(out: Float32Array, sr: number): void {
  const n = Math.min(out.length, Math.max(1, Math.round(TAIL_FADE * sr)))
  const from = out.length - n
  for (let i = 0; i < n; i++) out[from + i] *= 0.5 + 0.5 * Math.cos((Math.PI * (i + 1)) / n)
}

/** Seconds to samples. */
function at(seconds: number, sr: number): number {
  return Math.round(seconds * sr)
}

/**
 * Cloth flutter: white noise through an SVF bandpass at `hz`, under a fast irregular AM (a
 * flap rate that random-walks around amHz, depth amDepth) and a raised-cosine attack then a
 * linear release to 0 at `seconds`, x level. Adds from start; returns the end index.
 */
function clothFlutter(
  out: Float32Array,
  start: number,
  o: { seconds: number; hz: number; q: number; amHz: number; amDepth: number; attack: number; level: number },
  sr: number,
  rng: AudioRng,
): number {
  const s0 = dsp.clamp(Math.trunc(start), 0, out.length)
  const end = Math.min(out.length, s0 + at(o.seconds, sr))
  const white = new dsp.WhiteNoise(rng)
  const svf = new dsp.Svf()
  svf.set(dsp.clamp(o.hz, 20, 0.45 * sr), o.q, sr)
  const norm = 1 / Math.sqrt(o.q)
  let phase = rng.next()
  let rate = o.amHz
  for (let i = s0; i < end; i++) {
    const t = (i - s0) / sr
    // The flap rate wanders +/- 40% around amHz, re-drawn every 64 samples.
    if (((i - s0) & 63) === 0) rate = dsp.clamp(rate + (rng.next() - 0.5) * 0.3 * o.amHz, 0.6 * o.amHz, 1.4 * o.amHz)
    phase += rate / sr
    if (phase >= 1) phase -= Math.floor(phase)
    const am = 1 - o.amDepth * (0.5 + 0.5 * Math.cos(dsp.TAU * phase))
    const att = t < o.attack ? 0.5 - 0.5 * Math.cos((Math.PI * t) / o.attack) : 1
    const rel = 1 - t / o.seconds
    svf.process(white.next())
    out[i] += svf.bp * norm * am * att * rel * o.level
  }
  return end
}

/** The shared fiesta family shape. render mixes the recipe into the first
 *  round(maxSeconds x sr) frames of the zeroed out, then finishes and returns that count. */
function fiestaFamily(id: string, maxSeconds: number, recipe: Recipe): FamilyDef {
  return defineFamily({
    id,
    bus: 'effects',
    category: 'fx',
    variants: VARIANTS,
    maxSeconds,
    render: (buf: Float32Array, sampleRate: number, rng: AudioRng, _v: number): number => {
      const n = Math.min(buf.length, Math.round(maxSeconds * sampleRate))
      const out = buf.subarray(0, n)
      recipe(out, sampleRate, rng, drawJitter(rng))
      dcBlock(out)
      gen.normalizePeak(out, PEAK_DB)
      tailFade(out, sampleRate)
      return n
    },
  })
}

/** Jump: a springy rising 'boing' (two slightly detuned harmonic stacks gliding up about an
 *  octave), a short upward air puff (gen.noiseSweep) and a small push-off sub thump. */
const jump: Recipe = (out, sr, rng, j) => {
  const p = j.pitch
  const len = 0.32 * j.decay
  const spring = { seconds: len, partials: 6, tiltDb: -7, inharm: 0.0015, detuneCents: 8, attack: 0.012, decay: 0.16 * j.decay }
  mix(out, 1, (s) => gen.harmonicStack(s, at(0.004, sr), { ...spring, f0: 210 * p, f1: 470 * p, level: 0.55 }, sr, rng))
  mix(out, 0.35, (s) => gen.harmonicStack(s, at(0.004, sr), { ...spring, f0: 213 * p, f1: 481 * p, partials: 3, level: 0.55 }, sr, rng))
  mix(out, 0.5 * j.balance, (s) => gen.noiseSweep(s, 0, { seconds: 0.13 * j.decay, f0: 700 * p, f1: 3600 * p, q: 1.6, curve: 1.2, level: 1 }, sr, rng))
  mix(out, 0.3, (s) => gen.subTransient(s, 0, { f0: 150 * p, f1: 70 * p, seconds: 0.11 * j.decay, level: 1 }, sr))
}

/** Dive: a falling whoosh (gen.noiseSweep 2.6 kHz down to 300 Hz), cloth flutter riding the
 *  whoosh, then a plastic belly-flop thud (gen.modalHit) when the body lands. */
const dive: Recipe = (out, sr, rng, j) => {
  const p = j.pitch
  const land = at(0.17 * j.timing, sr)
  mix(out, 0.9, (s) => gen.noiseSweep(s, 0, { seconds: 0.42 * j.decay, f0: 2600 * p, f1: 300 * p, q: 1.8, curve: 0.8, level: 1 }, sr, rng))
  mix(out, 0.45 * j.balance, (s) => clothFlutter(s, at(0.01, sr), { seconds: 0.26 * j.decay, hz: 1900 * p, q: 1.4, amHz: 32, amDepth: 0.8, attack: 0.03, level: 1 }, sr, rng))
  mix(out, 1, (s) => gen.modalHit(s, land, { material: 'plastic', size: 'medium', velocity: rng.range(0.85, 1), contact: rng.range(0.003, 0.006), level: 1.1 }, sr, rng))
  mix(out, 0.35, (s) => gen.subTransient(s, land, { f0: 120 * p, f1: 55 * p, seconds: 0.14 * j.decay, level: 1 }, sr))
}

/** Grab: a filtered-noise cloth snatch (short downward gen.noiseSweep), a low resonant ping
 *  (the grip on the costume) and a rubber squeak (bright harmonic stack bending up). */
const grab: Recipe = (out, sr, rng, j) => {
  const p = j.pitch
  mix(out, 0.8, (s) => gen.noiseSweep(s, 0, { seconds: 0.06 * j.decay, f0: 3400 * p, f1: 1300 * p, q: 1.2, curve: 1, level: 1 }, sr, rng))
  mix(out, 0.8, (s) => gen.resonantPing(s, at(0.003, sr), { hz: 190 * p, q: 9 * j.decay, seconds: 0.2, level: 1, excite: 'noise' }, sr, rng))
  mix(out, 0.5 * j.balance, (s) => gen.harmonicStack(s, at(0.025 * j.timing, sr), {
    seconds: 0.11 * j.decay, f0: 880 * p, f1: 1250 * p, partials: 5, tiltDb: -5, inharm: 0.001, detuneCents: 12,
    attack: 0.01, decay: 0.08 * j.decay, level: 1,
  }, sr, rng))
}

/**
 * Jelly wobble AM over out[start, end): gain 1 - depth e^(-t / decay) (0.5 - 0.5 cos phase),
 * the rate gliding exponentially from hz0 to hz1 over the span (a settling jelly slows down).
 * It starts at gain 1, so it adds no click at the layer onset.
 */
function wobbleAm(
  out: Float32Array,
  start: number,
  end: number,
  o: { hz0: number; hz1: number; depth: number; decay: number },
  sr: number,
): void {
  const s0 = dsp.clamp(Math.trunc(start), 0, out.length)
  const e = dsp.clamp(Math.trunc(end), s0, out.length)
  const span = Math.max(1, e - s0)
  let phase = 0
  for (let i = s0; i < e; i++) {
    const x = (i - s0) / span
    phase += (o.hz0 * Math.pow(o.hz1 / o.hz0, x)) / sr
    const depth = o.depth * Math.exp(-(i - s0) / sr / o.decay)
    out[i] *= 1 - depth * (0.5 - 0.5 * Math.cos(dsp.TAU * phase))
  }
}

/** Eliminate: a cartoon fall. A slide whistle (near-pure harmonic stack, two voices a few
 *  cents apart for a wavering beat) falls about two octaves, then the landing: a poof of
 *  air (wide, low-Q noise burst sweeping down) and a low boom under it. */
const eliminate: Recipe = (out, sr, rng, j) => {
  const p = j.pitch
  const fall = 0.78 * j.decay
  const whistle = { seconds: fall, f0: 1500 * p, f1: 360 * p, partials: 3, tiltDb: -14, inharm: 0, detuneCents: 3, attack: 0.03, decay: 1.6 * j.decay }
  mix(out, 0.9, (s) => gen.harmonicStack(s, 0, { ...whistle, level: 1 }, sr, rng))
  mix(out, 0.3, (s) => gen.harmonicStack(s, 0, { ...whistle, f0: 1512 * p, f1: 364 * p, partials: 2, level: 1 }, sr, rng))
  const land = at(0.74 * j.timing, sr)
  mix(out, 0.8 * j.balance, (s) => gen.noiseSweep(s, land, { seconds: 0.3 * j.decay, f0: 1700 * p, f1: 380 * p, q: 0.7, curve: 1.6, level: 1 }, sr, rng))
  mix(out, 0.9, (s) => gen.subTransient(s, land, { f0: 95 * p, f1: 42 * p, seconds: 0.36 * j.decay, level: 1 }, sr))
}

/** Wobble: a jelly body (soft harmonic stack sagging a little in pitch) under a decaying
 *  AM whose rate settles from about 9 Hz to about 4 Hz, a rubbery resonant ping on the
 *  onset and a small soft sub push. */
const wobble: Recipe = (out, sr, rng, j) => {
  const p = j.pitch
  const hz0 = rng.range(7.5, 9)
  const hz1 = rng.range(4, 5)
  mix(out, 1, (s) => {
    const end = gen.harmonicStack(s, at(0.005, sr), {
      seconds: 0.56 * j.decay, f0: 250 * p, f1: 215 * p, partials: 5, tiltDb: -6, inharm: 0.002, detuneCents: 10,
      attack: 0.015, decay: 0.24 * j.decay, level: 1,
    }, sr, rng)
    wobbleAm(s, at(0.005, sr), end, { hz0, hz1, depth: 0.85, decay: 0.3 * j.decay }, sr)
    return end
  })
  mix(out, 0.6 * j.balance, (s) => gen.resonantPing(s, 0, { hz: 150 * p, q: 7 * j.decay, seconds: 0.3, level: 1, excite: 'noise' }, sr, rng))
  mix(out, 0.3, (s) => gen.subTransient(s, 0, { f0: 120 * p, f1: 65 * p, seconds: 0.14 * j.decay, level: 1 }, sr))
}

/** Squash: a squelch (narrow band of noise snapping down from bright to low), the low
 *  thump of the body flattening and a short wet plastic click on the contact. */
const squash: Recipe = (out, sr, rng, j) => {
  const p = j.pitch
  mix(out, 1, (s) => gen.noiseSweep(s, 0, { seconds: 0.24 * j.decay, f0: 2600 * p, f1: 520 * p, q: 3.2, curve: 1.8, level: 1 }, sr, rng))
  mix(out, 0.3, (s) => gen.subTransient(s, at(0.03 * j.timing, sr), { f0: 150 * p, f1: 55 * p, seconds: 0.18 * j.decay, level: 1 }, sr))
  mix(out, 0.4 * j.balance, (s) => gen.softClick(s, { material: 'plastic', pitch: 0.55 * p, brightness: 0.25, level: 1, at: at(0.012 * j.timing, sr) }, sr, rng))
}

/** The fiesta families, sorted by id (BUILTIN_FAMILIES re-sorts anyway). */
export const FIESTA_FAMILIES: readonly FamilyDef[] = [
  fiestaFamily('fiesta.dive', 0.5, dive),
  fiestaFamily('fiesta.eliminate', 1.2, eliminate),
  fiestaFamily('fiesta.grab', 0.25, grab),
  fiestaFamily('fiesta.jump', 0.35, jump),
  fiestaFamily('fiesta.squash', 0.3, squash),
  fiestaFamily('fiesta.wobble', 0.6, wobble),
]
