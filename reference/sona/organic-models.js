/*
 * Physical-inspired exciters, not calibrated animal or fluid simulations.
 * Trumpet: two unstable tissue oscillators driven by an airstream; the precise
 * anatomical production mechanism of elephant trumpets is still unresolved.
 * Water: damped bubble modes dominate; impact/secondary spray are controllable
 * sound-design layers, not claims that every visible water motion is audible.
 * All random choices are seeded, and no objects are allocated in the sample loop.
 */
const TAU = Math.PI * 2;
const clamp = (x, lo, hi) => Math.max(lo, Math.min(hi, x));

export const ORGANIC_PARAMS = {
  reed: {
    frequency: [60, 1500, 380], breath: [0, 1, .35], growl: [0, 1, .4],
    pressure: [0, 1, .7], formant: [.5, 2, 1], instability: [0, 1, .25],
  },
  bubble: {
    frequency: [100, 9000, 2800], decay: [.02, 2, .11], tone: [0, 1, .55],
    repeat: [.2, 5, 1.2], scatter: [0, 1, .5], splash: [0, 1, .35],
  },
};

function noise(state) {
  let n = state.seed | 0;
  n ^= n << 13; n ^= n >>> 17; n ^= n << 5;
  state.seed = n >>> 0;
  return state.seed / 2147483648 - 1;
}

function resonator() {
  return { z1: 0, z2: 0, b0: 0, b2: 0, a1: 0, a2: 0 };
}

function tuneBandpass(filter, frequency, q, sr) {
  const omega = TAU * clamp(frequency, 30, sr * .42) / sr;
  const alpha = Math.sin(omega) / (2 * q), reciprocal = 1 / (1 + alpha);
  filter.b0 = alpha * reciprocal; filter.b2 = -filter.b0;
  filter.a1 = -2 * Math.cos(omega) * reciprocal;
  filter.a2 = (1 - alpha) * reciprocal;
}

function filterSample(filter, input) {
  const value = input * filter.b0 + filter.z1;
  filter.z1 = -filter.a1 * value + filter.z2;
  filter.z2 = input * filter.b2 - filter.a2 * value;
  return value;
}

function tissueFlow(phase, opening, closing) {
  // Smooth opening/closing of a pressure-driven aperture. Unlike a sawtooth,
  // the pressure pulse is derived from flow, with a finite closure interval.
  if (phase < opening) return .5 - .5 * Math.cos(Math.PI * phase / opening);
  if (phase < opening + closing) return .5 + .5 * Math.cos(Math.PI * (phase - opening) / closing);
  return 0;
}

function makeBubbleVoice() {
  const modes = [];
  for (let i = 0; i < 7; i++) modes.push({
    delay: 0, ratio: 1, amplitude: 0, damping: 1, phase: 0, cosPhase: 1,
    envelope: 0, rate: 0, sine: 0, cosine: 1, started: false, age: 0,
  });
  return {
    active: false, age: 0, pitch: 1, strength: 1, secondary: .05,
    modes, impactLow: 0, impactHigh: 0, lastOutput: 0,
  };
}

export function createOrganicState(type, sampleRate, seed = 1) {
  console.info('[Sona] Création de la source organique', type);
  const sr = clamp(Number(sampleRate) || 48000, 8000, 192000);
  const state = { type, sr, dt: 1 / sr, seed: (seed >>> 0) || 1, tick: 0, lastTime: -1, dcX: 0, dcY: 0 };
  if (type === 'reed') {
    Object.assign(state, {
      phaseA: .08, phaseB: .31, subPhase: 0, cycleA: 0,
      jitterA: 0, jitterB: 0, shimmerA: 1, shimmerB: 1,
      openA: .43, closeA: .17, openB: .46, closeB: .15,
      chaos: .471, slowNoise: 0, mediumNoise: 0, breathLow: 0,
      noiseLow: 0, flowA: 0, flowB: 0, sourceLow: 0, lowPressure: 0,
      frequency: 380, pressure: 0, pressureRise: 0, articulation: 0,
      formants: [resonator(), resonator(), resonator(), resonator()],
      airFilter: resonator(),
    });
    tuneBandpass(state.airFilter, 2300, .65, sr);
  } else if (type === 'bubble') {
    Object.assign(state, {
      voices: Array.from({ length: 8 }, makeBubbleVoice), voiceIndex: 0,
      nextEvent: 0, lastEvent: 0, eventSpacing: 1, lastRepeat: -1,
      eventCount: 0, frequency: 2800, tail: 0,
    });
  } else throw new Error('Source organique inconnue : ' + type);
  return state;
}

