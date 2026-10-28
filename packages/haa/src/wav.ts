// WAV writer (ARCHITECTURE §16): RIFF PCM 16-bit stereo, samples scaled by 32767,
// rounded half away from zero and clamped. Realm-neutral; uses no Node builtins.

const HEADER_BYTES = 44;
const BYTES_PER_SAMPLE = 2;
const CHANNELS = 2;
const BITS_PER_SAMPLE = 16;
const PCM_FORMAT = 1;

function writeTag(bytes: Uint8Array, at: number, tag: string): void {
  for (let i = 0; i < tag.length; i += 1) {
    bytes[at + i] = tag.charCodeAt(i);
  }
}

/** Scale one sample to the signed 16-bit range; non-finite samples become silence. */
function toPcm16(sample: number): number {
  if (!Number.isFinite(sample)) {
    return 0;
  }
  // Like the Rust writer, the product stays in f32, so a sample authored as
  // `0.5 / 32767` lands exactly on a tie.
  const scaled = Math.fround(Math.min(1, Math.max(-1, sample)) * 32767);
  return Math.sign(scaled) * Math.round(Math.abs(scaled));
}

/** Encode interleaved stereo PCM into a RIFF/WAVE byte buffer (§16). */
export function encodeWav(r: {
  left: Float32Array;
  right: Float32Array;
  sampleRate: number;
}): Uint8Array {
  const frames = r.left.length;
  const dataSize = frames * CHANNELS * BYTES_PER_SAMPLE;
  const bytes = new Uint8Array(HEADER_BYTES + dataSize);
  const view = new DataView(bytes.buffer);

  writeTag(bytes, 0, 'RIFF');
  view.setUint32(4, HEADER_BYTES - 8 + dataSize, true);
  writeTag(bytes, 8, 'WAVE');

  writeTag(bytes, 12, 'fmt ');
  view.setUint32(16, 16, true);
  view.setUint16(20, PCM_FORMAT, true);
  view.setUint16(22, CHANNELS, true);
  view.setUint32(24, r.sampleRate, true);
  view.setUint32(
    28,
    r.sampleRate * CHANNELS * BYTES_PER_SAMPLE,
    true,
  );
  view.setUint16(32, CHANNELS * BYTES_PER_SAMPLE, true);
  view.setUint16(34, BITS_PER_SAMPLE, true);

  writeTag(bytes, 36, 'data');
  view.setUint32(40, dataSize, true);

  for (let i = 0; i < frames; i += 1) {
    const right = i < r.right.length ? r.right[i] : 0;
    view.setInt16(HEADER_BYTES + i * 4, toPcm16(r.left[i]), true);
    view.setInt16(HEADER_BYTES + i * 4 + 2, toPcm16(right), true);
  }

  return bytes;
}
