// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports, renamed ids (legacy seed strings kept), no sfx aliases, render writes into the given buffer; tuned for m1 safety/loudness (HAA tuning comments)
// Baked music sting families (module content-core). DSP-safe: the same renderSting runs in the
// node proof, the build-time bake and the in-browser fallback. It imports only ../contract, ../dsp/*
// and ../music/instruments; every random draw comes from the seeded AudioRng and it uses no host API.
//
// Three families, bus 'music', category 'sting', 8 seeded variants each, fixed seed
// hashSeed('sting', legacy id): sting.climax (legacy id sting.finalLap), sting.win (legacy id sting.finishWin) and
// sting.end (legacy id sting.finish) keep their original seeds. A variant draws a jitter record (pad inversion, arpeggio rhythm offset
// +/- 1 sixteenth, per-instrument balance +/- 2 dB, velocity +/- 12%, decay +/- 15% and one detune
// seed per instrument) from its variant stream, then a builder schedules NoteEvents. Every
// instrument is instantiated at root 60 (C major); the feature plays the family at rate
// 2^((root - 60)/12). Events carry a frame offset inside the next BLOCK process call, process adds
// stereo into one scratch pair per slot, the slots are summed with the SLOT_MIX balance, the sum is
// glued (GLUE compressor, then a true-peak limiter LIMIT_DB under the sting's own peak) and the render
// adds (L + R)/2 into the first round(maxSeconds x sr) frames of the given buffer, with a 12 ms cosine fade at the end. The bank normalizes
// the peak, so the glue is what lets a sting sit at the music's loudness: the tonal body, not the
// snare or crash transients, sets its level.
// GM drum map (from ./instruments): kick 36, snare 38, rim 37, closed hat 42, open hat 46,
// crash 49, toms 45/47/50, shakers 70/82.
import type { FamilyDef } from '../contract.ts'
import { BLOCK } from '../contract.ts'
import { hashSeed } from '../dsp/rng.ts'
import type { AudioRng } from '../dsp/rng.ts'
import { clamp, dbToGain } from '../dsp/math.ts'
import { Compressor, TruePeakLimiter } from '../dsp/dynamics.ts'
import { DcBlocker } from '../dsp/filters.ts'
import type { CompPreset } from '../contract.ts'
import {
  PadInstrument, BassInstrument, LeadInstrument, PluckInstrument,
  KickInstrument, SnareInstrument, HatInstrument, DRUM_NOTES,
} from '../music/instruments.ts'
import type { Instrument } from '../music/instruments.ts'

/** Variants per sting family (within FAMILY_VARIANTS_MIN..MAX). */
const VARIANTS = 8
/** The cosine fade that ends every sting (s). */
const FADE = 0.012
/** Nominal 16th-note length for the arpeggio jitter (s): 120 bpm. */
const SIXTEENTH = 0.125
/** Instrument slots (LEAD2 is the octave-up doubling of the FM lead). */
const PAD = 0, BASS = 1, LEAD = 2, LEAD2 = 3, PLUCK = 4, KICK = 5, SNARE = 6, HAT = 7
const INST_COUNT = 8
/** Per-slot balance (linear): pad, bass, lead, lead2, pluck, kick, snare, hat/crash. The raw pad
 *  sits ~15 dB under the drums; the drums only punctuate. */
const SLOT_MIX = [4, 1, 1, 0.9, 1.2, 0.8, 0.35, 0.4]
/** Glue on the normalized sum (the sting's peak is scaled to 0 dBFS first). */
const GLUE: CompPreset = { thresholdDb: -20, ratio: 3, kneeDb: 6, attackMs: 4, releaseMs: 160, makeupDb: 0 }
/** Limiter ceiling (dBTP) after the glue: the remaining transients are held this far down. */
const LIMIT_DB = -11

/** One scheduled note: onset seconds, slot, midi, velocity, note-off seconds (-1 = one-shot). */
interface Note { at: number; inst: number; midi: number; vel: number; off: number }

/** Per-variant seeded jitter. */
interface Jitter {
  inv: number; arp: number; vel: number; decay: number
  bal: Float32Array; seeds: Uint32Array
}

/** Draws the jitter record, in field order, from the variant stream. */
function drawJitter(rng: AudioRng): Jitter {
  const bal = new Float32Array(INST_COUNT)
  const seeds = new Uint32Array(INST_COUNT)
  for (let i = 0; i < INST_COUNT; i++) {
    bal[i] = dbToGain(rng.range(-2, 2))
    seeds[i] = (rng.next() * 4294967296) >>> 0
  }
  return {
    inv: rng.int(0, 2), arp: rng.range(-SIXTEENTH, SIXTEENTH), vel: 1 + rng.range(-0.12, 0.12),
    decay: 1 + rng.range(-0.15, 0.15), bal, seeds,
  }
}

/** A note-onset record; dur <= 0 is a one-shot (drums). */
function put(notes: Note[], at: number, inst: number, midi: number, vel: number, dur: number): void {
  notes.push({ at, inst, midi, vel: clamp(vel, 0, 1), off: dur > 0 ? at + dur : -1 })
}

