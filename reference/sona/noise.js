/** Seeded noise colours with a continuous, level-compensated stereo width. */
export const NOISE_PARAMS = { level: [0, 1, .15], width: [0, 1, 0] };

export function createNoiseState(seed = 1) {
  console.info('[Sona] Creating a reusable stereo noise source');
  return {
    seed: seed >>> 0 || 1, brown: 0, pink0: 0, pink1: 0, pink2: 0,
    previousPink: 0, previousWhite: 0, outL: 0, outR: 0,
    side: { seed: (seed ^ 0x9e3779b9) >>> 0 || 1, brown: 0,
      pink0: 0, pink1: 0, pink2: 0, previousPink: 0, previousWhite: 0 },
  };
}

// Called for every sample. The mono path retains the original arithmetic and
// random sequence, so existing graphs without stereo width sound identical.
function colourSample(state, colour) {
  let seed = state.seed | 0;
  seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5;
  state.seed = seed >>> 0;
  const white = state.seed / 2147483648 - 1;
  let value;
  if (colour === 'brown') {
    state.brown = (state.brown + .02 * white) / 1.02;
    value = state.brown * 3.5;
  } else if (colour === 'pink' || colour === 'blue') {
    state.pink0 = .99765 * state.pink0 + white * .099046;
    state.pink1 = .963 * state.pink1 + white * .2965164;
    state.pink2 = .57 * state.pink2 + white * 1.0526913;
    const pink = (state.pink0 + state.pink1 + state.pink2 + white * .1848) * .12;
    value = colour === 'blue' ? (pink - (state.previousPink || 0)) * 4.2 : pink;
    state.previousPink = pink;
  } else if (colour === 'violet') {
    value = (white - (state.previousWhite || 0)) / Math.SQRT2;
  } else value = white;
  state.previousWhite = white;
  return value;
}

/** A zero width is exactly mono; at full width the two channels decorrelate. */
export function noiseSample(state, params, colour) {
  const gain = params.level * .6;
  const mid = colourSample(state, colour) * gain;
  const width = Math.min(1, Math.max(0, params.width || 0));
  if (width === 0) {
    state.outL = state.outR = mid;
    return mid;
  }
  const side = colourSample(state.side, colour) * gain * width;
  const normalization = 1 / Math.sqrt(1 + width * width);
  state.outL = (mid + side) * normalization;
  state.outR = (mid - side) * normalization;
  return mid * normalization;
}
