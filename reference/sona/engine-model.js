/*
 * Pressure-event V12 model. This is a physically informed sound-design model,
 * not a calibrated simulation or a recording of a particular engine.
 * Twelve unequal exhaust releases per 720 crank degrees feed two lossy pipes.
 */
const TAU = 2 * Math.PI;
const FIRING_ORDER = [0, 6, 1, 7, 2, 8, 3, 9, 4, 10, 5, 11];

export const ENGINE_PARAMS = {
  rpm: [650, 9500, 1100],
  throttle: [0, 1, .6],
  roughness: [0, 1, .25],
  inertia: [.02, .8, .12],
  mechanics: [0, 1, .22],
  exhaust: [.25, 2, .75],
  crackle: [0, 1, .15],
};

function random(state) {
  let x = state.seed | 0;
  x ^= x << 13; x ^= x >>> 17; x ^= x << 5;
  state.seed = x >>> 0;
  return state.seed / 2147483648 - 1;
}

function delayed(buffer, index, samples) {
  let x = index - samples;
  if (x < 0) x += buffer.length;
  const i = Math.floor(x), f = x - i;
  return buffer[i] + (buffer[(i + 1) % buffer.length] - buffer[i]) * f;
}

export function createEngineState(sampleRate = 48000, seed = 1) {
  console.info('[Sona] Création du modèle de combustion V12');
  const sr = Math.max(8000, Math.min(192000, Number(sampleRate) || 48000));
  const state = {
    sr, invSr: 1 / sr, seed: (seed >>> 0) || 1, initialized: false,
    rpm: 1100, load: .6, exhaust: .75, firePhase: .99, firing: 0,
    drift: 0, airLow: 0, airMid: 0, airHigh: 0, intakeLow: 0,
    body: 0, radiatedLow: 0, outputLow: 0, dcIn: 0, dcOut: 0,
    pop: 0, popSlow: 0, popHigh: 0, popDecay: Math.exp(-1 / (sr * .018)),
    loadSmooth: 1 - Math.exp(-1 / (sr * .018)),
    driftSmooth: 1 - Math.exp(-1 / (sr * .024)),
    exhaustSmooth: 1 - Math.exp(-1 / (sr * .08)),
    dcCoeff: Math.exp(-TAU * 24 / sr),
    airLowCoeff: 1 - Math.exp(-TAU * 280 / sr),
    airMidCoeff: 1 - Math.exp(-TAU * 1050 / sr),
    intakeCoeff: 1 - Math.exp(-TAU * 1700 / sr),
    bodyCoeff: 1 - Math.exp(-TAU * 260 / sr),
    radiationCoeff: 1 - Math.exp(-TAU * 90 / sr),
    rpmSmooth: 1 - Math.exp(-1 / (sr * .12)),
    airHighCoeff: .55, outputCoeff: .62, headerCoeff: .45,
    lastInertia: -1, clock: 0,
    slow: new Float64Array(12), fast: new Float64Array(12),
    slowDecay: new Float64Array(12), fastDecay: new Float64Array(12),
    gain: new Float64Array(12), runner: new Float64Array(12),
    pending: new Int32Array(12), pendingGain: new Float64Array(12),
    pendingSlow: new Float64Array(12), pendingFast: new Float64Array(12),
    banks: [],
  };
  state.pending.fill(-1);
  for (let i = 0; i < 12; i++) {
    // Fixed cylinder variation supplies low engine orders without detuned oscillators.
    state.gain[i] = .93 + .07 * random(state) + .045 * Math.sin(i * 2.17);
    state.runner[i] = .19 + .045 * (i % 6) + .025 * random(state);
    state.slowDecay[i] = Math.exp(-1 / (sr * .0016));
    state.fastDecay[i] = Math.exp(-1 / (sr * .00012));
  }
  for (let bank = 0; bank < 2; bank++) {
    state.banks.push({
      header: new Float32Array(Math.ceil(sr * .012) + 4),
      tail: new Float32Array(Math.ceil(sr * .025) + 4),
      headerPos: 0, tailPos: 0, headerDamp: 0, tailDamp: 0,
      pressure: 0, output: 0,
    });
  }
  return state;
}

