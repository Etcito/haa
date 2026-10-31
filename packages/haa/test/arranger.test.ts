/**
 * Arrangement tests against the compiled wasm engine: the length formula, note onset, loop wrap
 * and the validation messages. Run with:
 *   node --test packages/haa/test/arranger.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { readFile } from 'node:fs/promises';

import type { HaaPatch } from '../src/types.ts';
import { renderArrangement, validateArrangement } from '../src/arranger.ts';
import type { Arrangement, Instrument } from '../src/arranger.ts';
import type { StereoRender } from '../src/offline.ts';

const module = await WebAssembly.compile(await readFile(new URL('../pkg/haa.wasm', import.meta.url)));

/** A minimal live patch publishing `note` (oscillator frequency) and `vel` (output gain). */
const PATCH: HaaPatch = {
  haa: 1,
  id: 'arranger-test-tone',
  title: 'Arranger test tone',
  inputs: [
    { name: 'note', kind: 'continuous', min: 0, max: 127, default: 60, smooth: 0 },
    { name: 'vel', kind: 'continuous', min: 0, max: 1, default: 0, smooth: 0 },
  ],
  master: 1,
  graph: {
    duration: 0.01,
    loop: false,
    mode: 'live',
    nodes: [
      { id: 'note', type: 'input', x: 40, y: 40, label: 'Note', params: { name: 'note' } },
      { id: 'vel', type: 'input', x: 40, y: 140, label: 'Velocity', params: { name: 'vel' } },
      { id: 'osc', type: 'oscillator', x: 240, y: 40, label: 'Osc', params: { frequency: 220, wave: 'sine', level: 0.5 } },
      { id: 'gain', type: 'gain', x: 440, y: 40, label: 'Gain', params: { gain: 0 } },
      { id: 'out', type: 'output', x: 640, y: 40, label: 'Output', params: { gain: 1 } },
    ],
    connections: [
      { id: 'note-frequency', from: 'note', to: 'osc', target: 'frequency', amount: 1 },
      { id: 'vel-gain', from: 'vel', to: 'gain', target: 'gain', amount: 1 },
      { id: 'osc-gain', from: 'osc', to: 'gain', target: 'audio', amount: 1 },
      { id: 'gain-out', from: 'gain', to: 'out', target: 'audio', amount: 1 },
    ],
  },
};

const INSTRUMENTS: Instrument[] = [
  { id: 'tone', patch: PATCH, noteInput: 'note', velocityInput: 'vel', releaseS: 0.1 },
];

/** A valid arrangement; each test overrides only the fields it exercises. */
function arrangement(overrides: Partial<Arrangement> = {}): Arrangement {
  return {
    format: 'haa.arrangement',
    version: 0,
    id: 'arranger-test',
    bpm: 120,
    beatsPerBar: 4,
    loop: false,
    seed: 1,
    instruments: [
      { id: 'tone', patch: 'audio/inst.tone.haa.json', noteInput: 'note', velocityInput: 'vel', releaseS: 0.1 },
    ],
    sections: [{ id: 'a', bars: 1, tracks: [] }],
    order: ['a'],
    ...overrides,
  };
}

/** RMS of both channels over frames [from, to) in dBFS, floored at -200. */
function rmsDbfs(r: StereoRender, from = 0, to = r.left.length): number {
  let sum = 0;
  let count = 0;
  for (let i = Math.max(0, from); i < Math.min(to, r.left.length); i++) {
    sum += r.left[i] * r.left[i] + r.right[i] * r.right[i];
    count += 2;
  }
  return count === 0 ? -200 : Math.max(-200, 10 * Math.log10(sum / count));
}

test('the output length is round(sum(bars·beatsPerBar·60/bpm)·rate)', () => {
  const a = arrangement({
    bpm: 100,
    beatsPerBar: 3,
    sections: [
      { id: 'a', bars: 2, tracks: [] },
      { id: 'b', bars: 3, tracks: [] },
    ],
    order: ['a', 'b', 'a'],
  });
  const seconds = (2 + 3 + 2) * 3 * (60 / 100);
  const at48 = renderArrangement(module, a, INSTRUMENTS);
  assert.equal(at48.sampleRate, 48_000);
  assert.equal(at48.left.length, Math.round(seconds * 48_000));
  assert.equal(at48.right.length, at48.left.length);

  const at22 = renderArrangement(module, a, INSTRUMENTS, { sampleRate: 22_050 });
  assert.equal(at22.sampleRate, 22_050);
  assert.equal(at22.left.length, Math.round(seconds * 22_050));
  assert.equal(at22.right.length, at22.left.length);
});

test('a note at beat 2 starts there with silence before it', () => {
  const a = arrangement({
    bpm: 120,
    sections: [{
      id: 'a',
      bars: 2,
      tracks: [{ instrument: 'tone', gain: 1, pan: 0, notes: [{ beat: 2, len: 1, note: 60, vel: 1 }] }],
    }],
  });
  const r = renderArrangement(module, a, INSTRUMENTS);
  const onset = Math.round(2 * (60 / 120) * 48_000);
  assert.ok(rmsDbfs(r, 0, onset) < -60, 'silence before the beat');
  assert.ok(rmsDbfs(r, onset, onset + 2_400) > -60, 'energy just after the beat');
});

test('loop wraps a release past the end onto the first frames and loop false truncates it', () => {
  const sections = [{
    id: 'a',
    bars: 1,
    tracks: [{ instrument: 'tone', gain: 1, pan: 0, notes: [{ beat: 3, len: 1, note: 60, vel: 1 }] }],
  }];
  const instruments: Instrument[] = [{ ...INSTRUMENTS[0], releaseS: 1 }];
  const looped = renderArrangement(module, arrangement({ loop: true, sections }), instruments);
  const truncated = renderArrangement(module, arrangement({ loop: false, sections }), instruments);
  const early = Math.round(0.5 * 48_000);
  assert.equal(looped.left.length, truncated.left.length);
  assert.ok(rmsDbfs(looped, 0, early) > -60, 'the release wraps to the head');
  assert.ok(rmsDbfs(truncated, 0, early) < -60, 'the release is cut at the end');
});

test('validation reports the exact message for an unknown instrument', () => {
  const a = arrangement({
    sections: [{ id: 'a', bars: 1, tracks: [{ instrument: 'ghost', gain: 1, pan: 0, notes: [] }] }],
  });
  assert.deepEqual(validateArrangement(a, INSTRUMENTS), ['section "a" track 0: unknown instrument "ghost"']);
});

test('validation reports the exact message for a bad order reference', () => {
  const a = arrangement({ order: ['a', 'missing'] });
  assert.deepEqual(validateArrangement(a, INSTRUMENTS), ['order names no section "missing"']);
});

test('validation reports the exact message for a note of 130 and renderArrangement throws it', () => {
  const a = arrangement({
    sections: [{
      id: 'a',
      bars: 1,
      tracks: [{ instrument: 'tone', gain: 1, pan: 0, notes: [{ beat: 0, len: 1, note: 130, vel: 1 }] }],
    }],
  });
  assert.deepEqual(validateArrangement(a, INSTRUMENTS), ['section "a" track 0 note 0: note 130 is outside 0..127']);
  assert.throws(
    () => renderArrangement(module, a, INSTRUMENTS),
    { message: '[haa] arrangement arranger-test: section "a" track 0 note 0: note 130 is outside 0..127' },
  );
});
