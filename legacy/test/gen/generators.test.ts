// Generators test (module generators): each generator renders 0.5 s at 48 kHz with a fixed
// seed; the output is finite, non-silent (RMS > -60 dBFS) and bit-identical on a second run.
// Everything is imported through src/index.ts, so the dsp and gen barrels are exercised.
import { describe, expect, it } from 'vitest'
import { AUDIO_RATE, AudioRng, BLOCK, dsp, gen, hashSeed } from '../../src/index.ts'

const SR = AUDIO_RATE
const N = Math.round(0.5 * SR)

/** Renders one generator into a fresh mono buffer of N frames with a fixed-seed rng. */
type Render = (out: Float32Array, rng: AudioRng) => void

function render(name: string, fn: Render): Float32Array {
  const out = new Float32Array(N)
  fn(out, new AudioRng(hashSeed('generators.test', name)))
  return out
}

/** Renders a block-based stereo source for N frames; returns L then R concatenated. */
function renderStereo(process: (l: Float32Array, r: Float32Array, n: number) => void): Float32Array {
  const both = new Float32Array(2 * N)
  const l = new Float32Array(BLOCK)
  const r = new Float32Array(BLOCK)
  for (let f = 0; f < N; f += BLOCK) {
    const n = Math.min(BLOCK, N - f)
    l.fill(0)
    r.fill(0)
    process(l, r, n)
    both.set(l.subarray(0, n), f)
    both.set(r.subarray(0, n), N + f)
  }
  return both
}

const CASES: Record<string, () => Float32Array> = {
  'renderImpact metal medium': () =>
    render('impact', (out, rng) => {
      gen.renderImpact({ material: 'metal', size: 'medium', velocity: 0.8, contact: 0.0015 }, SR, rng, out)
    }),
  softClick: () =>
    render('softClick', (out, rng) => {
      gen.softClick(out, { material: 'glass', pitch: 1, brightness: 0.5, level: 0.5 }, SR, rng)
    }),
  tick: () =>
    render('tick', (out, rng) => {
      gen.tick(out, { material: 'wood', pitch: 1, drift: 0.1, level: 0.5 }, SR, rng)
    }),
  chime: () =>
    render('chime', (out, rng) => {
      gen.chime(out, { notes: [880, 1320], spacing: 0.08, bright: 0.5, tail: 0.6, level: 0.5 }, SR, rng)
    }),
  swell: () =>
    render('swell', (out, rng) => {
      gen.swell(out, { notes: [220, 277.18, 329.63], seconds: 0.5, bright: 0.5, level: 0.5 }, SR, rng)
    }),
  resonantPing: () =>
    render('resonantPing', (out, rng) => {
      gen.resonantPing(out, 0, { hz: 660, q: 20, seconds: 0.5, level: 0.5, excite: 'click' }, SR, rng)
    }),
  'WindModel speed 30': () => {
    const wind = new gen.WindModel(SR, hashSeed('generators.test', 'wind'))
    // speed, speed01, air, boost, slip, yaw, underwater, level
    const p = new Float32Array([30, 0.6, 0, 0, 0, 0, 0, 1])
    return renderStereo((l, r, n) => wind.process(p, 0, l, r, n))
  },
  'PadInstrument noteOn': () => {
    const pad = new gen.PadInstrument(SR, hashSeed('generators.test', 'pad'))
    let first = true
    return renderStereo((l, r, n) => {
      if (first) {
        pad.noteOn(60, 0.8, 0)
        first = false
      }
      pad.process(l, r, n)
    })
  },
}

describe('generators', () => {
  for (const [name, run] of Object.entries(CASES)) {
    it(`${name}: finite, non-silent and deterministic`, () => {
      const a = run()
      expect(dsp.countNonFinite(a)).toBe(0)
      expect(dsp.rmsDb(a, 0, a.length)).toBeGreaterThan(-60)
      const b = run()
      expect(Buffer.from(b.buffer).equals(Buffer.from(a.buffer))).toBe(true)
    })
  }
})
