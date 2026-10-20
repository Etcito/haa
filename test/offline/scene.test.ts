import { describe, expect, it } from 'vitest'
import { AUDIO_RATE, BLOCK } from '../../src/contract.ts'
import { pcmHash } from '../../src/bank/pack.ts'
import { countNonFinite, rmsDb, samplePeakDb } from '../../src/dsp/meter.ts'
import { createOfflineAudio } from '../../src/offline/scene.ts'
import { registerBed } from '../../src/registry/registry.ts'

describe('offline scene', () => {
  it('renders a first-play wood impact audibly below the safety ceiling (Q1)', () => {
    const audio = createOfflineAudio()
    expect(audio.play('impact.wood.small')).not.toBeNull()
    audio.advance(1)
    const r = audio.take('wood')
    expect(r.sampleRate).toBe(AUDIO_RATE)
    expect(r.left.length).toBe(AUDIO_RATE)
    expect(r.right.length).toBe(AUDIO_RATE)
    expect(audio.time).toBe(1)
    expect(audio.stats).toEqual({ played: 1, dropped: 0, stolen: 0 })
    expect(countNonFinite(r.left) + countNonFinite(r.right)).toBe(0)
    expect(samplePeakDb(r.left, r.right)).toBeLessThanOrEqual(-1)
    expect(rmsDb(r.left, 0, r.left.length)).toBeGreaterThan(-60)
    expect(rmsDb(r.right, 0, r.right.length)).toBeGreaterThan(-60)
  })

  it('gives identical PCM for the same script in fresh scenes (Q3)', () => {
    const run = () => {
      const audio = createOfflineAudio({ seed: 91 })
      audio.play('impact.wood.small')
      audio.advance(0.1)
      audio.play('impact.wood.small')
      audio.advance(0.2)
      return pcmHash(audio.take('repeat').left)
    }
    expect(run()).toBe(run())
  })

  it('resolves mix ids, advances partial blocks and returns independent snapshots', () => {
    expect(() => createOfflineAudio({ mix: 'missing' })).toThrow('[haa] unknown mix missing')
    const audio = createOfflineAudio({ mix: 'base', sampleRate: 24000 })
    expect(audio.take('empty').left.length).toBe(0)
    audio.advance((BLOCK + 1) / 24000)
    const snapshot = audio.take('snapshot')
    snapshot.left.fill(1)
    snapshot.right.fill(1)
    audio.advance((BLOCK + 2) / 24000)
    const r = audio.take('grown')
    expect(r.left.length).toBe(2 * BLOCK + 3)
    expect(audio.time).toBe(r.left.length / 24000)
    expect(r.left.every((x) => x === 0) && r.right.every((x) => x === 0)).toBe(true)
  })

  it('sums multiple beds on their bus and send, and removes stopped beds', () => {
    let calls = 0
    registerBed({
      id: 'offline-test', bus: 'effects', positional: false,
      params: ['level'], defaults: [0.1], unsmoothed: ['level'],
      create: () => ({
        process(p, offset, l, r, n) {
          calls++
          for (let i = 0; i < n; i++) l[i] = r[i] = p[offset]
        },
        reset() {},
      }),
    })
    const audio = createOfflineAudio()
    const positive = audio.startBed('offline-test')!
    const negative = audio.startBed('offline-test', { values: [-0.1] })!
    audio.advance(0.02)
    const cancelled = audio.take('cancelled')
    expect(cancelled.left.every((x) => x === 0) && cancelled.right.every((x) => x === 0)).toBe(true)
    negative.stop(0)
    positive.setParam('level', 0.2)
    audio.advance(0.02)
    expect(rmsDb(audio.take('one').left, 960, 1920)).toBeGreaterThan(-60)
    positive.stop(0)
    const before = calls
    audio.advance(0.02)
    expect(calls).toBe(before)
  })
})
