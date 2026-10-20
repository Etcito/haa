// Offline render helpers (module offline). DSP-safe: no DOM, no Web Audio, no node builtins.
//
// - renderOffline drives a block callback over preallocated, zeroed BLOCK-sized buffers and
//   collects a StereoRender of round(seconds × sr) frames (the last block is partial).
// - renderFamilyStrip lays the normalized variants of one family end to end, centered.
// - renderBedDemo runs one BedVoice under a parameter script and returns its raw output.

import { AUDIO_RATE, BLOCK } from '../contract.ts';
import type { BedDef, FamilyDef, MixPreset, StereoRender } from '../contract.ts';
import { renderFamilyVariants } from '../bank/render.ts';

const DEFAULT_GAP_SEC = 0.25;

/**
 * Render `seconds` of stereo audio by calling `fn(l, r, n, frame)` once per block of at most
 * BLOCK frames. `l` and `r` are zeroed before every call (fn ADDS into them); `frame` is the
 * index of the block's first frame. The output has round(seconds × sampleRate) frames.
 */
export function renderOffline(
  fn: (l: Float32Array, r: Float32Array, n: number, frame: number) => void,
  seconds: number,
  sampleRate: number = AUDIO_RATE,
  name = 'render',
): StereoRender {
  const total = Number.isFinite(seconds) && seconds > 0 ? Math.round(seconds * sampleRate) : 0;
  const left = new Float32Array(total);
  const right = new Float32Array(total);
  const l = new Float32Array(BLOCK);
  const r = new Float32Array(BLOCK);
  for (let frame = 0; frame < total; frame += BLOCK) {
    const n = Math.min(BLOCK, total - frame);
    l.fill(0);
    r.fill(0);
    fn(l, r, n, frame);
    for (let i = 0; i < n; i++) {
      left[frame + i] = l[i];
      right[frame + i] = r[i];
    }
  }
  return { name, left, right, sampleRate };
}

/**
 * The normalized variants of one family (renderFamilyVariants with the preset's category
 * target) in order, separated by gapSec (default 0.25 s) of silence, placed centered:
 * × √½ on both channels, so L === R.
 */
export function renderFamilyStrip(
  def: FamilyDef,
  preset: MixPreset,
  o: { gapSec?: number; sampleRate?: number } = {},
): StereoRender {
  const sr = o.sampleRate ?? AUDIO_RATE;
  const gap = Math.max(0, Math.round((o.gapSec ?? DEFAULT_GAP_SEC) * sr));
  const variants = renderFamilyVariants(def, sr, preset);
  let total = 0;
  for (let v = 0; v < variants.length; v++) total += variants[v].length + (v > 0 ? gap : 0);
  const left = new Float32Array(total);
  const right = new Float32Array(total);
  let at = 0;
  for (let v = 0; v < variants.length; v++) {
    if (v > 0) at += gap;
    const x = variants[v];
    for (let i = 0; i < x.length; i++) {
      const y = x[i] * Math.SQRT1_2;
      left[at + i] = y;
      right[at + i] = y;
    }
    at += x.length;
  }
  return { name: def.id, left, right, sampleRate: sr };
}

/**
 * Run one BedVoice from def.create(sr, seed) for `seconds`. The params p start as a copy of
 * def.defaults; before every block, script(t, p) runs with t = the block start in seconds,
 * then the voice processes p at offset 0. Returns the raw bed output (no mix, no trim).
 */
export function renderBedDemo(
  def: BedDef,
  o: {
    seconds: number;
    seed: number;
    script: (t: number, p: Float32Array) => void;
    sampleRate?: number;
    name?: string;
  },
): StereoRender {
  const sr = o.sampleRate ?? AUDIO_RATE;
  const voice = def.create(sr, o.seed);
  const p = Float32Array.from(def.defaults);
  return renderOffline(
    (l, r, n, frame) => {
      o.script(frame / sr, p);
      voice.process(p, 0, l, r, n);
    },
    o.seconds,
    sr,
    o.name ?? def.id,
  );
}
