/**
 * Reusable periodic excitation. There are no engine/animal rules in this module.
 * A pulse is integrated over each sample, so very short events retain their
 * area instead of disappearing between samples. This reduces (not eliminates)
 * aliasing; the graph can add its own spectral shaping afterwards.
 */
export const PULSE_PARAMS = {
  frequency: [0, 4000, 100], width: [.002, .95, .08], phase: [0, 1, 0],
  jitter: [0, .25, .02], shimmer: [0, 1, .05], level: [0, 1, .5],
};
export const PULSE_ENUMS = { shape: ['rounded', 'exponential'] };

const TAU = Math.PI * 2;
const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

function random(state) {
  let value = state.seed | 0;
  value ^= value << 13; value ^= value >>> 17; value ^= value << 5;
  state.seed = value >>> 0;
  return state.seed / 4294967296;
}

export function createPulseState(sampleRate, seed = 1) {
  const sr = clamp(Number(sampleRate) || 48000, 8000, 192000);
  return {
    sampleRate: sr, seed: seed >>> 0 || 1,
    phase: 0, phaseSetting: 0, initialized: false, counter: 0,
    delay: 0, amplitude: 1, dc: 0,
    dcCoefficient: 1 - Math.exp(-TAU * 8 / sr),
    width: -1, attack: 0, decayEnd: 0, attackEnd: 0,
    decayScale: 1, attackScale: 1, normalization: 1,
  };
}

function startCycle(state, jitter, shimmer) {
  // Timing varies inside a fixed cycle. It never accumulates into pitch drift,
  // so two identically driven clocks keep a deliberate phase relationship.
  state.delay = random(state) * jitter;
  state.amplitude = 1 + (random(state) * 2 - 1) * shimmer;
}

function updateWidth(state, width) {
  state.width = width;
  state.attack = width * .08;
  state.decayEnd = Math.exp(-1 / width);
  state.attackEnd = Math.exp(-1 / state.attack);
  state.decayScale = 1 / (1 - state.decayEnd);
  state.attackScale = 1 / (1 - state.attackEnd);
  const peakPosition = Math.log(width * (1 - state.decayEnd) / (state.attack * (1 - state.attackEnd))) / (1 / state.attack - 1 / width);
  const peak = (Math.exp(-peakPosition / width) - state.decayEnd) * state.decayScale
    - (Math.exp(-peakPosition / state.attack) - state.attackEnd) * state.attackScale;
  state.normalization = 1 / Math.max(.0001, peak);
}

// Integral from zero to phase. Each shape is zero at its cycle boundaries;
// changing a cycle's amplitude therefore does not introduce an abrupt edge.
function area(state, phase, exponential) {
  if (phase <= state.delay) return 0;
  const scale = 1 - state.delay;
  const position = Math.min(1, (phase - state.delay) / scale);
  let integral;
  if (exponential) {
    integral = (state.width * (1 - Math.exp(-position / state.width)) - position * state.decayEnd) * state.decayScale
      - (state.attack * (1 - Math.exp(-position / state.attack)) - position * state.attackEnd) * state.attackScale;
    integral *= state.normalization;
  } else if (position >= state.width) {
    integral = state.width * .5;
  } else {
    integral = position * .5 - state.width * Math.sin(TAU * position / state.width) / (2 * TAU);
  }
  return integral * scale * state.amplitude;
}

/** Finite mono excitation, with no allocations in the sample loop. */
export function pulseSample(state, params, timeSeconds = 0, frameIndex = 0) {
  const frequency = clamp(Number.isFinite(params.frequency) ? params.frequency : 100, 0, Math.min(4000, state.sampleRate * .5));
  const width = clamp(Number.isFinite(params.width) ? params.width : .08, .002, .95);
  const phaseSetting = clamp(Number.isFinite(params.phase) ? params.phase : 0, 0, 1);
  const jitter = clamp(Number.isFinite(params.jitter) ? params.jitter : .02, 0, .25);
  const shimmer = clamp(Number.isFinite(params.shimmer) ? params.shimmer : .05, 0, 1);
  const level = clamp(Number.isFinite(params.level) ? params.level : .5, 0, 1);
  const exponential = params.shape === 'exponential';
  if (!state.initialized) {
    state.phase = phaseSetting % 1;
    state.phaseSetting = phaseSetting;
    state.initialized = true;
    updateWidth(state, width);
    startCycle(state, jitter, shimmer);
  } else if (phaseSetting !== state.phaseSetting) {
    state.phase += phaseSetting - state.phaseSetting;
    state.phase -= Math.floor(state.phase);
    state.phaseSetting = phaseSetting;
  }
  if ((state.counter++ & 15) === 0 && state.width !== width) updateWidth(state, width);

  // A zero-frequency control can stop any pulse sequence without a division
  // by zero. Keep its clock position for a coherent restart; discard DC slowly.
  if (frequency === 0) {
    state.dc *= 1 - state.dcCoefficient;
    return 0;
  }

  const step = frequency / state.sampleRate;
  const end = state.phase + step;
  let integrated;
  if (end < 1) {
    integrated = area(state, end, exponential) - area(state, state.phase, exponential);
    state.phase = end;
  } else {
    integrated = area(state, 1, exponential) - area(state, state.phase, exponential);
    startCycle(state, jitter, shimmer);
    state.phase = end - 1;
    integrated += area(state, state.phase, exponential);
  }
  const sample = integrated / step;
  state.dc += (sample - state.dc) * state.dcCoefficient;
  return (sample - state.dc) * level;
}
