// Unit tests for the WAV codec: round trips, header fields and rejection of bad input.
import { describe, expect, it } from 'vitest';
import { AudioRng } from '../../src/dsp/rng.ts';
import { decodeWav, encodeWav } from '../../src/offline/wav.ts';

function noise(seed: number, n: number): Float32Array {
  const rng = new AudioRng(seed);
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) out[i] = rng.next() * 2 - 1;
  return out;
}

function maxError(a: Float32Array, b: Float32Array): number {
  let worst = 0;
  for (let i = 0; i < a.length; i++) worst = Math.max(worst, Math.abs(a[i] - b[i]));
  return worst;
}

function ascii(bytes: Uint8Array, offset: number): string {
  return String.fromCharCode(...bytes.subarray(offset, offset + 4));
}

describe('offline/wav', () => {
  it('round trips stereo samples within 1/32767', () => {
    const left = noise(1, 1000);
    const right = noise(2, 1000);
    const d = decodeWav(encodeWav(left, right, 48000));
    expect(d.sampleRate).toBe(48000);
    expect(d.channels.length).toBe(2);
    expect(d.channels[0].length).toBe(1000);
    expect(maxError(left, d.channels[0])).toBeLessThanOrEqual(1 / 32767);
    expect(maxError(right, d.channels[1])).toBeLessThanOrEqual(1 / 32767);
  });

  it('round trips mono samples within 1/32767', () => {
    const left = noise(3, 777);
    const d = decodeWav(encodeWav(left, null, 48000));
    expect(d.channels.length).toBe(1);
    expect(d.channels[0].length).toBe(777);
    expect(maxError(left, d.channels[0])).toBeLessThanOrEqual(1 / 32767);
  });

  it('writes the RIFF header fields', () => {
    const bytes = encodeWav(noise(4, 100), noise(5, 100), 48000);
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    expect(ascii(bytes, 0)).toBe('RIFF');
    expect(ascii(bytes, 8)).toBe('WAVE');
    expect(view.getUint16(20, true)).toBe(1);
    expect(view.getUint16(22, true)).toBe(2);
    expect(view.getUint32(24, true)).toBe(48000);
    expect(view.getUint16(34, true)).toBe(16);
    expect(ascii(bytes, 36)).toBe('data');
    expect(view.getUint32(40, true)).toBe(100 * 2 * 2);
    expect(bytes.length).toBe(44 + 400);
    const mono = encodeWav(noise(6, 50), null, 48000);
    expect(new DataView(mono.buffer).getUint16(22, true)).toBe(1);
    expect(new DataView(mono.buffer).getUint32(40, true)).toBe(100);
  });

  it('clamps out-of-range samples and rounds symmetrically around zero', () => {
    const d = decodeWav(encodeWav(new Float32Array([2, -2, 1, -1]), null, 48000));
    expect(Array.from(d.channels[0], (x) => Math.round(x * 32767))).toEqual([32767, -32767, 32767, -32767]);
    const x = noise(9, 500);
    const neg = x.map((v) => -v);
    const a = decodeWav(encodeWav(x, null, 48000)).channels[0];
    const b = decodeWav(encodeWav(neg, null, 48000)).channels[0];
    for (let i = 0; i < a.length; i++) expect(b[i]).toBe(-a[i]);
  });

  it('skips unknown chunks', () => {
    const wav = encodeWav(noise(7, 10), null, 48000);
    const extra = new Uint8Array([0x4c, 0x49, 0x53, 0x54, 3, 0, 0, 0, 1, 2, 3, 0]); // 'LIST', odd size + pad
    const bytes = new Uint8Array(wav.length + extra.length);
    bytes.set(wav.subarray(0, 36));
    bytes.set(extra, 36);
    bytes.set(wav.subarray(36), 36 + extra.length);
    expect(decodeWav(bytes).channels[0]).toEqual(decodeWav(wav).channels[0]);
  });

  it('throws on garbage bytes', () => {
    expect(() => decodeWav(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]))).toThrow('[haa] bad wav');
    expect(() => decodeWav(new Uint8Array(0))).toThrow('[haa] bad wav');
    const float = encodeWav(noise(8, 4), null, 48000);
    new DataView(float.buffer).setUint16(20, 3, true);
    expect(() => decodeWav(float)).toThrow('[haa] bad wav');
  });
});
