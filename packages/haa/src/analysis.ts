import type { StereoRender } from './offline.ts';

/** Level and validity measurements for an offline stereo render. */
export interface SoundStats {
  frames: number;
  sampleRate: number;
  peakDbfs: number;
  rmsDbfs: number;
  nonFinite: number;
  silent: boolean;
  clipped: boolean;
  dc: number;
}

/** Convert a linear amplitude to dBFS, with a finite silence floor. */
function dbfs(amplitude: number): number {
  return Math.max(-200, 20 * Math.log10(amplitude));
}

/**
 * Measure peak and RMS over finite channel samples. DC is the mean mono mix
 * over frames with two finite samples; empty measurements are zero in amplitude.
 */
export function analyse(r: StereoRender): SoundStats {
  const frames = r.left.length;
  let peak = 0;
  let squares = 0;
  let finite = 0;
  let nonFinite = 0;
  let monoSum = 0;
  let monoFrames = 0;

  for (const channel of [r.left, r.right]) {
    for (const sample of channel) {
      if (!Number.isFinite(sample)) {
        nonFinite++;
        continue;
      }
      peak = Math.max(peak, Math.abs(sample));
      squares += sample * sample;
      finite++;
    }
  }
  for (let i = 0; i < frames; i++) {
    const left = r.left[i];
    const right = r.right[i];
    if (Number.isFinite(left) && Number.isFinite(right)) {
      monoSum += (left + right) / 2;
      monoFrames++;
    }
  }

  const peakDbfs = dbfs(peak);
  return {
    frames,
    sampleRate: r.sampleRate,
    peakDbfs,
    rmsDbfs: dbfs(finite === 0 ? 0 : Math.sqrt(squares / finite)),
    nonFinite,
    silent: peakDbfs < -60,
    clipped: peakDbfs > 0,
    dc: monoFrames === 0 ? 0 : monoSum / monoFrames,
  };
}