function trumpetSample(state, p) {
  const sr = state.sr, dt = state.dt, white = noise(state);
  // Aperiodic low-rate breath/pressure motions, not a repeating sine vibrato.
  state.slowNoise += (white - state.slowNoise) * (1 - Math.exp(-dt * 26));
  state.mediumNoise += (white - state.mediumNoise) * (1 - Math.exp(-dt * 350));
  state.noiseLow += (white - state.noiseLow) * .24;
  state.breathLow += (state.noiseLow - state.breathLow) * .012;
  const oldPressure = state.pressure;
  state.pressure += (p.pressure - state.pressure) * (1 - Math.exp(-dt * 60));
  state.pressureRise += ((state.pressure - oldPressure) * sr - state.pressureRise) * .002;
  state.frequency += (p.frequency - state.frequency) * (1 - Math.exp(-dt * 170));
  const pressure = state.pressure, instability = p.instability, growl = p.growl;
  const coupled = .011 * growl * Math.sin(TAU * state.phaseB);
  const drift = state.slowNoise * (1.3 + 1.8 * instability);
  const frequency = clamp(state.frequency * (1 + drift + state.jitterA + coupled), 30, sr * .12);
  const increment = frequency / sr;
  state.phaseA += increment;
  if (state.phaseA >= 1) {
    state.phaseA -= 1; state.cycleA++;
    state.chaos = 3.82 * state.chaos * (1 - state.chaos);
    const rough = .003 + .035 * instability;
    state.jitterA = noise(state) * rough + (state.chaos - .6) * .027 * growl;
    // Shimmer and alternating pulse strengths introduce subharmonics gradually.
    state.shimmerA = 1 + noise(state) * (.025 + .18 * instability)
      + ((state.cycleA & 1) ? 1 : -1) * growl * (.08 + .22 * state.chaos);
    state.openA = .43 + .055 * noise(state) * instability - .09 * pressure;
    state.closeA = .20 - .10 * pressure + .025 * noise(state) * instability;
  }
  const secondFrequency = clamp(state.frequency * (1.067 + .045 * state.slowNoise
    + state.jitterB + .007 * growl * Math.sin(TAU * state.phaseA)), 30, sr * .14);
  state.phaseB += secondFrequency / sr;
  if (state.phaseB >= 1) {
    state.phaseB -= 1;
    state.jitterB = noise(state) * (.005 + .035 * instability);
    state.shimmerB = 1 + noise(state) * (.04 + .2 * instability);
    state.openB = .40 + .07 * noise(state) * instability;
    state.closeB = .16 + .035 * noise(state) * instability;
  }
  const flowA = tissueFlow(state.phaseA, state.openA, state.closeA);
  const flowB = tissueFlow(state.phaseB, state.openB, state.closeB);
  // Radiation approximately differentiates the flow. Normalize the derivative
  // by phase increment to keep level useful as the user changes pitch.
  const first = (flowA - state.flowA) / Math.max(.001, increment) * .095 * state.shimmerA;
  const second = (flowB - state.flowB) / Math.max(.001, secondFrequency / sr) * .085 * state.shimmerB;
  state.flowA = flowA; state.flowB = flowB;
  state.subPhase += state.frequency * .5 / sr;
  if (state.subPhase >= 1) state.subPhase -= 1;
  const sub = Math.sin(TAU * state.subPhase) * .10 * growl * (1 + state.mediumNoise * 2);
  const nonlinearAir = (state.noiseLow - state.breathLow) * (.14 + .5 * flowA)
    * p.breath * (1 + instability + Math.max(0, state.pressureRise) * .04);
  const phonation = first + second * (.14 + .6 * growl) + sub;
  const tissue = Math.tanh(phonation * (1.1 + .55 * pressure + .35 * growl));
  state.sourceLow += (tissue - state.sourceLow) * (.25 + .32 * pressure);
  const excitation = state.sourceLow + nonlinearAir;
  if ((state.tick & 31) === 0) {
    const tract = p.formant * (1 + state.slowNoise * .45 + .045 * (pressure - .5));
    tuneBandpass(state.formants[0], 540 * tract, .85, sr);
    tuneBandpass(state.formants[1], 1230 * tract, 1.65, sr);
    tuneBandpass(state.formants[2], 2420 * tract, 2.3, sr);
    tuneBandpass(state.formants[3], 3900 * tract, 2.1, sr);
    tuneBandpass(state.airFilter, 1800 + pressure * 2400, .7, sr);
  }
  const body = filterSample(state.formants[0], excitation) * .52
    + filterSample(state.formants[1], excitation) * 1.06
    + filterSample(state.formants[2], excitation) * .65
    + filterSample(state.formants[3], excitation) * .23;
  const breath = filterSample(state.airFilter, white) * p.breath * (.16 + .12 * instability);
  const result = (body * 1.35 + excitation * .16 + breath) * Math.pow(pressure, .75);
  return Math.tanh(result * 1.05) * .70;
}

const WATER_RATIOS = [1, .73, 1.39, 2.13, 3.17, 1.81, 4.31];
const WATER_AMPS = [.54, .10, .115, .085, .05, .06, .035];
const WATER_DELAYS = [.009, .004, .017, .028, .047, .074, .095];
const WATER_DAMPS = [1, .7, .46, .34, .23, .30, .18];

