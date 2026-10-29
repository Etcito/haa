/** Generic causal transforms for control signals. No sound-specific behavior. */
export const RESPONSE_PARAMS = {
  input: [0, 1, 0], attack: [0, 2, .003], release: [0, 3, .035],
  delay: [0, 2, 0], shape: [.25, 4, 1], gain: [0, 4, 1],
  sensitivity: [.001, 1, .04],
};
export const RESPONSE_ENUMS = { mode: ['follow', 'rise', 'fall'] };
const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

export function createControlResponseState(sampleRate = 48000) {
  console.info('[Sona] Creating a reusable control response');
  const sr = clamp(Number(sampleRate) || 48000, 8000, 192000);
  return {
    sampleRate: sr, history: new Float64Array(Math.ceil(sr * RESPONSE_PARAMS.delay[1]) + 2),
    write: 0, available: 0, previous: 0, envelope: 0, lastTime: -1,
    attack: -1, release: -1, attackCoefficient: 1, releaseCoefficient: 1,
  };
}

// Called only by the audio loop. Invalidating the history avoids an allocation
// or a long buffer clear at every loop boundary.
function resetResponse(state) {
  state.write = 0; state.available = 0;
  state.previous = 0; state.envelope = 0;
}

/**
 * Follow: delay -> power/gain -> independent attack/release smoothing.
 * Rise/Fall: rectify the signed input slope first. `sensitivity` is the time
 * in seconds over which a full-scale ramp gives a full-scale edge response.
 * Zero attack/release/delay with unit gain/power is an exact identity in Follow.
 */
export function controlResponseSample(state, params, timeSeconds) {
  if (state.lastTime >= 0 && (timeSeconds < state.lastTime || timeSeconds - state.lastTime > 2 / state.sampleRate)) resetResponse(state);
  state.lastTime = timeSeconds;
  const input = clamp(Number.isFinite(params.input) ? params.input : 0, 0, 1);
  const delay = clamp(Number.isFinite(params.delay) ? params.delay : 0, 0, RESPONSE_PARAMS.delay[1]) * state.sampleRate;
  const frames = Math.floor(delay), fraction = delay - frames, size = state.history.length;
  state.history[state.write] = input;
  let recent = state.write - frames;
  if (recent < 0) recent += size;
  const older = recent ? recent - 1 : size - 1;
  const a = frames <= state.available ? state.history[recent] : 0;
  const b = frames + 1 <= state.available ? state.history[older] : 0;
  const delayed = a + (b - a) * fraction;
  state.write = (state.write + 1) % size;
  state.available = Math.min(size - 1, state.available + 1);

  let target = delayed;
  if (params.mode === 'rise' || params.mode === 'fall') {
    const sensitivity = clamp(Number.isFinite(params.sensitivity) ? params.sensitivity : .04, .001, 1);
    const slope = (delayed - state.previous) * state.sampleRate * sensitivity;
    target = clamp(params.mode === 'rise' ? slope : -slope, 0, 1);
  }
  state.previous = delayed;
  const power = clamp(Number.isFinite(params.shape) ? params.shape : 1, .25, 4);
  const gain = clamp(Number.isFinite(params.gain) ? params.gain : 1, 0, 4);
  if (power !== 1) target = Math.pow(target, power);
  target = clamp(target * gain, 0, 1);
  const attack = clamp(Number.isFinite(params.attack) ? params.attack : .003, 0, 2);
  const release = clamp(Number.isFinite(params.release) ? params.release : .035, 0, 3);
  if (attack !== state.attack) {
    state.attack = attack;
    state.attackCoefficient = attack === 0 ? 1 : -Math.expm1(-1 / (attack * state.sampleRate));
  }
  if (release !== state.release) {
    state.release = release;
    state.releaseCoefficient = release === 0 ? 1 : -Math.expm1(-1 / (release * state.sampleRate));
  }
  const coefficient = target > state.envelope ? state.attackCoefficient : state.releaseCoefficient;
  state.envelope = coefficient === 1 ? target : state.envelope + (target - state.envelope) * coefficient;
  return clamp(state.envelope, 0, 1);
}
