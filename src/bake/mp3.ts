// MP3 encoder used by bake() (bake module, node only): mono Float32 PCM at 48 kHz to a CBR
// 128 kbps MPEG-1 Layer III stream through lamejs 1.2.1, pinned and unmodified.
//
// lamejs 1.2.1's CommonJS sources use the free identifiers MPEGMode, Lame and BitStream at
// run time without requiring them, so a plain `new Mp3Encoder()` throws 'MPEGMode is not
// defined'. loadLame() sets those globals from lamejs's own src/js modules (only when they
// are undefined) before taking Mp3Encoder. It runs once, on the first encoder creation,
// never at import, and leaves the globals set because encodeBuffer and flush read them too.

import { createRequire } from 'node:module'

/** The subset of lamejs 1.2.1's Mp3Encoder that the bake uses. */
interface LameMp3Encoder {
  encodeBuffer(left: Int16Array, right?: Int16Array): Int8Array
  flush(): Int8Array
}

type LameMp3EncoderCtor = new (channels: number, rate: number, kbps: number) => LameMp3Encoder

/** Samples per MPEG-1 Layer III frame, the chunk size fed to encodeBuffer. */
const MP3_CHUNK = 1152

/** The globals lamejs 1.2.1 reads without requiring them. */
const LAME_GLOBALS = ['MPEGMode', 'Lame', 'BitStream'] as const

let mp3EncoderCtor: LameMp3EncoderCtor | null = null

/** Load lamejs once, defining its missing globals first. */
function loadLame(): LameMp3EncoderCtor {
  if (mp3EncoderCtor) return mp3EncoderCtor
  const require = createRequire(import.meta.url)
  const g = globalThis as Record<string, unknown>
  for (const name of LAME_GLOBALS) {
    if (g[name] === undefined) g[name] = require(`lamejs/src/js/${name}.js`)
  }
  const lamejs = require('lamejs') as { Mp3Encoder: LameMp3EncoderCtor }
  mp3EncoderCtor = lamejs.Mp3Encoder
  return mp3EncoderCtor
}

/** Float32 to Int16: scaled by 32767, rounded half away from zero, clamped to ±32767. */
function toInt16(pcm: Float32Array, start: number, out: Int16Array): void {
  for (let i = 0; i < out.length; i++) {
    const x = pcm[start + i] * 32767
    const r = x < 0 ? -Math.round(-x) : Math.round(x)
    out[i] = r > 32767 ? 32767 : r < -32767 ? -32767 : r || 0
  }
}

/**
 * Encode mono PCM to MP3 with lamejs 1.2.1: Mp3Encoder(1, rate, kbps), 1152-sample Int16
 * chunks, then flush, concatenated into one Uint8Array. Deterministic: the same PCM gives
 * the same bytes in every run and process.
 */
export function encodeMp3(pcm: Float32Array, rate: 48000, kbps: 128): Uint8Array {
  const Mp3Encoder = loadLame()
  const encoder = new Mp3Encoder(1, rate, kbps)
  const parts: Int8Array[] = []
  let total = 0
  const push = (part: Int8Array): void => {
    if (part.length === 0) return
    parts.push(part)
    total += part.length
  }
  const chunk = new Int16Array(MP3_CHUNK)
  for (let start = 0; start < pcm.length; start += MP3_CHUNK) {
    const n = Math.min(MP3_CHUNK, pcm.length - start)
    const block = n === MP3_CHUNK ? chunk : new Int16Array(n)
    toInt16(pcm, start, block)
    push(encoder.encodeBuffer(block))
  }
  push(encoder.flush())
  const out = new Uint8Array(total)
  let offset = 0
  for (const part of parts) {
    out.set(new Uint8Array(part.buffer, part.byteOffset, part.length), offset)
    offset += part.length
  }
  return out
}
