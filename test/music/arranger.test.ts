// Arranger unit tests (module music, owner M3): in-scale notes over 64 bars per style, a seeded
// deterministic NoteEvent stream, bar-quantized sections and style switches, and the empty-list
// error. State changes only at bar lines (every 16 nextStep calls) unless a test says otherwise.
import { describe, expect, it } from 'vitest'
import { Arranger } from '../../src/music/arranger.ts'
import type { MusicState, NoteEvent, Section } from '../../src/music/arranger.ts'
import type { MusicStyle } from '../../src/contract.ts'
import { CALM_STYLE, UPBEAT_STYLE } from '../../src/content/styles.ts'

const BAR = 16
const PITCHED = new Set<NoteEvent['inst']>(['pad', 'bass', 'lead', 'pluck'])

function state(o: Partial<MusicState> = {}): MusicState {
  return { style: 0, styleB: 0, styleMix: 0, intensity: 0.5, phase: 1, climax: 0, resolved: 0, won: 0, sparkle: 0, tension: 0, ...o }
}

/** The 64-bar script: intro 4, main 4..43 with intensity 0.2 -> 0.9 (sparkle in 20..35), climax
 *  44..51, then resolveWin and post until bar 63. */
function script(bar: number): MusicState {
  if (bar < 4) return state({ phase: 0, intensity: 0.3 })
  if (bar < 44) return state({ intensity: 0.2 + 0.7 * (bar - 4) / 39, sparkle: bar >= 20 && bar < 36 ? 1 : 0 })
  if (bar < 52) return state({ intensity: 0.9, climax: 1 })
  return state({ phase: 2, resolved: 1, won: 1, intensity: 0.5 })
}

function stream(seed: number, styles: readonly MusicStyle[], bars: number): string[] {
  const a = new Arranger(seed, styles), out: NoteEvent[] = [], ev: string[] = []
  for (let bar = 0; bar < bars; bar++) {
    a.setState(script(bar))
    for (let s = 0; s < BAR; s++) {
      const n = a.nextStep(out)
      for (let i = 0; i < n; i++) { const e = out[i]; ev.push(`${e.inst},${e.midi},${e.vel},${e.step},${e.len}`) }
    }
  }
  return ev
}

describe('Arranger', () => {
  for (const style of [UPBEAT_STYLE, CALM_STYLE]) {
    it(`keeps every pitched note in scale over 64 bars (${style.id})`, () => {
      const a = new Arranger(7, [style]), out: NoteEvent[] = []
      const counts: Record<string, number> = { pad: 0, bass: 0, lead: 0, pluck: 0 }
      let outOfScale = 0
      for (let bar = 0; bar < 64; bar++) {
        const st = script(bar)
        a.setState(st)
        for (let s = 0; s < BAR; s++) {
          const n = a.nextStep(out)
          for (let i = 0; i < n; i++) {
            const e = out[i]
            if (!PITCHED.has(e.inst)) continue
            counts[e.inst]++
            const pc = (((e.midi - style.root - (st.climax ? 2 : 0)) % 12) + 12) % 12
            if (!style.scale.includes(pc)) outOfScale++
          }
        }
      }
      expect(outOfScale).toBe(0)
      for (const k of Object.keys(counts)) expect(counts[k]).toBeGreaterThan(0)
    })
  }

  it('gives the same NoteEvent stream for the same seed and a different one for another seed', () => {
    const styles = [UPBEAT_STYLE, CALM_STYLE]
    const a = stream(11, styles, 32), b = stream(11, styles, 32), c = stream(12, styles, 32)
    expect(a.length).toBeGreaterThan(0)
    expect(b).toEqual(a)
    expect(c).not.toEqual(a)
  })

  it('switches sections on a bar line only', () => {
    const a = new Arranger(3, [UPBEAT_STYLE]), out: NoteEvent[] = []
    let prev: Section = a.section, changes = 0, idx = 0
    for (let bar = 0; bar < 64; bar++) {
      a.setState(script(bar))
      for (let s = 0; s < BAR; s++) {
        a.nextStep(out)
        if (a.section !== prev) { expect(idx % BAR).toBe(0); prev = a.section; changes++ }
        idx++
      }
    }
    expect(changes).toBeGreaterThanOrEqual(4)
  })

  it('throws on an empty style list', () => {
    expect(() => new Arranger(1, [])).toThrow('[haa] no music style')
  })

  it('applies a mid-bar style switch at the next bar line', () => {
    const a = new Arranger(5, [UPBEAT_STYLE, CALM_STYLE]), out: NoteEvent[] = []
    a.setState(state())
    for (let s = 0; s < BAR + 5; s++) a.nextStep(out)
    a.setState(state({ style: 1 }))
    let calls = 0
    while (a.styleIndex !== 1 && calls <= BAR) { a.nextStep(out); calls++ }
    expect(a.styleIndex).toBe(1)
    expect(calls).toBeLessThanOrEqual(BAR)
    expect((BAR + 5 + calls - 1) % BAR).toBe(0)
  })
})
