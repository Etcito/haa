import { describe, expect, it } from 'vitest'
import { BLOCK, SP, SP_LENGTH } from '../../src/contract.ts'
import type { BedDef, BedVoice } from '../../src/contract.ts'
import { AdditiveBank } from '../../src/dsp/osc.ts'
import { BedHost } from '../../src/host/bedHost.ts'

const SR = 48000

/** Test-only bed: a 440 Hz two-partial additive tone scaled by param 0 ('level'). */
function toneDef(positional: boolean): BedDef {
  return {
    id: positional ? 'test-tone-pos' : 'test-tone',
    bus: 'ambience',
    params: ['level', 'doppler'],
    defaults: [0.5, 1],
    positional,
    create(sr: number): BedVoice {
      const bank = new AdditiveBank(2)
      bank.setPartial(0, 1, 0.8)
      bank.setPartial(1, 2, 0.2)
      const mono = new Float32Array(BLOCK)
      return {
        process(p, offset, l, r, n) {
          mono.fill(0)
          bank.render(440 * p[offset + 1], mono, n, sr, p[offset])
          for (let i = 0; i < n; i++) { l[i] += mono[i]; r[i] += mono[i] }
        },
        reset() { bank.reset() },
      }
    },
  }
}

function run(host: BedHost, blocks: number): { l: Float32Array; r: Float32Array; sl: Float32Array } {
  const l = new Float32Array(blocks * BLOCK)
  const r = new Float32Array(blocks * BLOCK)
  const sl = new Float32Array(blocks * BLOCK)
  const bus: [Float32Array, Float32Array] = [new Float32Array(BLOCK), new Float32Array(BLOCK)]
  const send: [Float32Array, Float32Array] = [new Float32Array(BLOCK), new Float32Array(BLOCK)]
  for (let b = 0; b < blocks; b++) {
    host.process(bus, send, BLOCK)
    l.set(bus[0], b * BLOCK)
    r.set(bus[1], b * BLOCK)
    sl.set(send[0], b * BLOCK)
  }
  return { l, r, sl }
}

function rmsDb(x: Float32Array): number {
  let s = 0
  for (const v of x) s += v * v
  return 10 * Math.log10(s / x.length + 1e-30)
}

function spPanned(pan: number): Float32Array {
  const sp = new Float32Array(SP_LENGTH)
  sp[SP.COUNT] = 1
  sp[SP.SEND] = 0.1
  sp[SP.GAIN] = 1
  sp[SP.PAN] = pan
  sp[SP.LOWPASS] = 20000
  return sp
}

describe('BedHost', () => {
  it('pans a positional bed hard left and hard right', () => {
    for (const pan of [-1, 1]) {
      const host = new BedHost(SR, toneDef(true), 1)
      host.setSpatial(1, spPanned(pan))
      const { l, r } = run(host, 40)
      const [loud, quiet] = pan < 0 ? [l, r] : [r, l]
      expect(rmsDb(loud)).toBeGreaterThan(-30)
      expect(rmsDb(loud) - rmsDb(quiet)).toBeGreaterThanOrEqual(40)
    }
  })

  it('applies trimDb to a non-positional bed and sends bus × sp[SEND]', () => {
    const ref = run(new BedHost(SR, toneDef(false), 1), 20)
    const trimmed = run(new BedHost(SR, toneDef(false), 1, -6), 20)
    expect(rmsDb(trimmed.l) - rmsDb(ref.l)).toBeCloseTo(-6, 3)
    for (let i = 0; i < ref.l.length; i += 97) expect(ref.sl[i]).toBeCloseTo(ref.l[i] * 0.08, 6)
  })

  it('ends after the fade and stays silent', () => {
    const host = new BedHost(SR, toneDef(false), 1)
    run(host, 4)
    host.end(0.05)
    expect(host.done).toBe(false)
    run(host, Math.ceil((0.05 * SR) / BLOCK))
    expect(host.done).toBe(true)
    const after = run(host, 4)
    expect(after.l.every((v) => v === 0) && after.r.every((v) => v === 0)).toBe(true)
  })

  it('smooths a param ramp instead of jumping to the target', () => {
    const host = new BedHost(SR, toneDef(false), 1)
    const base = rmsDb(run(host, 40).l.subarray(20 * BLOCK))
    host.setParams([1, 1])
    const first = rmsDb(run(host, 1).l)
    run(host, 200)
    const settled = rmsDb(run(host, 20).l)
    expect(settled - base).toBeCloseTo(6.02, 0)
    expect(first).toBeLessThan(settled - 1)
    expect(first).toBeGreaterThan(base - 1)
  })
})
