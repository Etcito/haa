/* Acoustic colour and stereo processing. EQ coefficients follow the RBJ/W3C cookbook. */
const TAU = Math.PI * 2;
const clamp = (value, lo, hi) => Math.min(hi, Math.max(lo, value));
const dbToGain = value => Math.pow(10, value / 20);
const FDN_TIMES = [.0217, .0271, .0319, .0373, .0437, .0499, .0571, .0637];
const EARLY_TIMES = [.0043, .0079, .0127, .0181, .0269, .0397];
const EARLY_GAINS = [.38, -.27, .24, .19, -.13, .10];
const MODE_RATIOS = [1, 1.47, 2.13, 2.91, 4.17, 5.43];
const MODE_GAINS = [1, .68, .48, .34, .21, .14];

export const EFFECT_PARAMS = {
  filter: { cutoff: [10, 18000, 4000], resonance: [.1, 12, .7] },
  drive: { amount: [0, 1, .25], mix: [0, 1, .5] },
  gain: { gain: [0, 2, .8] },
  delay: { time: [.01, 1, .22], feedback: [0, .85, .3], mix: [0, 1, .2] },
  reverb: { size: [0, 1, .4], mix: [0, 1, .18] },
  resonator: { frequency: [80, 6000, 600], decay: [.005, 1, .09], color: [0, 1, .35], mix: [0, 1, .4] },
  formant: { frequency: [120, 1600, 500], spread: [.5, 2, 1], resonance: [.3, 6, 1.3], mix: [0, 1, .75] },
  equalizer: { low: [-18, 18, 0], mid: [-18, 18, 0], high: [-18, 18, 0], frequency: [150, 8000, 1600] },
  compressor: { threshold: [-48, 0, -16], ratio: [1, 12, 3], attack: [.001, .2, .008], release: [.02, 1, .12], makeup: [0, 18, 2] },
  space: { size: [0, 1, .35], damping: [0, 1, .5], mix: [0, 1, .15], width: [0, 1, .7], distance: [0, 1, .15] },
  output: { gain: [0, 1, .8] },
};

function biquad() {
  return { b0: 1, b1: 0, b2: 0, a1: 0, a2: 0, l1: 0, l2: 0, r1: 0, r2: 0 };
}

function normalize(b, b0, b1, b2, a0, a1, a2) {
  const norm = 1 / a0;
  b.b0 = b0 * norm; b.b1 = b1 * norm; b.b2 = b2 * norm;
  b.a1 = a1 * norm; b.a2 = a2 * norm;
}

function runBiquad(b, left, right) {
  const l = left * b.b0 + b.l1, r = right * b.b0 + b.r1;
  b.l1 = left * b.b1 - b.a1 * l + b.l2;
  b.l2 = left * b.b2 - b.a2 * l;
  b.r1 = right * b.b1 - b.a1 * r + b.r2;
  b.r2 = right * b.b2 - b.a2 * r;
  b.left = l; b.right = r;
}

function bandpass(b, frequency, q, sampleRate) {
  const w = TAU * clamp(frequency, 30, sampleRate * .43) / sampleRate;
  const a = Math.sin(w) / (2 * q);
  normalize(b, a, 0, -a, 1 + a, -2 * Math.cos(w), 1 - a);
}

function equalizerCoefficients(b, type, gainDb, frequency, sr) {
  const A = Math.pow(10, gainDb / 40), w = TAU * clamp(frequency, 20, sr * .43) / sr;
  const c = Math.cos(w), alpha = Math.sin(w) / Math.SQRT2;
  if (type === 'peak') {
    const peakAlpha = Math.sin(w) / 1.6;
    normalize(b, 1 + peakAlpha * A, -2 * c, 1 - peakAlpha * A,
      1 + peakAlpha / A, -2 * c, 1 - peakAlpha / A);
  } else {
    const t = 2 * Math.sqrt(A) * alpha;
    if (type === 'low') normalize(b,
      A * ((A + 1) - (A - 1) * c + t), 2 * A * ((A - 1) - (A + 1) * c), A * ((A + 1) - (A - 1) * c - t),
      (A + 1) + (A - 1) * c + t, -2 * ((A - 1) + (A + 1) * c), (A + 1) + (A - 1) * c - t);
    else normalize(b,
      A * ((A + 1) + (A - 1) * c + t), -2 * A * ((A - 1) + (A + 1) * c), A * ((A + 1) + (A - 1) * c - t),
      (A + 1) - (A - 1) * c + t, 2 * ((A - 1) - (A + 1) * c), (A + 1) - (A - 1) * c - t);
  }
}

