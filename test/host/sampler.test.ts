// Sampler host tests: centered single-listener playback, two-listener split and stop fades.
import { describe, expect, it } from 'vitest'
import { BLOCK, BUS_INDEX, MIX_INPUTS, SP, SP_LENGTH } from '../../src/contract.ts'
import { Sampler } from '../../src/host/sampler.ts'

const SR = 48000
const pairs = (): [Float32Array, Float32Array][] =>
  Array.from({ length: MIX_INPUTS }, () => [new Float32Array(BLOCK), new Float32Array(BLOCK)])

function spBlock(gains: number[], pans: number[]): Float32Array {
  const sp = new Float32Array(SP_LENGTH)
  sp[SP.COUNT] = gains.length
  sp[SP.SEND] = 0.1
  gains.forEach((g, l) => { sp[SP.GAIN + l] = g; sp[SP.PAN + l] = pans[l]; sp[SP.LOWPASS + l] = 20000 })
  return sp
}

function variant(len: number): Float32Array {
  const x = new Float32Array(len)
  for (let i = 0; i < len; i++) x[i] = 0.5 * Math.sin(i * 0.37) * Math.cos(i * 0.011)
  return x
}

function setup(gains: number[], pans: number[]): { s: Sampler; x: Float32Array } {
  const s = new Sampler(SR)
  const x = variant(BLOCK * 4)
  s.load('t', [x], SR)
  s.play({ voice: 3, id: 't', variant: 0, bus: BUS_INDEX.effects, gain: 1, rate: 1, sp: spBlock(gains, pans) })
  return { s, x }
}

describe('Sampler', () => {
  it('one listener at gain 1 and pan 0 gives the variant x sqrt(1/2) on both channels', () => {
    // At lowpass 20000 the one-pole is bypassed (not transparent at 48 kHz otherwise).
    const { s, x } = setup([1], [0])
    const out = pairs()
    for (let b = 0; b < 4; b++) {
      out[0][0].fill(9)
      s.process(out, BLOCK)
      expect(out[0][0][0]).toBe(0)
      const [l, r] = out[BUS_INDEX.effects]
      for (let i = 0; i < BLOCK; i++) {
        const want = x[b * BLOCK + i] * Math.SQRT1_2
        expect(Math.abs(l[i] - want)).toBeLessThan(1e-6)
        expect(Math.abs(r[i] - want)).toBeLessThan(1e-6)
      }
    }
  })

  it('two listeners at gain 0.5 with pans -1 and +1 split left and right', () => {
    const { s, x } = setup([0.5, 0.5], [-1, 1])
    const out = pairs()
    s.process(out, BLOCK)
    const [l, r] = out[BUS_INDEX.effects]
    for (let i = 0; i < BLOCK; i++) {
      expect(Math.abs(l[i] - 0.5 * x[i])).toBeLessThan(1e-6)
      expect(Math.abs(r[i] - 0.5 * x[i])).toBeLessThan(1e-6)
    }
  })

  it('a stop fade reaches exactly 0 and the voice is done', () => {
    const { s } = setup([1], [0])
    const out = pairs()
    s.process(out, BLOCK)
    s.stop(3, 64 / SR)
    expect(s.voiceDone(3)).toBe(false)
    s.process(out, BLOCK)
    const l = out[BUS_INDEX.effects][0]
    expect(Math.abs(l[60])).toBeGreaterThan(0)
    for (let i = 64; i < BLOCK; i++) expect(l[i]).toBe(0)
    expect(s.voiceDone(3)).toBe(true)
    expect(s.playing).toBe(0)
  })
})
