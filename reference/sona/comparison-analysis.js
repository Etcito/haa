// Measurements operate on decoded PCM. No generated or reference-shaped data is
// substituted for a measured signal. Spectral bands report RMS power in dBFS.
import { audibleBandRms } from './comparison-level.js';
const DB_FLOOR = -120;
const FFT_PLANS = new Map();
const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
const finite = value => Number.isFinite(value) ? value : 0;
export const amplitudeDb = value => value > 0 ? Math.max(DB_FLOOR, 20 * Math.log10(value)) : DB_FLOOR;

export function clampRegion(duration, start = 0, length = duration) {
  if (!Number.isFinite(duration) || duration <= 0) throw new Error('Le son de référence est vide.');
  if (!Number.isFinite(Number(start)) || !Number.isFinite(Number(length))) throw new Error('La région doit avoir un début et une durée valides.');
  const minimum = Math.min(.25, duration);
  const safeStart = clamp(Number(start), 0, Math.max(0, duration - minimum));
  const safeLength = Math.min(clamp(Number(length), minimum, 30), duration - safeStart);
  return { start: safeStart, length: safeLength, end: safeStart + safeLength, duration: safeLength };
}

export function cropPCM(channels, sampleRate, start, duration, { fadeSeconds = .005 } = {}) {
  if (!channels.length || !Number.isFinite(sampleRate) || sampleRate <= 0) throw new Error('Données audio invalides.');
  const available = Math.min(...channels.map(channel => channel.length));
  const first = clamp(Math.round(start * sampleRate), 0, Math.max(0, available - 1));
  const count = Math.min(available - first, Math.max(1, Math.round(duration * sampleRate)));
  const fade = Math.min(Math.floor(count / 2), Math.max(0, Math.round(fadeSeconds * sampleRate)));
  return channels.map(channel => {
    const result = new Float32Array(count);
    for (let i = 0; i < count; i++) {
      const edge = fade > 1 ? Math.min(1, i / (fade - 1), (count - 1 - i) / (fade - 1)) : 1;
      result[i] = finite(channel[first + i]) * Math.max(0, edge);
    }
    return result;
  });
}

function fftPlan(size) {
  if (FFT_PLANS.has(size)) return FFT_PLANS.get(size);
  const bits = Math.log2(size), reverse = new Uint32Array(size);
  const cos = new Float64Array(size / 2), sin = new Float64Array(size / 2);
  const window = new Float64Array(size);
  let windowEnergy = 0;
  for (let i = 0; i < size; i++) {
    let value = i, reversed = 0;
    for (let bit = 0; bit < bits; bit++) { reversed = (reversed << 1) | (value & 1); value >>>= 1; }
    reverse[i] = reversed;
    window[i] = .5 - .5 * Math.cos(2 * Math.PI * i / (size - 1));
    windowEnergy += window[i] * window[i];
  }
  for (let i = 0; i < size / 2; i++) {
    cos[i] = Math.cos(-2 * Math.PI * i / size);
    sin[i] = Math.sin(-2 * Math.PI * i / size);
  }
  const plan = { reverse, cos, sin, window, windowEnergy };
  FFT_PLANS.set(size, plan);
  return plan;
}

function transform(real, imaginary, plan) {
  const size = real.length;
  for (let i = 0; i < size; i++) {
    const j = plan.reverse[i];
    if (j > i) {
      [real[i], real[j]] = [real[j], real[i]];
      [imaginary[i], imaginary[j]] = [imaginary[j], imaginary[i]];
    }
  }
  for (let width = 2; width <= size; width *= 2) {
    const half = width / 2, step = size / width;
    for (let base = 0; base < size; base += width) {
      for (let j = 0; j < half; j++) {
        const twiddle = j * step, left = base + j, right = left + half;
        const re = real[right] * plan.cos[twiddle] - imaginary[right] * plan.sin[twiddle];
        const im = real[right] * plan.sin[twiddle] + imaginary[right] * plan.cos[twiddle];
        real[right] = real[left] - re; imaginary[right] = imaginary[left] - im;
        real[left] += re; imaginary[left] += im;
      }
    }
  }
}