export function engineSample(s, p, timeSeconds, frameIndex) {
  const targetRpm = p.rpm ?? 1100;
  const targetLoad = p.throttle ?? .6;
  const roughness = p.roughness ?? .25;
  const inertia = p.inertia ?? .12;
  const mechanics = p.mechanics ?? .22;
  const pipeLength = p.exhaust ?? .75;
  const crackle = p.crackle ?? .15;

  if (!s.initialized) {
    s.rpm = targetRpm; s.load = targetLoad; s.exhaust = pipeLength;
    s.initialized = true;
  }
  // Expensive coefficient updates occur at control rate; parameters remain smooth.
  if ((s.clock++ & 31) === 0) {
    if (inertia !== s.lastInertia) {
      s.rpmSmooth = 1 - Math.exp(-1 / (s.sr * Math.max(.02, inertia)));
      s.lastInertia = inertia;
    }
    s.airHighCoeff = 1 - Math.exp(-TAU * Math.min(s.sr * .36, 3400 + s.rpm * .36 + s.load * 1900) / s.sr);
    s.outputCoeff = 1 - Math.exp(-TAU * Math.min(s.sr * .42, 6800 + s.rpm * .48) / s.sr);
    s.headerCoeff = 1 - Math.exp(-TAU * Math.min(s.sr * .32, 2900 + s.load * 3500) / s.sr);
  }
  s.rpm += (targetRpm - s.rpm) * s.rpmSmooth;
  s.load += (targetLoad - s.load) * s.loadSmooth;
  s.exhaust += (pipeLength - s.exhaust) * s.exhaustSmooth;
  const white = random(s);
  s.drift += (white - s.drift) * s.driftSmooth;
  const idleRoughness = 1 + 2.5 * Math.max(0, 1 - s.rpm / 2300);
  s.firePhase += s.rpm * s.invSr * .1 * (1 + s.drift * roughness * .055 * idleRoughness);
  if (s.firePhase >= 1) {
    s.firePhase -= 1;
    const cylinder = FIRING_ORDER[s.firing];
    s.firing = (s.firing + 1) % 12;
    const variance = random(s);
    const loadEnergy = .35 + .9 * s.load;
    const amplitude = loadEnergy * s.gain[cylinder] * (1 + variance * (.025 + roughness * .24) * idleRoughness);
    // The finite release has a quick front and a broad gas-pressure tail.
    // Its absolute duration shrinks at speed, but never degenerates into an impulse.
    const release = .00048 + .0019 * Math.pow(1200 / Math.max(800, s.rpm), .38) + s.load * .00035;
    const front = .000095 + .00008 * (1 - s.load) + .00003 * (variance + 1);
    s.pending[cylinder] = Math.max(1, Math.round(s.runner[cylinder] * s.sr / 520));
    s.pendingGain[cylinder] = amplitude;
    s.pendingSlow[cylinder] = Math.exp(-1 / (s.sr * release));
    s.pendingFast[cylinder] = Math.exp(-1 / (s.sr * front));
    const overrun = Math.max(0, Math.min(1, (s.rpm - targetRpm) / 1200)) * (1 - s.load);
    if (crackle > .001 && overrun > .05 && random(s) > 1 - crackle * overrun * .075) {
      s.pop = Math.min(1.5, s.pop + (.4 + .5 * random(s)) * crackle * (1 + overrun));
    }
  }

  let bankA = 0, bankB = 0;
  for (let c = 0; c < 12; c++) {
    if (s.pending[c] >= 0 && --s.pending[c] === 0) {
      s.slow[c] += s.pendingGain[c];
      s.fast[c] += s.pendingGain[c];
      s.slowDecay[c] = s.pendingSlow[c];
      s.fastDecay[c] = s.pendingFast[c];
      s.pending[c] = -1;
    }
    s.slow[c] *= s.slowDecay[c];
    s.fast[c] *= s.fastDecay[c];
    const pressure = s.slow[c] - s.fast[c];
    if (c < 6) bankA += pressure;
    else bankB += pressure;
  }

  // Gas flow is broadband and changes with both pressure and throttle.
  s.airLow += (white - s.airLow) * s.airLowCoeff;
  s.airMid += (white - s.airMid) * s.airMidCoeff;
  s.airHigh += (white - s.airHigh) * s.airHighCoeff;
  const gas = s.airHigh - s.airMid;
  const rumble = s.airLow;
  const gasLevel = .055 + .24 * s.load + .12 * roughness;
  bankA += gas * (.12 + bankA * .52) * gasLevel + rumble * roughness * .2;
  bankB += (gas * -.67 + random(s) * .12) * (.12 + bankB * .52) * gasLevel;

  s.pop *= s.popDecay;
  s.popSlow += (white - s.popSlow) * .09;
  s.popHigh += (white - s.popHigh) * .55;
  const afterfire = (s.popHigh - s.popSlow) * s.pop * 2;
  bankA += afterfire;
  bankB += afterfire * -.43;

  const raw = bankA + bankB;
  let exhaust = 0;
  for (let b = 0; b < 2; b++) {
    const bank = s.banks[b];
    const source = b ? bankB : bankA;
    const asymmetry = b ? 1.033 : .979;
    const headerTime = (.28 + s.exhaust * .37) * asymmetry * s.sr / 520;
    const tailTime = 2 * s.exhaust * asymmetry * s.sr / 495;
    const headerReturn = delayed(bank.header, bank.headerPos, headerTime);
    bank.headerDamp += (headerReturn - bank.headerDamp) * s.headerCoeff;
    const headerIn = source - bank.headerDamp * (.24 + .09 * s.load);
    bank.header[bank.headerPos] = headerIn;
    if (++bank.headerPos === bank.header.length) bank.headerPos = 0;
    const expanded = headerReturn * .71 + headerIn * .29;
    const tailReturn = delayed(bank.tail, bank.tailPos, tailTime);
    bank.tailDamp += (tailReturn - bank.tailDamp) * (.21 + .28 * s.load);
    // Negative open-end reflection produces damped pipe modes, not a pitched string.
    bank.tail[bank.tailPos] = expanded - bank.tailDamp * (.45 + .09 * s.load);
    if (++bank.tailPos === bank.tail.length) bank.tailPos = 0;
    bank.output = .68 * tailReturn + .32 * expanded;
    exhaust += bank.output * (b ? .96 : 1);
  }

  // The block and intake are quiet independent paths, not synthetic lead tones.
  s.body += (raw - s.body) * s.bodyCoeff;
  s.intakeLow += (white - s.intakeLow) * s.intakeCoeff;
  const cycle = (s.firing + s.firePhase) / 12;
  const shaft = Math.sin(TAU * cycle * 2);
  const valveOrder = Math.sin(TAU * cycle * 6);
  const mechanical = mechanics * (s.body * .2 + shaft * .025 + valveOrder * .017 + (gas - s.airLow) * raw * .085);
  const intake = (s.intakeLow - s.airLow) * (.06 + s.load * .19) * (.7 + .3 * Math.abs(valveOrder));
  s.radiatedLow += (exhaust - s.radiatedLow) * s.radiationCoeff;
  const radiated = exhaust - s.radiatedLow * .82;
  const mixed = radiated * .68 + mechanical + intake;
  const dc = mixed - s.dcIn + s.dcCoeff * s.dcOut;
  s.dcIn = mixed; s.dcOut = dc;
  s.outputLow += (dc - s.outputLow) * s.outputCoeff;
  // A smooth pressure ceiling prevents extreme editable settings from exploding.
  return Math.tanh(s.outputLow * 2.2) * .88;
}
