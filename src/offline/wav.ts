// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports, decodeWav added
// Offline WAV writer and reader (module offline).
//
// Pure and dependency-free: it turns floating point render buffers into a
// little-endian RIFF/WAVE 16-bit PCM file and back. Node, the browser main thread and
// the proof tooling all use it. No imports, no DOM, no Web Audio.

const WAV_HEADER_BYTES = 44;
const PCM_FORMAT = 1;
const BITS_PER_SAMPLE = 16;
const BYTES_PER_SAMPLE = 2;

/** Clamp to [-1, 1], scale by 32767 and round half away from zero. */
function toPcm16(x: number): number {
  let v = x;
  if (v < -1) v = -1;
  else if (v > 1) v = 1;
  const scaled = v * 32767;
  return scaled < 0 ? -Math.round(-scaled) : Math.round(scaled);
}

/**
 * Encode floating point samples as a RIFF/WAVE PCM 16-bit file.
 *
 * - `right === null` writes a mono file, otherwise a stereo one (L, R, ...).
 * - `length` is the per-channel sample count; it defaults to `left.length`.
 */
export function encodeWav(
  left: Float32Array,
  right: Float32Array | null,
  sampleRate: number,
  length?: number,
): Uint8Array {
  const channels = right === null ? 1 : 2;
  const count = length === undefined ? left.length : length;

  const dataBytes = count * channels * BYTES_PER_SAMPLE;
  const out = new Uint8Array(WAV_HEADER_BYTES + dataBytes);
  const view = new DataView(out.buffer);

  // RIFF chunk descriptor.
  writeAscii(out, 0, 'RIFF');
  view.setUint32(4, 36 + dataBytes, true);
  writeAscii(out, 8, 'WAVE');

  // fmt subchunk.
  writeAscii(out, 12, 'fmt ');
  view.setUint32(16, 16, true); // subchunk size
  view.setUint16(20, PCM_FORMAT, true);
  view.setUint16(22, channels, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * channels * BYTES_PER_SAMPLE, true); // byte rate
  view.setUint16(32, channels * BYTES_PER_SAMPLE, true); // block align
  view.setUint16(34, BITS_PER_SAMPLE, true);

  // data subchunk, interleaved little-endian samples.
  writeAscii(out, 36, 'data');
  view.setUint32(40, dataBytes, true);

  if (channels === 1) {
    for (let i = 0; i < count; i++) {
      view.setInt16(WAV_HEADER_BYTES + i * BYTES_PER_SAMPLE, toPcm16(left[i]), true);
    }
  } else {
    const r = right as Float32Array;
    let offset = WAV_HEADER_BYTES;
    for (let i = 0; i < count; i++) {
      view.setInt16(offset, toPcm16(left[i]), true);
      view.setInt16(offset + BYTES_PER_SAMPLE, toPcm16(r[i]), true);
      offset += 2 * BYTES_PER_SAMPLE;
    }
  }

  return out;
}

function writeAscii(out: Uint8Array, offset: number, text: string): void {
  for (let i = 0; i < text.length; i++) {
    out[offset + i] = text.charCodeAt(i) & 0xff;
  }
}

function readAscii(bytes: Uint8Array, offset: number): string {
  return String.fromCharCode(bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]);
}

function badWav(): Error {
  return new Error('[haa] bad wav');
}

/**
 * Decode a RIFF/WAVE PCM 16-bit file into per-channel floats (sample / 32767).
 *
 * - Unknown chunks are skipped (word-aligned).
 * - Anything else than PCM format 1 at 16 bits, or a truncated file, throws `[haa] bad wav`.
 */
export function decodeWav(bytes: Uint8Array): { sampleRate: number; channels: Float32Array[] } {
  if (bytes.length < 12 || readAscii(bytes, 0) !== 'RIFF' || readAscii(bytes, 8) !== 'WAVE') throw badWav();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);

  let channelCount = 0;
  let sampleRate = 0;
  let offset = 12;
  while (offset + 8 <= bytes.length) {
    const id = readAscii(bytes, offset);
    const size = view.getUint32(offset + 4, true);
    const body = offset + 8;
    if (body + size > bytes.length) throw badWav();

    if (id === 'fmt ') {
      if (size < 16) throw badWav();
      const format = view.getUint16(body, true);
      channelCount = view.getUint16(body + 2, true);
      sampleRate = view.getUint32(body + 4, true);
      const bits = view.getUint16(body + 14, true);
      if (format !== PCM_FORMAT || bits !== BITS_PER_SAMPLE || channelCount < 1 || sampleRate < 1) throw badWav();
    } else if (id === 'data') {
      if (channelCount === 0) throw badWav();
      const frames = Math.floor(size / (channelCount * BYTES_PER_SAMPLE));
      const channels: Float32Array[] = [];
      for (let c = 0; c < channelCount; c++) channels.push(new Float32Array(frames));
      let p = body;
      for (let i = 0; i < frames; i++) {
        for (let c = 0; c < channelCount; c++) {
          channels[c][i] = view.getInt16(p, true) / 32767;
          p += BYTES_PER_SAMPLE;
        }
      }
      return { sampleRate, channels };
    }
    offset = body + size + (size & 1);
  }
  throw badWav();
}
