import type { StereoRender } from './offline.ts';

/** A frame-major mono magnitude spectrum, in dB with a -90 dB floor. */
export interface Spectrogram {
  bins: number;
  frames: number;
  hopS: number;
  maxHz: number;
  db: Float32Array;
}

/** Transform complex samples in place with an iterative radix-2 FFT. */
function transform(real: Float64Array, imaginary: Float64Array): void {
  const n = real.length;
  for (let i = 1, reversed = 0; i < n; i++) {
    let bit = n / 2;
    while (reversed >= bit) {
      reversed -= bit;
      bit /= 2;
    }
    reversed += bit;
    if (i < reversed) {
      [real[i], real[reversed]] = [real[reversed], real[i]];
      [imaginary[i], imaginary[reversed]] = [imaginary[reversed], imaginary[i]];
    }
  }

  for (let size = 2; size <= n; size *= 2) {
    const half = size / 2;
    const angle = -2 * Math.PI / size;
    const stepReal = Math.cos(angle);
    const stepImaginary = Math.sin(angle);
    for (let start = 0; start < n; start += size) {
      let twiddleReal = 1;
      let twiddleImaginary = 0;
      for (let j = 0; j < half; j++) {
        const even = start + j;
        const odd = even + half;
        const oddReal = twiddleReal * real[odd] - twiddleImaginary * imaginary[odd];
        const oddImaginary = twiddleReal * imaginary[odd] + twiddleImaginary * real[odd];
        real[odd] = real[even] - oddReal;
        imaginary[odd] = imaginary[even] - oddImaginary;
        real[even] += oddReal;
        imaginary[even] += oddImaginary;
        const nextReal = twiddleReal * stepReal - twiddleImaginary * stepImaginary;
        twiddleImaginary = twiddleReal * stepImaginary + twiddleImaginary * stepReal;
        twiddleReal = nextReal;
      }
    }
  }
}

/**
 * Analyse the mono mix with a periodic Hann window. Window gain is removed so
 * a full-scale, bin-centred sine peaks at 0 dB. Short renders are zero-padded;
 * non-finite channel samples contribute zero to the mix.
 */
export function spectrogram(r: StereoRender, o: { fft?: number; hop?: number } = {}): Spectrogram {
  const fft = o.fft ?? 1024;
  const hop = o.hop ?? 256;
  if (!Number.isSafeInteger(fft) || fft < 2 || !Number.isInteger(Math.log2(fft))) {
    throw new Error(`[haa] fft must be a power of two >= 2: ${fft}`);
  }
  if (!Number.isSafeInteger(hop) || hop <= 0) {
    throw new Error(`[haa] hop must be a positive integer: ${hop}`);
  }

  const bins = fft / 2;
  const frames = Math.max(1, Math.floor((r.left.length - fft) / hop) + 1);
  const db = new Float32Array(frames * bins);
  const window = new Float64Array(fft);
  let windowSum = 0;
  for (let i = 0; i < fft; i++) {
    window[i] = 0.5 - 0.5 * Math.cos(2 * Math.PI * i / fft);
    windowSum += window[i];
  }

  const real = new Float64Array(fft);
  const imaginary = new Float64Array(fft);
  for (let frame = 0; frame < frames; frame++) {
    for (let i = 0; i < fft; i++) {
      const index = frame * hop + i;
      const left = r.left[index];
      const right = r.right[index];
      const mono = ((Number.isFinite(left) ? left : 0) + (Number.isFinite(right) ? right : 0)) / 2;
      real[i] = mono * window[i];
    }
    imaginary.fill(0);
    transform(real, imaginary);
    for (let bin = 0; bin < bins; bin++) {
      const magnitude = Math.hypot(real[bin], imaginary[bin]) * (bin === 0 ? 1 : 2) / windowSum;
      db[frame * bins + bin] = Math.max(-90, 20 * Math.log10(magnitude));
    }
  }

  return { bins, frames, hopS: hop / r.sampleRate, maxHz: r.sampleRate / 2, db };
}
