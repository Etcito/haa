/** Generic synthesized grains. No recordings or sound-specific rules. */
export const GRAIN_PARAMS = {
  frequency: [40, 12000, 1800], rate: [0, 2000, 12],
  attack: [.0001, .1, .001], decay: [.001, 2, .035],
  spread: [0, 36, 5], jitter: [0, 1, .8], variation: [0, 1, .5],
  glide: [-.9, 3, .05], delay: [0, 20, .02], width: [0, 1, .25], level: [0, 1, .3],
  resonance: [.25, 4, .7],
};
export const GRAIN_ENUMS = { wave: ['sine', 'noise', 'filtered-noise'] };
const TAU = Math.PI * 2;
const VOICES = 64;
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

function random(s) {
  let seed = s.seed | 0; seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5;
  s.seed = seed >>> 0;
  return s.seed / 4294967296;
}

export function createGrainState(sampleRate, seed) {
  console.info('[Sona] Creating a synthesized grain source');
  const make = () => new Float64Array(VOICES);
  return { sampleRate, seed: seed >>> 0 || 1, active: new Uint8Array(VOICES),
    phase: make(), step: make(), targetStep: make(), glideRate: make(),
    envelope: make(), decayRate: make(), attackFrames: make(), age: make(),
    gain: make(), left: make(), right: make(), noiseState: new Uint32Array(VOICES),
    filterG: make(), filterTargetG: make(), filterK: make(), filterIC1: make(), filterIC2: make(),
    clock: 0, threshold: 1, started: false, lastTime: -1,
    outL: 0, outR: 0 };
}

function startGrain(s, p) {
  let index = -1, quietest = Infinity;
  for (let i = 0; i < VOICES; i++) {
    if (!s.active[i]) { index = i; break; }
    const level = s.envelope[i] * s.gain[i];
    if (level < quietest) { quietest = level; index = i; }
  }
  const pitch = Math.pow(2, (random(s) * 2 - 1) * p.spread / 12);
  const frequency = clamp(p.frequency * pitch, 20, s.sampleRate * .42);
  const duration = p.decay * (1 + (random(s) * 2 - 1) * p.variation * .5);
  const pan = (random(s) * 2 - 1) * p.width;
  s.phase[index] = 0;
  s.step[index] = frequency / s.sampleRate;
  s.targetStep[index] = clamp(frequency * (1 + p.glide), 20, s.sampleRate * .42) / s.sampleRate;
  s.glideRate[index] = 1 - Math.exp(-1 / (s.sampleRate * Math.max(.002, duration * .4)));
  // Each coloured burst has its own filter and pitch trajectory. Freeze these
  // at birth so a new drop does not retune an earlier drop's ringing tail.
  // These calculations consume no random values; legacy grains stay identical.
  s.filterG[index] = Math.tan(Math.PI * s.step[index]);
  s.filterTargetG[index] = Math.tan(Math.PI * s.targetStep[index]);
  s.filterK[index] = 1 / clamp(p.resonance ?? .7, .25, 4);
  s.filterIC1[index] = s.filterIC2[index] = 0;
  s.envelope[index] = 1; s.decayRate[index] = Math.exp(-1 / (s.sampleRate * duration));
  s.attackFrames[index] = Math.max(1, p.attack * s.sampleRate);
  // Freeze density compensation into each new grain. Changing rate or decay
  // must not raise the volume of tails which are already sounding. The finite
  // voice pool also bounds the overlap that can actually be rendered.
  const overlap = Math.max(1, Math.min(VOICES, p.rate * p.decay * .5));
  s.age[index] = 0;
  s.gain[index] = (1 - random(s) * p.variation * .7) / Math.sqrt(overlap);
  s.left[index] = Math.cos((pan + 1) * Math.PI / 4);
  s.right[index] = Math.sin((pan + 1) * Math.PI / 4);
  s.noiseState[index] = (random(s) * 4294967295) >>> 0 || 1;
  s.active[index] = 1;
}

export function grainSample(s, p, time) {
  if (time < s.lastTime) { s.started = false; s.clock = 0; }
  s.lastTime = time;
  if (!s.started && time >= p.delay) {
    startGrain(s, p); s.started = true;
    s.threshold = Math.max(.02, 1 + p.jitter * (-Math.log(Math.max(1e-7, random(s))) - 1));
  } else if (s.started && p.rate > 0) {
    s.clock += p.rate / s.sampleRate;
    let count = 0;
    while (s.clock >= s.threshold && count++ < 8) {
      s.clock -= s.threshold; startGrain(s, p);
      s.threshold = Math.max(.02, 1 + p.jitter * (-Math.log(Math.max(1e-7, random(s))) - 1));
    }
  }
  let left = 0, right = 0;
  for (let i = 0; i < VOICES; i++) {
    if (!s.active[i]) continue;
    const age = s.age[i]++;
    const attack = Math.min(1, age / s.attackFrames[i]);
    const envelope = s.envelope[i] * attack * s.gain[i];
    let value;
    if (p.wave === 'noise' || p.wave === 'filtered-noise') {
      let seed = s.noiseState[i] | 0; seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5;
      s.noiseState[i] = seed >>> 0;
      const noise = s.noiseState[i] / 2147483648 - 1;
      if (p.wave === 'filtered-noise') {
        // A topology-preserving state-variable band-pass remains stable while
        // its centre moves. Interpolating g avoids trigonometry in this loop.
        const g = s.filterG[i], k = s.filterK[i];
        const a1 = 1 / (1 + g * (g + k)), a2 = g * a1;
        const v3 = noise - s.filterIC2[i];
        const v1 = a1 * s.filterIC1[i] + a2 * v3;
        const v2 = s.filterIC2[i] + a2 * s.filterIC1[i] + g * a2 * v3;
        s.filterIC1[i] = 2 * v1 - s.filterIC1[i];
        s.filterIC2[i] = 2 * v2 - s.filterIC2[i];
        value = v1 * k * envelope;
        s.filterG[i] += (s.filterTargetG[i] - g) * s.glideRate[i];
      } else value = noise * envelope;
    } else {
      value = Math.sin(TAU * s.phase[i]) * envelope;
      s.phase[i] = (s.phase[i] + s.step[i]) % 1;
      s.step[i] += (s.targetStep[i] - s.step[i]) * s.glideRate[i];
    }
    left += value * s.left[i]; right += value * s.right[i];
    s.envelope[i] *= s.decayRate[i];
    if (s.envelope[i] < .0001) s.active[i] = 0;
  }
  // Level is the explicit live macro; existing grains keep their birth gain.
  s.outL = left * p.level; s.outR = right * p.level;
  return (s.outL + s.outR) * .5;
}