/** Triad plus inversion (0 root, 1 first, 2 second). */
function voice(t: readonly number[], inv: number): number[] {
  if (inv === 1) return [t[1], t[2], t[0] + 12]
  if (inv === 2) return [t[2], t[0] + 12, t[1] + 12]
  return [t[0], t[1], t[2]]
}

/** A held triad on one instrument, scaled by the velocity and decay jitter. */
function chord(notes: Note[], at: number, inst: number, triad: readonly number[], inv: number, vel: number, dur: number, j: Jitter): void {
  const v = voice(triad, inv)
  for (let k = 0; k < 3; k++) put(notes, at, inst, v[k], vel * j.vel * j.bal[inst], dur * j.decay)
}

const C_MAJOR: readonly number[] = [60, 64, 67]
const F_MAJOR: readonly number[] = [65, 69, 72]
const G_MAJOR: readonly number[] = [67, 71, 74]

/** finalLap: a rising 4-note lead fanfare doubled an octave up over a pad chord, a snare roll
 *  crescendo and a crash on the last note. */
function buildFinalLap(notes: Note[], j: Jitter): void {
  chord(notes, 0, PAD, C_MAJOR, j.inv, 0.6, 1.12, j)
  const fan = [55, 60, 64, 67]
  const at = [0.05, 0.3, 0.55, 0.8]
  for (let i = 0; i < 4; i++) {
    const dur = (i === 3 ? 0.34 : 0.22) * j.decay
    const vel = (0.72 + 0.08 * i) * j.vel
    put(notes, at[i], LEAD, fan[i], vel * j.bal[LEAD], dur)
    put(notes, at[i], LEAD2, fan[i] + 12, vel * 0.75 * j.bal[LEAD2], dur)
  }
  for (let i = 0; i < 8; i++) {
    put(notes, 0.02 + i * 0.095, SNARE, DRUM_NOTES.snare, (0.28 + i * 0.08) * j.vel * j.bal[SNARE], 0)
  }
  put(notes, 0.8, HAT, DRUM_NOTES.crash, 0.8 * j.vel * j.bal[HAT], 0)
}

/** finishWin: a IV-V-I cadence (F, G, C), the lead resolving to C, bass on the roots, a kick and
 *  a crash on the I and a bright pluck arpeggio. */
function buildFinishWin(notes: Note[], j: Jitter): void {
  const tri = [F_MAJOR, G_MAJOR, C_MAJOR]
  const roots = [53, 55, 48]
  const at = [0, 0.4, 0.8]
  for (let i = 0; i < 3; i++) {
    chord(notes, at[i], PAD, tri[i], j.inv, 0.5, 0.36, j)
    put(notes, at[i], BASS, roots[i], 0.62 * j.vel * j.bal[BASS], 0.38 * j.decay)
  }
  const mel = [69, 71, 72]
  for (let i = 0; i < 3; i++) {
    const dur = (i === 2 ? 0.5 : 0.34) * j.decay
    put(notes, at[i], LEAD, mel[i], 0.6 * j.vel * j.bal[LEAD], dur)
    put(notes, at[i], LEAD2, mel[i] + 12, 0.4 * j.vel * j.bal[LEAD2], dur)
  }
  put(notes, 0.8, KICK, DRUM_NOTES.kick, 0.9 * j.vel * j.bal[KICK], 0)
  put(notes, 0.8, HAT, DRUM_NOTES.crash, 0.8 * j.vel * j.bal[HAT], 0)
  const arp = j.inv === 1 ? [76, 79, 84, 88] : j.inv === 2 ? [79, 84, 88, 91] : [72, 76, 79, 84]
  for (let i = 0; i < 4; i++) {
    put(notes, 0.84 + i * 0.15 + j.arp, PLUCK, arp[i], 0.6 * j.vel * j.bal[PLUCK], 0.18 * j.decay)
  }
}

/** finish: a soft pad plus pluck I chord (C), lower velocities and no drums. */
function buildFinish(notes: Note[], j: Jitter): void {
  chord(notes, 0, PAD, C_MAJOR, j.inv, 0.5, 0.92, j)
  const arp = j.inv === 1 ? [64, 67, 72] : j.inv === 2 ? [67, 72, 76] : [60, 64, 67]
  for (let i = 0; i < 3; i++) put(notes, 0.02, PLUCK, arp[i], 0.4 * j.vel * j.bal[PLUCK], 0.32 * j.decay)
  for (let i = 0; i < 3; i++) {
    put(notes, 0.34 + i * 0.16 + j.arp, PLUCK, arp[(i + 1) % 3] + 12, 0.3 * j.vel * j.bal[PLUCK], 0.22 * j.decay)
  }
}

/** A builder: schedules the family's notes into `notes`. */
type Builder = (notes: Note[], j: Jitter) => void

/** Renders one variant: schedules the notes, runs the eight instruments in BLOCK chunks into
 *  preallocated stereo scratch, adds the mono sum with a short end fade into the first
 *  round(maxSeconds x sr) frames of `out` (capped at out.length) and returns that frame count. */