export function analyzePCM(channels, sampleRate, { waveformPoints = 384, spectrumPoints = 80, fftSize = 4096, maxFrames = 48 } = {}) {
  console.info('[Sona] Analyzing the comparison segment');
  if (!Array.isArray(channels) || !channels.length || !Number.isFinite(sampleRate) || sampleRate <= 0) throw new Error('Données audio invalides.');
  const length = Math.min(...channels.map(channel => channel.length));
  if (length < 1) throw new Error('Le segment audio est vide.');
  const pointCount = Math.min(length, clamp(Math.round(waveformPoints) || 384, 1, 512));
  const waveform = Array.from({ length: pointCount }, () => [Infinity, -Infinity]);
  let sumSquares = 0, peak = 0;
  for (const channel of channels) {
    for (let i = 0; i < length; i++) {
      const value = finite(channel[i]);
      sumSquares += value * value; peak = Math.max(peak, Math.abs(value));
      const bucket = waveform[Math.min(pointCount - 1, Math.floor(i * pointCount / length))];
      bucket[0] = Math.min(bucket[0], value); bucket[1] = Math.max(bucket[1], value);
    }
  }
  const rms = Math.sqrt(sumSquares / (length * channels.length));
  const matchRms = audibleBandRms(channels, sampleRate);
  const size = fftSize === 2048 ? 2048 : 4096;
  const plan = fftPlan(size), real = new Float64Array(size), imaginary = new Float64Array(size);
  const power = new Float64Array(size / 2 + 1);
  const frameCount = Math.min(clamp(Math.round(maxFrames) || 48, 1, 128), Math.max(1, Math.ceil(length / (size / 2))));
  const lastStart = Math.max(0, length - size);
  let weightedWindowEnergy = plan.windowEnergy;
  if (length < size) {
    weightedWindowEnergy = 0;
    for (let i = 0; i < length; i++) weightedWindowEnergy += plan.window[i] ** 2;
  }
  const scale = 1 / (size * Math.max(1e-20, weightedWindowEnergy) * frameCount * channels.length);
  for (let frame = 0; frame < frameCount; frame++) {
    const first = frameCount > 1 ? Math.round(frame * lastStart / (frameCount - 1)) : 0;
    for (const channel of channels) {
      for (let i = 0; i < size; i++) real[i] = (first + i < length ? finite(channel[first + i]) : 0) * plan.window[i];
      imaginary.fill(0);
      transform(real, imaginary, plan);
      for (let bin = 0; bin < power.length; bin++) {
        const sided = bin === 0 || bin === size / 2 ? 1 : 2;
        power[bin] += (real[bin] ** 2 + imaginary[bin] ** 2) * scale * sided;
      }
    }
  }
  const minFrequency = 20, maxFrequency = Math.min(20000, sampleRate / 2);
  const logRange = Math.log(maxFrequency / minFrequency);
  const bandCount = clamp(Math.round(spectrumPoints) || 80, 1, 96);
  const buckets = Array.from({ length: bandCount }, () => ({ power: 0, frequencySum: 0, count: 0 }));
  let totalPower = 0, weightedFrequency = 0, lowPower = 0, midPower = 0, highPower = 0;
  for (let bin = 1; bin < power.length; bin++) {
    const frequency = bin * sampleRate / size;
    if (frequency < minFrequency || frequency > maxFrequency) continue;
    const energy = power[bin];
    totalPower += energy; weightedFrequency += frequency * energy;
    if (frequency < 250) lowPower += energy;
    else if (frequency < 2000) midPower += energy;
    else highPower += energy;
    const index = clamp(Math.floor(Math.log(frequency / minFrequency) / logRange * bandCount), 0, bandCount - 1);
    buckets[index].power += energy; buckets[index].frequencySum += Math.log(frequency); buckets[index].count++;
  }
  const spectrum = [];
  for (let i = 0; i < bandCount; i++) {
    const bucket = buckets[i];
    // Empty bands below the FFT resolution are omitted, never interpolated into measurements.
    if (!bucket.count) continue;
    spectrum.push({
      frequency: Math.exp(bucket.frequencySum / bucket.count),
      db: bucket.power > 0 ? Math.max(DB_FLOOR, 10 * Math.log10(bucket.power)) : DB_FLOOR,
      lowHz: minFrequency * Math.exp(i / bandCount * logRange),
      highHz: minFrequency * Math.exp((i + 1) / bandCount * logRange),
    });
  }
  return {
    rms, peak, rmsDb: amplitudeDb(rms), peakDb: amplitudeDb(peak),
    matchRms, matchRmsDb: amplitudeDb(matchRms), levelBand: [20, Math.min(20000, sampleRate * .49)],
    crestFactorDb: rms > 0 && peak > 0 ? 20 * Math.log10(peak / rms) : 0,
    centroid: totalPower > 1e-20 ? weightedFrequency / totalPower : 0,
    centroidWeighting: 'power',
    lowFraction: totalPower > 1e-20 ? lowPower / totalPower : 0,
    midFraction: totalPower > 1e-20 ? midPower / totalPower : 0,
    highFraction: totalPower > 1e-20 ? highPower / totalPower : 0,
    waveform, spectrum, duration: length / sampleRate, sampleRate, channels: channels.length,
    sampleCount: length, fftSize: size, analyzedFrames: frameCount, window: 'Hann',
    spectrumUnit: 'dBFS RMS par bande',
    bands: { low: [20, 250], mid: [250, 2000], high: [2000, maxFrequency] },
    silent: rms < 1e-8,
  };
}

export function computeLevelMatch(reference, synthesis, enabled = true) {
  console.info('[Sona] Matching comparison levels', { enabled });
  const refRms = Math.max(0, reference?.matchRms ?? reference?.rms ?? 0);
  const synthRms = Math.max(0, synthesis?.matchRms ?? synthesis?.rms ?? 0);
  let referenceGain = 1, synthesisGain = 1;
  const canMatch = !!reference && !!synthesis && refRms > 1e-8 && synthRms > 1e-8;
  if (enabled && canMatch) {
    // Exclude infrasonic rumble from the gain measurement, without filtering
    // either playback buffer. Raw peaks still set shared safe headroom.
    const target = .1; // -20 dBFS RMS before any peak-driven attenuation.
    referenceGain = target / refRms; synthesisGain = target / synthRms;
  }
  // A shared attenuation retains relative levels when matching is disabled.
  const peak = Math.max((reference?.peak || 0) * referenceGain, (synthesis?.peak || 0) * synthesisGain);
  const headroom = peak > .9 ? .9 / peak : 1;
  referenceGain *= headroom; synthesisGain *= headroom;
  return {
    referenceGain, synthesisGain,
    gainDbReference: amplitudeDb(referenceGain), gainDbSynthesis: amplitudeDb(synthesisGain),
    targetRmsDb: enabled && canMatch ? amplitudeDb(refRms * referenceGain) : null,
    matching: !!enabled && canMatch,
    method: reference?.matchRms != null && synthesis?.matchRms != null ? 'RMS 20 Hz–20 kHz' : 'RMS', peakLimit: .9,
  };
}