function readDelay(buffer, index, samples) {
  let pos = index - samples;
  while (pos < 0) pos += buffer.length;
  const base = Math.floor(pos), fraction = pos - base;
  return buffer[base] + fraction * (buffer[(base + 1) % buffer.length] - buffer[base]);
}

function logCosh(x) {
  const a = Math.abs(x);
  return a + Math.log1p(Math.exp(-2 * a)) - Math.LN2;
}

export function createEffectState(type, sampleRate) {
  const state = { sr: sampleRate, outL: 0, outR: 0, fresh: true, previousL: 0, previousR: 0 };
  if (type === 'filter') {
    state.ic1L = state.ic2L = state.ic1R = state.ic2R = 0;
    state.a1 = 1; state.a2 = state.a3 = 0; state.k = 1;
  } else if (type === 'delay') {
    state.delayL = new Float32Array(Math.ceil(sampleRate * 1.02) + 2);
    state.delayR = new Float32Array(state.delayL.length); state.index = 0;
  } else if (type === 'resonator') {
    state.modes = Array.from({ length: 6 }, () => biquad());
  } else if (type === 'formant' || type === 'equalizer') {
    state.bands = Array.from({ length: 3 }, () => biquad());
  } else if (type === 'compressor') {
    state.envelope = 0; state.compression = 1; state.targetGain = 1;
    state.attack = state.release = 0; state.makeup = 1;
  } else if (type === 'space' || type === 'reverb') {
    state.lines = FDN_TIMES.map(t => new Float32Array(Math.ceil(t * 2.25 * sampleRate) + 4));
    state.indices = new Uint32Array(8); state.damping = new Float64Array(8);
    state.reads = new Float64Array(8); state.delaySamples = new Float64Array(8);
    state.feedback = new Float64Array(8); state.earlySamples = new Float64Array(6);
    state.earlyL = new Float32Array(Math.ceil(.12 * sampleRate) + 8);
    state.earlyR = new Float32Array(state.earlyL.length); state.earlyIndex = 0;
    state.distanceL = state.distanceR = 0; state.airAlpha = 1; state.dampAlpha = .3;
  }
  return state;
}

function acousticSpace(state, inputL, inputR, p, frame, legacy) {
  const size = p.size, damping = legacy ? .5 : p.damping, distance = legacy ? 0 : p.distance;
  const width = legacy ? .7 : p.width, mix = p.mix, sr = state.sr;
  if ((frame & 31) === 0 || state.fresh) {
    const scale = .4 + size * 1.65, rt60 = .16 + size * 2.6;
    for (let i = 0; i < 8; i++) {
      state.delaySamples[i] = FDN_TIMES[i] * scale * sr;
      state.feedback[i] = Math.exp(-6.907755 * FDN_TIMES[i] * scale / rt60);
    }
    for (let i = 0; i < 6; i++) state.earlySamples[i] = EARLY_TIMES[i] * (.55 + size * 1.35) * sr;
    state.dampAlpha = 1 - Math.exp(-TAU * (14000 * Math.pow(.085, damping)) / sr);
    state.airAlpha = 1 - Math.exp(-TAU * (17000 / (1 + distance * 8)) / sr);
    state.fresh = false;
  }
  state.distanceL += state.airAlpha * (inputL - state.distanceL);
  state.distanceR += state.airAlpha * (inputR - state.distanceR);
  const airBlend = distance * .85;
  const sourceL = inputL * (1 - airBlend) + state.distanceL * airBlend;
  const sourceR = inputR * (1 - airBlend) + state.distanceR * airBlend;
  const earlyIndex = state.earlyIndex;
  state.earlyL[earlyIndex] = sourceL; state.earlyR[earlyIndex] = sourceR;
  let earlyL = 0, earlyR = 0;
  for (let i = 0; i < 6; i++) {
    const t = state.earlySamples[i];
    earlyL += readDelay(i & 1 ? state.earlyR : state.earlyL, earlyIndex, t) * EARLY_GAINS[i];
    earlyR += readDelay(i & 1 ? state.earlyL : state.earlyR, earlyIndex, t * 1.103 + sr * .0013) * EARLY_GAINS[i];
  }
  state.earlyIndex = (earlyIndex + 1) % state.earlyL.length;
  let sum = 0, lateL = 0, lateR = 0;
  for (let i = 0; i < 8; i++) {
    const delayed = readDelay(state.lines[i], state.indices[i], state.delaySamples[i]);
    state.damping[i] += (delayed - state.damping[i]) * state.dampAlpha;
    state.reads[i] = state.damping[i];
    sum += state.damping[i];
    lateL += delayed * (i & 1 ? -.25 : .25);
    lateR += delayed * (i & 2 ? -.25 : .25);
  }
  // Householder feedback matrix is energy-preserving; per-line losses set decay.
  for (let i = 0; i < 8; i++) {
    const feed = (i & 1 ? sourceR : sourceL) * (i & 2 ? -.27 : .27);
    state.lines[i][state.indices[i]] = clamp(feed + (state.reads[i] - .25 * sum) * state.feedback[i], -8, 8);
    state.indices[i] = (state.indices[i] + 1) % state.lines[i].length;
  }
  const wetL = earlyL * .6 + lateL * 1.45, wetR = earlyR * .6 + lateR * 1.45;
  const mid = (wetL + wetR) * .5, side = (wetL - wetR) * .5 * width;
  const distanceGain = 1 / (1 + distance * 1.6), wetMix = clamp(mix + distance * .2, 0, 1);
  state.outL = (sourceL * (1 - wetMix) + (mid + side) * wetMix) * distanceGain;
  state.outR = (sourceR * (1 - wetMix) + (mid - side) * wetMix) * distanceGain;
}

