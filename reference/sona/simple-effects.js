// A short, fractional feedback delay. This is a general comb filter: it has no
// knowledge of engines, voices, water, firing orders or preset timelines.
export const SIMPLE_EFFECT_PARAMS = {
  comb: {
    time: [.0002, .05, .004], feedback: [-.95, .95, .55],
    damping: [0, 1, .4], mix: [0, 1, .5],
  },
};

const clamp = (x, lo, hi) => Math.min(hi, Math.max(lo, x));

export function createSimpleEffectState(type, sampleRate) {
  if (type !== 'comb') throw new Error('Effet simple inconnu : ' + type);
  const length = Math.ceil(sampleRate * .055) + 4;
  return { sampleRate, length, left: new Float32Array(length), right: new Float32Array(length),
    index: 0, filterL: 0, filterR: 0, coefficient: .5, counter: 0, outL: 0, outR: 0 };
}

export function processSimpleEffect(type, state, inputL, inputR, params) {
  if (type !== 'comb') return;
  if ((state.counter++ & 15) === 0) {
    const cutoff = 180 + Math.pow(1 - params.damping, 2) * Math.min(16000, state.sampleRate * .42);
    state.coefficient = 1 - Math.exp(-2 * Math.PI * cutoff / state.sampleRate);
  }
  const delay = clamp(params.time * state.sampleRate, 1, state.length - 2);
  let position = state.index - delay;
  if (position < 0) position += state.length;
  const first = Math.floor(position), second = (first + 1) % state.length, fraction = position - first;
  const delayedL = state.left[first] + (state.left[second] - state.left[first]) * fraction;
  const delayedR = state.right[first] + (state.right[second] - state.right[first]) * fraction;
  state.filterL += (delayedL - state.filterL) * state.coefficient;
  state.filterR += (delayedR - state.filterR) * state.coefficient;
  const feedback = clamp(params.feedback, -.95, .95);
  const normalization = 1 - Math.abs(feedback);
  const wetL = inputL * normalization + state.filterL * feedback;
  const wetR = inputR * normalization + state.filterR * feedback;
  state.left[state.index] = wetL;
  state.right[state.index] = wetR;
  state.index = (state.index + 1) % state.length;
  state.outL = inputL + (wetL - inputL) * params.mix;
  state.outR = inputR + (wetR - inputR) * params.mix;
}