function renderSting(out: Float32Array, maxSeconds: number, sr: number, rng: AudioRng, build: Builder): number {
  const n = Math.min(out.length, Math.round(maxSeconds * sr))
  const j = drawJitter(rng)
  const notes: Note[] = []
  build(notes, j)
  const insts: Instrument[] = [
    new PadInstrument(sr, j.seeds[PAD]),
    new BassInstrument(sr, j.seeds[BASS]),
    new LeadInstrument(sr, j.seeds[LEAD]),
    new LeadInstrument(sr, j.seeds[LEAD2]),
    new PluckInstrument(sr, j.seeds[PLUCK]),
    new KickInstrument(sr, j.seeds[KICK]),
    new SnareInstrument(sr, j.seeds[SNARE]),
    new HatInstrument(sr, j.seeds[HAT]),
  ]
  const sl: Float32Array[] = [], sr2: Float32Array[] = []
  for (let k = 0; k < INST_COUNT; k++) { sl.push(new Float32Array(BLOCK)); sr2.push(new Float32Array(BLOCK)) }
  const L = new Float32Array(n)
  const R = new Float32Array(n)
  for (let base = 0; base < n; base += BLOCK) {
    const f = Math.min(BLOCK, n - base)
    for (let e = 0; e < notes.length; e++) {
      const ev = notes[e]
      const on = Math.round(ev.at * sr) - base
      if (on >= 0 && on < f) insts[ev.inst].noteOn(ev.midi, ev.vel, on)
      if (ev.off >= 0) {
        const of = Math.round(ev.off * sr) - base
        if (of >= 0 && of < f) insts[ev.inst].noteOff(ev.midi, of)
      }
    }
    for (let k = 0; k < INST_COUNT; k++) {
      const a = sl[k], b = sr2[k], g = SLOT_MIX[k]
      a.fill(0, 0, f); b.fill(0, 0, f)
      insts[k].process(a, b, f)
      for (let i = 0; i < f; i++) { L[base + i] += a[i] * g; R[base + i] += b[i] * g }
    }
  }
  // HAA tuning: the pluck and drum voices leave a DC offset (up to 2e-2 once normalized, m1
  // check 2); a DcBlocker per channel over the whole written length removes it before the glue.
  dcBlock(L, n)
  dcBlock(R, n)
  glue(L, R, n, sr)
  for (let i = 0; i < n; i++) L[i] = 0.5 * (L[i] + R[i])
  const fade = Math.min(n, Math.max(1, Math.round(FADE * sr)))
  for (let i = 0; i < fade; i++) L[n - fade + i] *= 0.5 + 0.5 * Math.cos((Math.PI * (i + 1)) / fade)
  for (let i = 0; i < n; i++) out[i] += L[i]
  return n
}

/** Runs x[0..n) through a fresh DcBlocker in place. */
function dcBlock(x: Float32Array, n: number): void {
  const dc = new DcBlocker()
  for (let i = 0; i < n; i++) x[i] = dc.process(x[i])
}

/** Scales the stereo sting to a 0 dBFS peak, then compresses and true-peak limits it in place. */
function glue(L: Float32Array, R: Float32Array, n: number, sr: number): void {
  let peak = 0
  for (let i = 0; i < n; i++) { const a = Math.abs(L[i]), b = Math.abs(R[i]); if (a > peak) peak = a; if (b > peak) peak = b }
  if (!(peak > 1e-9) || !Number.isFinite(peak)) return
  const g = 1 / peak
  for (let i = 0; i < n; i++) { L[i] *= g; R[i] *= g }
  const comp = new Compressor(sr)
  comp.set(GLUE)
  const lim = new TruePeakLimiter(sr)
  lim.set(LIMIT_DB, 1.5, 60)
  for (let base = 0; base < n; base += BLOCK) {
    const f = Math.min(BLOCK, n - base)
    const a = L.subarray(base, base + f), b = R.subarray(base, base + f)
    comp.processStereo(a, b, f)
    lim.processStereo(a, b, f)
  }
}

/** The shared sting family shape; the seed hashes `seedId` (the legacy id), not `id`. */
function stingFamily(id: string, seedId: string, maxSeconds: number, build: Builder): FamilyDef {
  return {
    id,
    bus: 'music',
    category: 'sting',
    variants: VARIANTS,
    maxSeconds,
    seed: hashSeed('sting', seedId),
    render: (out: Float32Array, sampleRate: number, rng: AudioRng, _v: number): number => renderSting(out, maxSeconds, sampleRate, rng, build),
  }
}

/** Every sting family (3 x 8 variants, 3.75 s each = 30 s). */
export const STING_FAMILIES: readonly FamilyDef[] = [
  stingFamily('sting.climax', 'sting.finalLap', 1.2, buildFinalLap),
  stingFamily('sting.win', 'sting.finishWin', 1.6, buildFinishWin),
  stingFamily('sting.end', 'sting.finish', 0.95, buildFinish),
]
