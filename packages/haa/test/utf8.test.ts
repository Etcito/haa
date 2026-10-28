/**
 * Host test for the pure UTF-8 codec.
 *
 * Runs with Node's native type stripping:
 *   node --test packages/haa/test/utf8.test.ts
 * Buffer is used here only as the test oracle; the codec itself uses no
 * platform global.
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import { decodeUtf8, encodeUtf8 } from '../src/utf8.ts';

const SAMPLES: string[] = [
  '',
  'HAA engine 0.2.0',
  'ASCII only: abcXYZ 0123 !?',
  'é',
  'café déjà vu',
  '音',
  '音声エンジン',
  '🔊',
  'mix: ASCII é 音 🔊',
];

test('round trip ASCII, accents, CJK and emoji', () => {
  for (const sample of SAMPLES) {
    assert.equal(decodeUtf8(encodeUtf8(sample)), sample, sample);
  }
});

test('stress round trip across the decode chunk boundary', () => {
  const long = '音🔊é'.repeat(3000);
  assert.equal(decodeUtf8(encodeUtf8(long)), long);
});

test('encodes é as C3 A9', () => {
  assert.deepEqual(Array.from(encodeUtf8('é')), [0xc3, 0xa9]);
});

test('encodes CJK 音 as three bytes', () => {
  const bytes = encodeUtf8('音');
  assert.equal(bytes.length, 3);
  assert.deepEqual(Array.from(bytes), [0xe9, 0x9f, 0xb3]);
});

test('encodes the speaker emoji as four bytes', () => {
  const bytes = encodeUtf8('🔊');
  assert.equal(bytes.length, 4);
  assert.deepEqual(Array.from(bytes), [0xf0, 0x9f, 0x94, 0x8a]);
});

test('matches Buffer UTF-8 encoding', () => {
  for (const sample of SAMPLES) {
    assert.deepEqual(
      encodeUtf8(sample),
      new Uint8Array(Buffer.from(sample, 'utf8')),
      sample,
    );
  }
});

test('decodes Buffer UTF-8 bytes', () => {
  for (const sample of SAMPLES) {
    assert.equal(decodeUtf8(new Uint8Array(Buffer.from(sample, 'utf8'))), sample);
  }
});

test('a lone surrogate becomes U+FFFD', () => {
  assert.deepEqual(Array.from(encodeUtf8('\ud800')), [0xef, 0xbf, 0xbd]);
  assert.deepEqual(Array.from(encodeUtf8('\udc00')), [0xef, 0xbf, 0xbd]);
  assert.equal(decodeUtf8(encodeUtf8('a\ud800b')), 'a\ufffdb');
});

test('malformed and truncated sequences become U+FFFD', () => {
  assert.equal(decodeUtf8(new Uint8Array([0xff])), '\ufffd');
  assert.equal(decodeUtf8(new Uint8Array([0xc3])), '\ufffd');
  assert.equal(decodeUtf8(new Uint8Array([0xe9, 0x9f])), '\ufffd');
  assert.equal(decodeUtf8(new Uint8Array([0xf0, 0x9f, 0x94])), '\ufffd');
  assert.equal(decodeUtf8(new Uint8Array([0xe9, 0x9f, 0x41])), '\ufffdA');
  assert.equal(
    decodeUtf8(new Uint8Array([0x61, 0xc3, 0x28, 0x62])),
    'a\ufffd(b',
  );
  assert.equal(decodeUtf8(new Uint8Array([0xc0, 0xaf])), '\ufffd\ufffd');
  assert.equal(
    decodeUtf8(new Uint8Array([0xed, 0xa0, 0x80])),
    '\ufffd\ufffd\ufffd',
  );
});
