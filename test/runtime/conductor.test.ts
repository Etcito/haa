// Unit tests for the Conductor: spatial gains, listener-local plays, mix messages, unknown
// ids, voice stealing, Q5 variation (no immediate repeat, micro-variation bounds) and music.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SP } from '../../src/contract.ts';
import type { BedDef, BedMsg, Listener, MixMsg, SamplerMsg } from '../../src/contract.ts';
import { BASE_PRESET } from '../../src/mix/presets/base.ts';
import { MUSIC_PARAMS } from '../../src/music/musicSynth.ts';
import { registerBuiltins } from '../../src/registry/builtins.ts';
import { familyTrimDb, styleIndex } from '../../src/registry/registry.ts';
import { Conductor } from '../../src/runtime/conductor.ts';
import type { ConductorBackend } from '../../src/runtime/conductor.ts';

type BedStart = { handle: number; def: BedDef; seed: number; trimDb: number; values: Float32Array };

/** A backend that records every message it receives. */
function recorder() {
  const sampler: SamplerMsg[] = [];
  const mix: MixMsg[] = [];
  const beds: BedStart[] = [];
  const bed: { handle: number; m: BedMsg }[] = [];
  const backend: ConductorBackend = {
    sampler: (m) => { sampler.push(m); },
    mix: (m) => { mix.push(m); },
    startBed: (handle, def, seed, trimDb, values) => { beds.push({ handle, def, seed, trimDb, values }); return true; },
    bed: (handle, m) => { bed.push({ handle, m }); },
  };
  return { backend, sampler, mix, beds, bed };
}

const clock = { t: 0 };
function make(loaded: (id: string) => number = () => 8) {
  const rec = recorder();
  const c = new Conductor({
    preset: BASE_PRESET, maxListeners: 4, seed: 1, spatial: null, backend: rec.backend,
    now: () => clock.t, loaded, variantLength: () => 4800, loadRate: 48000,
  });
  return { c, ...rec };
}

const at = (x: number): Listener => ({ pos: { x, y: 0, z: 0 }, quat: { x: 0, y: 0, z: 0, w: 1 } });
const plays = (msgs: SamplerMsg[]) =>
  msgs.filter((m): m is Extract<SamplerMsg, { t: 'play' }> => m.t === 'play');
const FAMILY = 'impact.wood.small';

beforeEach(() => { registerBuiltins(); clock.t = 0; });
afterEach(() => { vi.restoreAllMocks(); });

describe('runtime/conductor spatial', () => {
  it('gives each of 2 listeners 0.5 with no spatial model', () => {
    const { c, sampler } = make();
    c.setListeners([at(0), at(10)]);
    expect(c.play(FAMILY)).not.toBeNull();
    const p = plays(sampler)[0];
    expect(p.sp[SP.COUNT]).toBe(2);
    expect(p.sp[SP.GAIN + 0]).toBeCloseTo(0.5, 6);
    expect(p.sp[SP.GAIN + 1]).toBeCloseTo(0.5, 6);
  });

  it('keeps only the addressed listener for a listener-local play', () => {
    const { c, sampler } = make();
    c.setListeners([at(0), at(10)]);
    c.play(FAMILY, { listener: 1 });
    const p = plays(sampler)[0];
    expect(p.sp[SP.GAIN + 1]).toBeCloseTo(0.5, 6);
    expect(p.sp[SP.GAIN + 0]).toBe(0);
  });
});

describe('runtime/conductor mix and admission', () => {
  it('posts a duck message', () => {
    const { c, mix } = make();
    c.duck('impact', 0.5);
    expect(mix).toEqual([{ t: 'duck', kind: 'impact', strength: 0.5 }]);
  });

  it('returns null for an unknown family and warns once', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { c, sampler } = make();
    expect(c.play('nope.missing')).toBeNull();
    expect(c.play('nope.missing')).toBeNull();
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn).toHaveBeenCalledWith('[haa] unknown family nope.missing');
    expect(sampler).toHaveLength(0);
  });

  it('drops a play of an unloaded family', () => {
    const { c, sampler } = make(() => 0);
    expect(c.play(FAMILY)).toBeNull();
    expect(c.stats.dropped).toBe(1);
    expect(sampler).toHaveLength(0);
  });

  it('steals a voice with a 30 ms fade for the 13th impact at higher gain', () => {
    const { c, sampler } = make();
    const voices = new Set<number>();
    for (let i = 0; i < 12; i++) voices.add(c.play(FAMILY, { gain: 0.1 })!.voice);
    expect(sampler.some((m) => m.t === 'stop')).toBe(false);
    const shot = c.play(FAMILY, { gain: 1 });
    expect(shot).not.toBeNull();
    const stops = sampler.filter((m) => m.t === 'stop');
    expect(stops).toHaveLength(1);
    const stop = stops[0] as Extract<SamplerMsg, { t: 'stop' }>;
    expect(stop.fade).toBe(0.03);
    expect(voices.has(stop.voice)).toBe(true);
    expect(c.stats.stolen).toBe(1);
  });
});

describe('runtime/conductor variation (Q5)', () => {
  it('never posts the same variant twice in a row over 100 plays', () => {
    const { c, sampler } = make();
    for (let i = 0; i < 100; i++) { clock.t = i; c.play(FAMILY); }
    const v = plays(sampler).map((p) => p.variant);
    expect(v).toHaveLength(100);
    for (let i = 1; i < v.length; i++) expect(v[i]).not.toBe(v[i - 1]);
    expect(new Set(v).size).toBeGreaterThan(1);
  });

  it('keeps gain within ±1 dB of the family trim and rate within ±3 %', () => {
    const { c, sampler } = make();
    const trim = 10 ** (familyTrimDb(BASE_PRESET, FAMILY) / 20);
    for (let i = 0; i < 100; i++) { clock.t = i; c.play(FAMILY); }
    for (const p of plays(sampler)) {
      const db = 20 * Math.log10(p.gain / trim);
      expect(Math.abs(db)).toBeLessThanOrEqual(1 + 1e-9);
      expect(Math.abs(p.rate - 1)).toBeLessThanOrEqual(0.03 + 1e-9);
    }
  });
});

describe('runtime/conductor music', () => {
  it('starts the music bed with the style index and crossfades with fadeBars', () => {
    const { c, beds, bed } = make();
    c.music.play('calm');
    expect(beds).toHaveLength(1);
    expect(beds[0].def.id).toBe('music');
    expect(beds[0].values[MUSIC_PARAMS.indexOf('style')]).toBe(styleIndex('calm'));
    expect(c.music.style).toBe('calm');
    c.music.crossfade('upbeat', 4);
    const p = bed.filter((b) => b.m.t === 'p').at(-1)!;
    expect(p.handle).toBe(beds[0].handle);
    const v = (p.m as Extract<BedMsg, { t: 'p' }>).v;
    expect(v).toBeInstanceOf(Float32Array);
    expect(v).toHaveLength(MUSIC_PARAMS.length);
    expect(v[MUSIC_PARAMS.indexOf('fadeBars')]).toBe(4);
    expect(v[MUSIC_PARAMS.indexOf('style')]).toBe(styleIndex('upbeat'));
    expect(c.music.style).toBe('upbeat');
  });
});
