// Host test for the realm-neutral WAV writer (ARCHITECTURE §16).

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { encodeWav } from '../src/wav.ts';

const RATE = 48_000;

function view(bytes: Uint8Array): DataView {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

function ascii(bytes: Uint8Array, at: number, length: number): string {
  let text = '';
  for (let i = 0; i < length; i += 1) {
    text += String.fromCharCode(bytes[at + i]);
  }
  return text;
}

test('header is a 16-bit stereo riff at the given rate', () => {
  const frames = 3;
  const zeros = new Float32Array(frames);
  const wav = encodeWav({ left: zeros, right: zeros, sampleRate: RATE });
  const data = view(wav);

  assert.equal(ascii(wav, 0, 4), 'RIFF');
  assert.equal(data.getUint32(4, true), 36 + frames * 4);
  assert.equal(ascii(wav, 8, 4), 'WAVE');
  assert.equal(ascii(wav, 12, 4), 'fmt ');
  assert.equal(data.getUint32(16, true), 16);
  assert.equal(data.getUint16(20, true), 1);
  assert.equal(data.getUint16(22, true), 2);
  assert.equal(data.getUint32(24, true), RATE);
  assert.equal(data.getUint32(28, true), RATE * 4);
  assert.equal(data.getUint16(32, true), 4);
  assert.equal(data.getUint16(34, true), 16);
  assert.equal(ascii(wav, 36, 4), 'data');
  assert.equal(data.getUint32(40, true), frames * 4);
  assert.equal(wav.length, 44 + frames * 4);
});

test('frame count follows the left channel and a missing right sample is zero', () => {
  const left = new Float32Array([0, 0, 0]);
  const right = new Float32Array([0.5]);
  const wav = encodeWav({ left, right, sampleRate: RATE });
  const data = view(wav);

  assert.equal(data.getUint32(40, true), 3 * 4);
  assert.equal(wav.length, 44 + 3 * 4);
  assert.equal(data.getInt16(44, true), 0);
  assert.equal(data.getInt16(46, true), 16384);
  assert.equal(data.getInt16(48, true), 0);
  assert.equal(data.getInt16(50, true), 0);
  assert.equal(data.getInt16(52, true), 0);
  assert.equal(data.getInt16(54, true), 0);
});

test('samples clamp and round half away from zero', () => {
  const left = new Float32Array([
    1,
    -1,
    2,
    0.5 / 32767,
    -0.5 / 32767,
    Number.NaN,
  ]);
  const wav = encodeWav({
    left,
    right: new Float32Array(left.length),
    sampleRate: RATE,
  });
  const data = view(wav);
  const sample = (i: number): number => data.getInt16(44 + i * 4, true);

  assert.equal(sample(0), 32767);
  assert.equal(sample(1), -32767);
  assert.equal(sample(2), 32767);
  assert.equal(sample(3), 1);
  assert.equal(sample(4), -1);
  assert.equal(sample(5), 0);
});
