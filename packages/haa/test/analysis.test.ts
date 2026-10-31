/**
 * Synthetic-buffer analysis tests, without an engine or audio device.
 * Run with: node --test packages/haa/test/analysis.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import type { StereoRender } from '../src/offline.ts';
import { analyse } from '../src/analysis.ts';
import { spectrogram } from '../src/spectrum.ts';

/** Build a synthetic stereo render at 48 kHz. */
function render(left: Float32Array, right = left.slice()): StereoRender {
  return { left, right, sampleRate: 48_000, issues: [] };
}

/** Generate a sine with configurable amplitude, frequency and frame count. */
function sine(amplitude: number, hz = 1000, frames = 4800): StereoRender {
  return render(Float32Array.from({ length: frames }, (_, i) => amplitude * Math.sin(2 * Math.PI * hz * i / 48_000)));
}

test('silence is silent with finite peak and RMS floors', () => {
  const stats = analyse(render(new Float32Array(1024)));
  assert.equal(stats.frames, 1024);
  assert.equal(stats.sampleRate, 48_000);
  assert.equal(stats.silent, true);
  assert.equal(stats.clipped, false);
  assert.equal(stats.peakDbfs, -200);
  assert.equal(stats.rmsDbfs, -200);
  assert.equal(stats.dc, 0);
  assert.equal(stats.nonFinite, 0);
});

test('a 1.5 amplitude sine clips', () => {
  const stats = analyse(sine(1.5));
  assert.equal(stats.clipped, true);
  assert.equal(stats.silent, false);
  assert.ok(stats.peakDbfs > 0);
});

test('non-finite samples are counted in both channels and excluded from levels', () => {
  const stats = analyse(render(
    new Float32Array([NaN, 0.5, Infinity, -0.5]),
    new Float32Array([0.5, -Infinity, NaN, -0.5]),
  ));
  assert.equal(stats.nonFinite, 4);
  assert.ok(Math.abs(stats.peakDbfs + 6.0206) < 0.1);
  assert.ok(Math.abs(stats.rmsDbfs + 6.0206) < 0.1);
  assert.ok(Number.isFinite(stats.dc));
});

test('a half-scale 1 kHz sine peaks at -6.02 dBFS', () => {
  const stats = analyse(sine(0.5));
  assert.ok(Math.abs(stats.peakDbfs + 6.02) < 0.1);
  assert.ok(Math.abs(stats.rmsDbfs + 9.03) < 0.1);
  assert.ok(Math.abs(stats.dc) < 1e-6);
});

test('stereo levels include the right channel and DC is the mean mono mix', () => {
  const stats = analyse(render(new Float32Array([0, 0]), new Float32Array([1, 0.5])));
  assert.equal(stats.peakDbfs, 0);
  assert.equal(stats.clipped, false);
  assert.equal(stats.dc, 0.375);
  assert.ok(Math.abs(stats.rmsDbfs - 10 * Math.log10(1.25 / 4)) < 1e-10);
});

test('empty and entirely non-finite renders have finite silence measurements', () => {
  for (const r of [render(new Float32Array()), render(new Float32Array([NaN, Infinity]))]) {
    const stats = analyse(r);
    assert.equal(stats.peakDbfs, -200);
    assert.equal(stats.rmsDbfs, -200);
    assert.equal(stats.dc, 0);
    assert.equal(stats.silent, true);
  }
});

test('every spectrogram frame peaks at the 1 kHz bin', () => {
  const r = sine(1);
  const spectrum = spectrogram(r);
  const expected = Math.round(1000 / (48_000 / 1024));
  assert.equal(spectrum.bins, 512);
  assert.equal(spectrum.frames, Math.floor((r.left.length - 1024) / 256) + 1);
  assert.equal(spectrum.hopS, 256 / 48_000);
  assert.equal(spectrum.maxHz, 24_000);
  assert.equal(spectrum.db.length, spectrum.frames * spectrum.bins);
  for (let frame = 0; frame < spectrum.frames; frame++) {
    const offset = frame * spectrum.bins;
    let peakBin = 0;
    for (let bin = 1; bin < spectrum.bins; bin++) {
      if (spectrum.db[offset + bin] > spectrum.db[offset + peakBin]) peakBin = bin;
    }
    assert.ok(Math.abs(peakBin - expected) <= 1, `frame ${frame}: peak bin ${peakBin}`);
    assert.ok(Math.abs(spectrum.db[offset + peakBin]) < 1);
  }
});

test('a bin-centred full-scale sine peaks at 0 dB with custom FFT and hop', () => {
  const spectrum = spectrogram(sine(1, 1500, 1024), { fft: 256, hop: 128 });
  assert.equal(spectrum.bins, 128);
  assert.equal(spectrum.frames, 7);
  assert.equal(spectrum.hopS, 128 / 48_000);
  for (let frame = 0; frame < spectrum.frames; frame++) {
    assert.ok(Math.abs(spectrum.db[frame * spectrum.bins + 8]) < 0.01);
  }
});

test('short and empty buffers produce one zero-padded frame at the silence floor', () => {
  for (const frames of [0, 10, 1024]) {
    const spectrum = spectrogram(render(new Float32Array(frames)));
    assert.equal(spectrum.frames, 1);
    assert.equal(spectrum.db.length, 512);
    assert.ok(spectrum.db.every((value) => value === -90));
  }
});

test('spectrogram mixes stereo channels before transforming', () => {
  const r = sine(1);
  r.right = r.left.map((value) => -value);
  assert.ok(spectrogram(r).db.every((value) => value === -90));
});

test('spectrogram rejects invalid FFT sizes with the value in its error', () => {
  const r = render(new Float32Array());
  for (const fft of [0, -2, 3, 1000, 2.5, NaN, Infinity]) {
    assert.throws(() => spectrogram(r, { fft }), (error: unknown) => {
      return error instanceof Error && error.message.includes(String(fft));
    });
  }
});

test('spectrogram rejects non-positive or fractional hops', () => {
  const r = render(new Float32Array());
  for (const hop of [0, -1, 0.5, NaN, Infinity]) {
    assert.throws(() => spectrogram(r, { hop }), /hop/);
  }
});