/** Writes outL/outR onto the retained state, with no allocation in the audio loop. */
export function processEffect(type, state, inputL, inputR, p, frame, mode) {
  let left = inputL, right = inputR;
  const sr = state.sr;
  switch (type) {
    case 'filter': {
      if ((frame & 15) === 0 || state.fresh) {
        const g = Math.tan(Math.PI * Math.min(sr * .42, p.cutoff) / sr), k = 1 / p.resonance;
        state.a1 = 1 / (1 + g * (g + k)); state.a2 = g * state.a1; state.a3 = g * state.a2; state.k = k;
        state.fresh = false;
      }
      const v3L = inputL - state.ic2L, v1L = state.a1 * state.ic1L + state.a2 * v3L;
      const v2L = state.ic2L + state.a2 * state.ic1L + state.a3 * v3L;
      const v3R = inputR - state.ic2R, v1R = state.a1 * state.ic1R + state.a2 * v3R;
      const v2R = state.ic2R + state.a2 * state.ic1R + state.a3 * v3R;
      state.ic1L = clamp(2 * v1L - state.ic1L, -64, 64); state.ic2L = clamp(2 * v2L - state.ic2L, -64, 64);
      state.ic1R = clamp(2 * v1R - state.ic1R, -64, 64); state.ic2R = clamp(2 * v2R - state.ic2R, -64, 64);
      left = mode === 'bandpass' ? v1L * state.k : mode === 'highpass' ? inputL - state.k * v1L - v2L : v2L;
      right = mode === 'bandpass' ? v1R * state.k : mode === 'highpass' ? inputR - state.k * v1R - v2R : v2R;
      break;
    }
    case 'drive': {
      const gain = 1 + p.amount * 10, compensation = 1 / Math.sqrt(1 + p.amount * 5);
      const dxL = inputL - state.previousL, dxR = inputR - state.previousR;
      // First-order antiderivative antialiasing of tanh avoids brittle high-frequency folding.
      const wetL = (Math.abs(dxL) > 1e-5 ? (logCosh(gain * inputL) - logCosh(gain * state.previousL)) / (gain * dxL) : Math.tanh(gain * (inputL + state.previousL) * .5)) * compensation;
      const wetR = (Math.abs(dxR) > 1e-5 ? (logCosh(gain * inputR) - logCosh(gain * state.previousR)) / (gain * dxR) : Math.tanh(gain * (inputR + state.previousR) * .5)) * compensation;
      state.previousL = inputL; state.previousR = inputR;
      left = inputL * (1 - p.mix) + wetL * p.mix; right = inputR * (1 - p.mix) + wetR * p.mix;
      break;
    }
    case 'gain': case 'output': left *= p.gain; right *= p.gain; break;
    case 'delay': {
      const l = readDelay(state.delayL, state.index, p.time * sr), r = readDelay(state.delayR, state.index, p.time * sr);
      state.delayL[state.index] = clamp(inputL + l * p.feedback, -8, 8);
      state.delayR[state.index] = clamp(inputR + r * p.feedback, -8, 8);
      state.index = (state.index + 1) % state.delayL.length;
      left = inputL * (1 - p.mix) + l * p.mix; right = inputR * (1 - p.mix) + r * p.mix;
      break;
    }
    case 'resonator': {
      if ((frame & 31) === 0 || state.fresh) {
        for (let i = 0; i < 6; i++) {
          const ratio = (2 * i + 1) * (1 - p.color) + MODE_RATIOS[i] * p.color;
          const f = Math.min(p.frequency * ratio, sr * .43), tau = p.decay / (1 + i * .22);
          const r = Math.exp(-1 / (tau * sr)), b = state.modes[i];
          b.b0 = 1 - r; b.b1 = 0; b.b2 = -(1 - r); b.a1 = -2 * r * Math.cos(TAU * f / sr); b.a2 = r * r;
        }
        state.fresh = false;
      }
      let wetL = 0, wetR = 0;
      for (let i = 0; i < 6; i++) { const b = state.modes[i]; runBiquad(b, inputL, inputR); wetL += b.left * MODE_GAINS[i]; wetR += b.right * MODE_GAINS[i]; }
      left = inputL * (1 - p.mix) + wetL * p.mix * 1.35;
      right = inputR * (1 - p.mix) + wetR * p.mix * 1.35;
      break;
    }
    case 'formant': {
      if ((frame & 31) === 0 || state.fresh) {
        const f = p.frequency, spread = p.spread;
        bandpass(state.bands[0], f, 1.8 + p.resonance, sr);
        bandpass(state.bands[1], f + 900 * spread, 2.5 + p.resonance * 1.2, sr);
        bandpass(state.bands[2], f + 2300 * spread, 3.2 + p.resonance * 1.5, sr);
        state.fresh = false;
      }
      runBiquad(state.bands[0], inputL, inputR); runBiquad(state.bands[1], inputL, inputR); runBiquad(state.bands[2], inputL, inputR);
      const wetL = state.bands[0].left * 1.5 + state.bands[1].left * 1.15 + state.bands[2].left * .65;
      const wetR = state.bands[0].right * 1.5 + state.bands[1].right * 1.15 + state.bands[2].right * .65;
      left = inputL * (1 - p.mix) + wetL * p.mix; right = inputR * (1 - p.mix) + wetR * p.mix;
      break;
    }
    case 'equalizer': {
      if ((frame & 31) === 0 || state.fresh) {
        equalizerCoefficients(state.bands[0], 'low', p.low, 170, sr);
        equalizerCoefficients(state.bands[1], 'peak', p.mid, p.frequency, sr);
        equalizerCoefficients(state.bands[2], 'high', p.high, 4800, sr);
        state.fresh = false;
      }
      for (let i = 0; i < 3; i++) { runBiquad(state.bands[i], left, right); left = state.bands[i].left; right = state.bands[i].right; }
      break;
    }
    case 'compressor': {
      if ((frame & 31) === 0 || state.fresh) {
        state.attack = Math.exp(-1 / (p.attack * sr)); state.release = Math.exp(-1 / (p.release * sr));
        state.makeup = dbToGain(p.makeup); state.fresh = false;
      }
      const peak = Math.max(Math.abs(inputL), Math.abs(inputR));
      const coef = peak > state.envelope ? state.attack : state.release;
      state.envelope = peak + coef * (state.envelope - peak);
      if ((frame & 7) === 0) {
        const over = 20 * Math.log10(Math.max(1e-9, state.envelope)) - p.threshold, knee = 6;
        const reduction = over <= -knee * .5 ? 0 : over >= knee * .5 ? (1 - 1 / p.ratio) * over : (1 - 1 / p.ratio) * (over + knee * .5) ** 2 / (2 * knee);
        state.targetGain = dbToGain(-reduction);
      }
      state.compression += (state.targetGain - state.compression) * .015;
      const gain = state.compression * state.makeup;
      left = inputL * gain; right = inputR * gain;
      break;
    }
    case 'space': case 'reverb': acousticSpace(state, inputL, inputR, p, frame, type === 'reverb'); return;
  }
  state.outL = Number.isFinite(left) ? left : 0; state.outR = Number.isFinite(right) ? right : 0;
}