function beginDroplet(state, p, time) {
  const voice = state.voices[state.voiceIndex];
  state.voiceIndex = (state.voiceIndex + 1) % state.voices.length;
  // Preserve a tiny decaying residual when extreme settings exhaust the pool.
  if (voice.active) state.tail += voice.lastOutput;
  voice.active = true; voice.age = 0;
  voice.pitch = 1 + noise(state) * .18 * p.scatter;
  voice.strength = .9 + noise(state) * .18 * p.scatter;
  voice.secondary = .055 + (noise(state) + 1) * .035;
  voice.impactLow = 0; voice.impactHigh = 0;
  for (let i = 0; i < voice.modes.length; i++) {
    const m = voice.modes[i], variation = noise(state) * p.scatter;
    m.delay = WATER_DELAYS[i] * (1 + variation * .28);
    m.ratio = WATER_RATIOS[i] * (1 + variation * (i ? .13 : .025));
    m.amplitude = WATER_AMPS[i] * voice.strength * (i ? .25 + 1.1 * p.splash : .62 + .38 * p.tone);
    m.damping = WATER_DAMPS[i] * (1 + noise(state) * p.scatter * .18);
    m.phase = 0; m.cosPhase = 1; m.envelope = 0; m.started = false; m.age = 0;
  }
  state.eventCount++;
  state.eventSpacing = 1 + noise(state) * .11 * p.scatter;
  state.lastEvent = time;
  state.lastRepeat = p.repeat;
  state.nextEvent = time + p.repeat * state.eventSpacing;
}

function dropletSample(state, p, time) {
  if (time < state.lastTime) state.nextEvent = 0;
  if (p.repeat !== state.lastRepeat && state.lastRepeat >= 0) {
    state.nextEvent = state.lastEvent + p.repeat * state.eventSpacing;
    state.lastRepeat = p.repeat;
  }
  if (time >= state.nextEvent) beginDroplet(state, p, time);
  state.lastTime = time;
  state.frequency += (p.frequency - state.frequency) * .006;
  const white = noise(state), sr = state.sr, dt = state.dt;
  const coefficientTick = (state.tick & 15) === 0;
  let sound = state.tail;
  state.tail *= .985;
  for (let v = 0; v < state.voices.length; v++) {
    const voice = state.voices[v];
    if (!voice.active) continue;
    const age = voice.age;
    let value = 0, envelopeSum = 0;
    for (let i = 0; i < voice.modes.length; i++) {
      const m = voice.modes[i];
      if (age < m.delay) { envelopeSum += 1; continue; }
      const start = !m.started;
      if (start) { m.started = true; m.envelope = 1; }
      if (m.envelope < .000015) continue;
      if (coefficientTick || start) {
        // A small upward relaxation of the entrained bubble frequency replaces
        // the exaggerated falling electronic "bloop" of the original model.
        const chirp = 1 - .035 * Math.exp(-m.age / .008);
        const f = Math.min(sr * .42, state.frequency * voice.pitch * m.ratio * chirp);
        const angular = TAU * f / sr;
        m.sine = Math.sin(angular); m.cosine = Math.cos(angular);
        const decay = Math.max(.003, p.decay * .5 * m.damping
          * Math.pow(1200 / Math.max(100, f), .32));
        m.rate = Math.exp(-dt / decay);
      }
      const sine = m.phase * m.cosine + m.cosPhase * m.sine;
      m.cosPhase = m.cosPhase * m.cosine - m.phase * m.sine;
      m.phase = sine;
      const attack = Math.min(1, m.age / (.0006 + .0002 * i));
      value += sine * m.envelope * m.amplitude * attack;
      m.envelope *= m.rate;
      m.age += dt;
      envelopeSum += m.envelope;
    }
    // A very short, band-limited wet transient and a weaker return splash.
    voice.impactLow += (white - voice.impactLow) * .09;
    voice.impactHigh += (white - voice.impactHigh) * .55;
    const wet = voice.impactLow * 1.8 + (voice.impactHigh - voice.impactLow) * .45;
    const initial = Math.exp(-age / .0027) * (1 - Math.exp(-age / .0002));
    const afterAge = age - voice.secondary;
    const after = afterAge > 0 ? Math.exp(-afterAge / .006) * Math.min(1, afterAge / .0004) : 0;
    value += wet * (initial * (.16 + p.splash * .46) + after * p.splash * .12) * voice.strength;
    voice.lastOutput = value;
    voice.age += dt;
    if (envelopeSum < .0001 && age > .14) voice.active = false;
    sound += value;
  }
  return Math.tanh(sound * 1.35) * .86;
}

export function organicSample(type, state, p, timeSeconds, frameIndex) {
  const result = type === 'reed' ? trumpetSample(state, p) : dropletSample(state, p, timeSeconds);
  state.tick++;
  // Keep asymmetric closure and pressure changes from adding DC downstream.
  const highpassed = result - state.dcX + .995 * state.dcY;
  state.dcX = result; state.dcY = highpassed;
  return Number.isFinite(highpassed) ? highpassed : 0;
}
