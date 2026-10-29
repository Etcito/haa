// Unit tests for the MP3 bake (bake module): a synthetic 0.5 s family bakes to an MP3 whose first
// frame header reads MPEG-1 Layer III, 128 kbps, 48 kHz, mono; the manifest follows the shared
// format; two bakes into separate folders are byte-identical (Q3); only 48 kHz / 128 kbps is accepted.
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import type { AudioRng } from '../../src/dsp/rng.ts'
import { bake } from '../../src/bake/bake.ts'
import { BASE_PRESET } from '../../src/mix/presets/base.ts'
import { defineFamily } from '../../src/registry/registry.ts'

/** A synthetic family: an exponentially decaying noise burst from the rng, under 0.5 s. */
const TONE = defineFamily({
  id: 'test.tone',
  bus: 'effects',
  category: 'impact',
  variants: 8,
  maxSeconds: 0.5,
  render(out: Float32Array, sr: number, rng: AudioRng, v: number): number {
    const n = Math.min(out.length, Math.floor(0.3 * sr))
    const tau = (0.04 + 0.005 * v) * sr
    for (let i = 0; i < n; i++) out[i] += 0.5 * (rng.next() * 2 - 1) * Math.exp(-i / tau)
    return n
  },
})

const dirs: string[] = []

/** Bake TONE into a fresh temp directory and return it with the manifest. */
function bakeOnce(): { dir: string; manifest: ReturnType<typeof bake> } {
  const dir = mkdtempSync(join(tmpdir(), 'haa-bake-'))
  dirs.push(dir)
  return { dir, manifest: bake([TONE], { outDir: dir, preset: BASE_PRESET }) }
}

afterAll(() => {
  for (const d of dirs) rmSync(d, { recursive: true, force: true })
})

describe('bake', () => {
  const first = bakeOnce()
  const mp3 = readFileSync(join(first.dir, 'test.tone.mp3'))
  const manifestText = readFileSync(join(first.dir, 'manifest.json'), 'utf8')

  it('writes an MPEG-1 Layer III 128 kbps 48 kHz mono frame', () => {
    let at = -1
    for (let i = 0; i + 3 < mp3.length; i++) {
      if (mp3[i] === 0xff && (mp3[i + 1]! & 0xe0) === 0xe0) {
        at = i
        break
      }
    }
    expect(at).toBeGreaterThanOrEqual(0)
    const b1 = mp3[at + 1]!
    const b2 = mp3[at + 2]!
    const b3 = mp3[at + 3]!
    expect((b1 >> 3) & 3).toBe(3) // version bits 11 = MPEG-1
    expect((b1 >> 1) & 3).toBe(1) // layer bits 01 = Layer III
    expect(b2 >> 4).toBe(9) // MPEG-1 Layer III bitrate index 9 = 128 kbps
    expect((b2 >> 2) & 3).toBe(1) // MPEG-1 sample rate index 1 = 48000 Hz
    expect(b3 >> 6).toBe(3) // channel mode 3 = mono
  })

  it('returns and writes the manifest in the shared format', () => {
    const m = first.manifest
    expect(m.families).toHaveLength(1)
    const e = m.families[0]!
    expect(e.id).toBe('test.tone')
    expect(e.file).toBe('test.tone.mp3')
    expect(e.variants).toHaveLength(8)
    expect(e.pcmHash).toMatch(/^[0-9a-f]{8}$/)
    expect(e.mp3Sha256).toBe(createHash('sha256').update(mp3).digest('hex'))
    expect(m.preset).toBe(BASE_PRESET.id)
    expect(manifestText.endsWith('\n')).toBe(true)
    expect(manifestText).not.toMatch(/time|date/i)
    expect(JSON.parse(manifestText)).toEqual(m)
  })

  it('gives byte-identical MP3 and manifest on a second bake (Q3)', () => {
    const second = bakeOnce()
    expect(second.dir).not.toBe(first.dir)
    expect(readFileSync(join(second.dir, 'test.tone.mp3')).equals(mp3)).toBe(true)
    expect(readFileSync(join(second.dir, 'manifest.json'), 'utf8')).toBe(manifestText)
  })

  it('rejects any rate other than 48000 Hz', () => {
    const dir = mkdtempSync(join(tmpdir(), 'haa-bake-'))
    dirs.push(dir)
    expect(() => bake([TONE], { outDir: dir, rate: 44100 as 48000, preset: BASE_PRESET })).toThrow(
      '[haa] bake supports 48000 Hz / 128 kbps in v0',
    )
  })
})
