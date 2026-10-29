/**
 * Pure UTF-8 codec for the HAA host.
 *
 * This module is realm-neutral: it uses no TextEncoder/TextDecoder or any
 * other global, because it also runs inside AudioWorkletGlobalScope, where
 * those APIs are absent. Text crosses the wasm boundary only as UTF-8 bytes
 * (ARCHITECTURE §13), so the worklet encodes and decodes it here.
 */

const REPLACEMENT_CODE_POINT = 0xfffd;

const HIGH_SURROGATE_MIN = 0xd800;
const HIGH_SURROGATE_MAX = 0xdbff;
const LOW_SURROGATE_MIN = 0xdc00;
const LOW_SURROGATE_MAX = 0xdfff;

/** Last code point that fits in a UTF-16 code unit. */
const BMP_MAX = 0xffff;

/** Maximum UTF-16 code units handed to one String.fromCharCode call. */
const DECODE_CHUNK_UNITS = 4096;

/**
 * Encodes a JavaScript string as UTF-8.
 *
 * Walks the UTF-16 code units, combines a high/low surrogate pair into one
 * code point, and writes 1–4 bytes for it. A lone surrogate becomes U+FFFD.
 * A first pass sizes the output, then a single Uint8Array is filled.
 */
export function encodeUtf8(s: string): Uint8Array {
  let byteLength = 0;
  for (let i = 0; i < s.length; i++) {
    const code = s.charCodeAt(i);
    if (code >= HIGH_SURROGATE_MIN && code <= HIGH_SURROGATE_MAX) {
      const next = i + 1 < s.length ? s.charCodeAt(i + 1) : 0;
      if (next >= LOW_SURROGATE_MIN && next <= LOW_SURROGATE_MAX) {
        byteLength += 4;
        i++;
      } else {
        byteLength += 3;
      }
    } else if (code >= LOW_SURROGATE_MIN && code <= LOW_SURROGATE_MAX) {
      byteLength += 3;
    } else if (code < 0x80) {
      byteLength += 1;
    } else if (code < 0x800) {
      byteLength += 2;
    } else {
      byteLength += 3;
    }
  }

  const out = new Uint8Array(byteLength);
  let p = 0;
  for (let i = 0; i < s.length; i++) {
    let code = s.charCodeAt(i);
    if (code >= HIGH_SURROGATE_MIN && code <= HIGH_SURROGATE_MAX) {
      const next = i + 1 < s.length ? s.charCodeAt(i + 1) : 0;
      if (next >= LOW_SURROGATE_MIN && next <= LOW_SURROGATE_MAX) {
        code =
          0x10000 +
          ((code - HIGH_SURROGATE_MIN) << 10) +
          (next - LOW_SURROGATE_MIN);
        i++;
      } else {
        code = REPLACEMENT_CODE_POINT;
      }
    } else if (code >= LOW_SURROGATE_MIN && code <= LOW_SURROGATE_MAX) {
      code = REPLACEMENT_CODE_POINT;
    }

    if (code < 0x80) {
      out[p++] = code;
    } else if (code < 0x800) {
      out[p++] = 0xc0 | (code >> 6);
      out[p++] = 0x80 | (code & 0x3f);
    } else if (code < 0x10000) {
      out[p++] = 0xe0 | (code >> 12);
      out[p++] = 0x80 | ((code >> 6) & 0x3f);
      out[p++] = 0x80 | (code & 0x3f);
    } else {
      out[p++] = 0xf0 | (code >> 18);
      out[p++] = 0x80 | ((code >> 12) & 0x3f);
      out[p++] = 0x80 | ((code >> 6) & 0x3f);
      out[p++] = 0x80 | (code & 0x3f);
    }
  }

  return out;
}

/**
 * Decodes UTF-8 bytes into a JavaScript string.
 *
 * Handles 1–4 byte sequences. A malformed or truncated sequence yields
 * U+FFFD (one per offending lead byte), and overlong forms, surrogates and
 * code points above U+10FFFF are rejected the same way. Code points above
 * 0xFFFF are written as surrogate pairs. The result is assembled in chunks of
 * at most 4096 code units through String.fromCharCode.
 */
export function decodeUtf8(b: Uint8Array): string {
  const chunks: string[] = [];
  let units: number[] = [];

  const push = (unit: number): void => {
    units.push(unit);
    if (units.length >= DECODE_CHUNK_UNITS) {
      chunks.push(String.fromCharCode(...units));
      units = [];
    }
  };

  let i = 0;
  while (i < b.length) {
    const lead = b[i];
    if (lead < 0x80) {
      push(lead);
      i += 1;
      continue;
    }

    let remaining = 0;
    let code = 0;
    let lower = 0x80;
    let upper = 0xbf;
    if (lead >= 0xc2 && lead <= 0xdf) {
      remaining = 1;
      code = lead & 0x1f;
    } else if (lead >= 0xe0 && lead <= 0xef) {
      remaining = 2;
      code = lead & 0x0f;
      if (lead === 0xe0) lower = 0xa0;
      else if (lead === 0xed) upper = 0x9f;
    } else if (lead >= 0xf0 && lead <= 0xf4) {
      remaining = 3;
      code = lead & 0x07;
      if (lead === 0xf0) lower = 0x90;
      else if (lead === 0xf4) upper = 0x8f;
    }

    if (remaining === 0) {
      push(REPLACEMENT_CODE_POINT);
      i += 1;
      continue;
    }

    let consumed = 1;
    let valid = true;
    for (let k = 0; k < remaining; k++) {
      const index = i + 1 + k;
      if (index >= b.length) {
        valid = false;
        break;
      }
      const byte = b[index];
      if (byte < lower || byte > upper) {
        valid = false;
        break;
      }
      code = (code << 6) | (byte & 0x3f);
      lower = 0x80;
      upper = 0xbf;
      consumed += 1;
    }

    if (!valid) {
      push(REPLACEMENT_CODE_POINT);
      i += consumed;
      continue;
    }

    if (code > BMP_MAX) {
      const value = code - 0x10000;
      push(HIGH_SURROGATE_MIN + (value >> 10));
      push(LOW_SURROGATE_MIN + (value & 0x3ff));
    } else {
      push(code);
    }
    i += consumed;
  }

  if (units.length > 0) {
    chunks.push(String.fromCharCode(...units));
  }
  return chunks.join('');
}
