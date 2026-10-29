/* Deterministic modular synthesis shared by AudioWorklet and the WAV renderer. */
import { ENGINE_PARAMS, createEngineState, engineSample } from './engine-model.js';
import { ORGANIC_PARAMS, createOrganicState, organicSample } from './organic-models.js';
import { EFFECT_PARAMS, createEffectState, processEffect } from './effects.js';
import { SIMPLE_EFFECT_PARAMS, createSimpleEffectState, processSimpleEffect } from './simple-effects.js';
import { PULSE_PARAMS, PULSE_ENUMS, createPulseState, pulseSample } from './primitives.js';
import { GRAIN_PARAMS, GRAIN_ENUMS, createGrainState, grainSample } from './grains.js';
import { RESPONSE_PARAMS, RESPONSE_ENUMS, createControlResponseState, controlResponseSample } from './control-response.js';
import { MOTION_PARAMS, createControlMotionState, controlMotionSample } from './control-motion.js';
import { MATH_PARAMS, MATH_ENUMS, mathSample } from './control-math.js';
import { NOISE_PARAMS, createNoiseState, noiseSample } from './noise.js';
import { CONTROL_TYPES } from './modulation-trace.js';

const TAU = Math.PI * 2;
const SIMPLE_EFFECT_TYPES = new Set(Object.keys(SIMPLE_EFFECT_PARAMS));
const EFFECT_TYPES = new Set([...Object.keys(EFFECT_PARAMS), ...SIMPLE_EFFECT_TYPES]);
const TYPES = new Set(['grain', 'pulse', 'engine', 'reed', 'bubble', 'oscillator', 'noise', ...CONTROL_TYPES, ...EFFECT_TYPES]);
const PARAMS = {
  pulse: PULSE_PARAMS,
  grain: GRAIN_PARAMS,
  engine: ENGINE_PARAMS,
  ...ORGANIC_PARAMS,
  oscillator: { frequency: [20, 12000, 220], level: [0, 1, .5] },
  noise: NOISE_PARAMS,
  curve: { amount: [0, 1, 1] },
  response: RESPONSE_PARAMS,
  motion: MOTION_PARAMS,
  lfo: { rate: [.05, 2000, 1], depth: [0, 1, 1] },
  random: { rate: [.05, 2000, 2], depth: [0, 1, .5], smooth: [0, 1, .7] },
  envelope: { attack: [.001, 3, .12], decay: [.001, 3, .2], sustain: [0, 1, .6], release: [.01, 4, .8], hold: [.01, 8, 1] },
  math: MATH_PARAMS,
  ...EFFECT_PARAMS,
  ...SIMPLE_EFFECT_PARAMS,
};
const ENUMS = {
  pulse: PULSE_ENUMS,
  grain: GRAIN_ENUMS,
  response: RESPONSE_ENUMS,
  oscillator: { wave: ['sine', 'saw', 'square', 'triangle'] },
  noise: { color: ['white', 'pink', 'brown', 'blue', 'violet'] },
  lfo: { wave: ['sine', 'triangle', 'square'] },
  filter: { mode: ['lowpass', 'bandpass', 'highpass'] },
  math: MATH_ENUMS,
};
const clamp = (x, lo, hi) => Math.min(hi, Math.max(lo, x));
const finite = (x, label) => { if (!Number.isFinite(x)) throw new Error(`${label} doit être un nombre fini.`); return x; };

/** Return an independent, normalized graph. Reject invalid ports and dependency cycles. */
export function validateGraph(graph) {
  if (!graph || !Array.isArray(graph.nodes) || !Array.isArray(graph.connections)) throw new Error('Format de projet audio invalide.');
  if (graph.nodes.length > 64 || graph.connections.length > 256) throw new Error('Maximum : 64 nœuds et 256 connexions.');
  const ids = new Set();
  const nodes = graph.nodes.map(node => {
    if (!node || typeof node.id !== 'string' || !node.id || ids.has(node.id)) throw new Error('Chaque nœud doit avoir un identifiant unique.');
    if (!TYPES.has(node.type)) throw new Error(`Type de nœud inconnu : ${node.type}`);
    ids.add(node.id);
    const params = {};
    for (const [key, value] of Object.entries(node.params || {})) {
      if (typeof value === 'number') finite(value, key);
      else if (typeof value !== 'string' && typeof value !== 'boolean') throw new Error(`Paramètre invalide : ${key}`);
    }
    for (const [key, [lo, hi, fallback]] of Object.entries(PARAMS[node.type])) params[key] = clamp(finite(Number(node.params?.[key] ?? fallback), key), lo, hi);
    for (const [key, options] of Object.entries(ENUMS[node.type] || {})) {
      const value = node.params?.[key] ?? options[0];
      if (!options.includes(value)) throw new Error(`Valeur invalide pour ${key}.`);
      params[key] = value;
    }
    let curve = node.curve ?? [[0, 0], [.2, .8], [.75, 1], [1, 0]];
    if (node.type === 'curve') {
      if (!Array.isArray(curve) || curve.length < 2 || curve.length > 128) throw new Error('Une courbe doit contenir entre 2 et 128 points.');
      curve = curve.map(point => {
        if (!Array.isArray(point) || point.length < 2) throw new Error('Point de courbe invalide.');
        return [clamp(finite(point[0], 'Temps de la courbe'), 0, 1), clamp(finite(point[1], 'Valeur de la courbe'), 0, 1)];
      }).sort((a, b) => a[0] - b[0]);
    }
    return { ...node, params, curve };
  });
  const byId = new Map(nodes.map(n => [n.id, n]));
  const edgeIds = new Set();
  const connections = graph.connections.map((edge, i) => {
    if (!edge || !ids.has(edge.from) || !ids.has(edge.to)) throw new Error('Une connexion pointe vers un nœud absent.');
    if (edge.from === edge.to) throw new Error('Un nœud ne peut pas se connecter à lui-même.');
    const source = byId.get(edge.from), target = byId.get(edge.to);
    if (source.type === 'output') throw new Error('La sortie ne peut pas alimenter un autre nœud.');
    const port = edge.target || 'audio';
    if (port === 'audio') {
      if (CONTROL_TYPES.has(source.type) || !EFFECT_TYPES.has(target.type)) throw new Error('Cette connexion audio est incompatible.');
    } else if (!CONTROL_TYPES.has(source.type) || !Object.hasOwn(PARAMS[target.type], port)) throw new Error(`Cette modulation est incompatible : ${port}.`);
    const id = edge.id || `edge-${i}`;
    if (edgeIds.has(id)) throw new Error('Chaque connexion doit avoir un identifiant unique.');
    edgeIds.add(id);
    return { ...edge, id, target: port, amount: clamp(finite(Number(edge.amount ?? 1), 'Profondeur de modulation'), -20000, 20000) };
  });
  const incoming = new Map(nodes.map(n => [n.id, 0]));
  const outgoing = new Map(nodes.map(n => [n.id, []]));
  for (const edge of connections) { incoming.set(edge.to, incoming.get(edge.to) + 1); outgoing.get(edge.from).push(edge.to); }
  const queue = nodes.filter(n => incoming.get(n.id) === 0).map(n => n.id);
  for (let i = 0; i < queue.length; i++) for (const next of outgoing.get(queue[i])) { const count = incoming.get(next) - 1; incoming.set(next, count); if (!count) queue.push(next); }
  if (queue.length !== nodes.length) throw new Error('Connexion refusée : une boucle de dépendance serait créée.');
  const duration = clamp(finite(Number(graph.duration ?? 8), 'Durée'), .25, 60);
  return { ...graph, ...(graph.performance ? {performance:{...graph.performance}} : {}), nodes, connections, duration, loop: graph.loop !== false, order: queue };
}

function seedFromId(id) {
  let s = 2166136261;
  for (let i = 0; i < id.length; i++) s = Math.imul(s ^ id.charCodeAt(i), 16777619);
  return s >>> 0 || 1;
}
function random(state) {
  let n = state.seed | 0; n ^= n << 13; n ^= n >>> 17; n ^= n << 5; state.seed = n >>> 0;
  return (state.seed / 2147483648) - 1;
}
function blep(t, dt) {
  if (t < dt) { const u = t / dt; return u + u - u * u - 1; }
  if (t > 1 - dt) { const u = (t - 1) / dt; return u * u + u + u + 1; }
  return 0;
}
function waveform(phase, dt, wave) {
  if (wave === 'saw') return 2 * phase - 1 - blep(phase, dt);
  if (wave === 'square') return (phase < .5 ? 1 : -1) + blep(phase, dt) - blep((phase + .5) % 1, dt);
  if (wave === 'triangle') return 1 - 4 * Math.abs(phase - .5);
  return Math.sin(TAU * phase);
}
function makeState(node, sr) {
  const seed = seedFromId(node.id);
  if (node.type === 'pulse') return createPulseState(sr, seed);
  if (node.type === 'grain') return createGrainState(sr, seed);
  if (node.type === 'response') return createControlResponseState(sr);
  if (node.type === 'motion') return createControlMotionState(sr);
  if (node.type === 'noise') return createNoiseState(seed);
  if (node.type === 'engine') return createEngineState(sr, seed);
  if (node.type === 'reed' || node.type === 'bubble') return createOrganicState(node.type, sr, seed);
  if (SIMPLE_EFFECT_TYPES.has(node.type)) return createSimpleEffectState(node.type, sr);
  if (EFFECT_TYPES.has(node.type)) return createEffectState(node.type, sr);
  const state = { seed, phase: 0, noise: 0, brown: 0, pink0: 0, pink1: 0, pink2: 0, previousPink: 0, previousWhite: 0, randomStart: 0, randomTarget: 0 };
  if (node.type === 'random') state.randomTarget = random(state);
  return state;
}
function curveAt(points, t) {
  if (t <= points[0][0]) return points[0][1];
  for (let i = 1; i < points.length; i++) {
    if (t <= points[i][0]) {
      const a = points[i - 1], b = points[i], u = (t - a[0]) / Math.max(.000001, b[0] - a[0]);
      return a[1] + (b[1] - a[1]) * u;
    }
  }
  return points[points.length - 1][1];
}

/** process(left,right[,length]) fills caller-owned stereo Float32Arrays. */
export class GraphDSP {
  constructor(graph, sampleRate = 48000) {
    this.sampleRate = clamp(Number(sampleRate) || 48000, 8000, 192000);
    this.frame = 0; this.totalFrames = 0; this.startFrames = 0; this.position = 0; this.ended = false;
    this.dcInL = this.dcInR = this.dcOutL = this.dcOutR = 0;
    this.dcCoefficient = Math.exp(-TAU * 5 / this.sampleRate);
    this.limiter = 1; this.limiterRelease = 1 - Math.exp(-1 / (.06 * this.sampleRate));
    this.paramSmoothing = 1 - Math.exp(-1 / (.005 * this.sampleRate));
    this.states = new Map(); this.compiled = [];
    this.setGraph(graph, false);
  }
  setGraph(graph, preserveState = true) {
    const clean = validateGraph(graph);
    const clockModeChanged = (this.graph?.performance?.mode === 'live') !== (clean.performance?.mode === 'live');
    const old = new Map(this.compiled.map(n => [n.id, n]));
    this.graph = clean; this.durationFrames = Math.max(1, Math.round(clean.duration * this.sampleRate));
    this.frame %= this.durationFrames; this.ended = false;
    const byId = new Map(clean.nodes.map(n => [n.id, n]));
    const order = clean.order.map(id => byId.get(id));
    const indices = new Map(order.map((n, i) => [n.id, i]));
    const oldEdges = new Map();
    if (preserveState) for (const n of this.compiled) for (const edge of n.audio) oldEdges.set(edge.id, edge.current);
    this.compiled = order.map(node => {
      const previous = preserveState && old.get(node.id)?.type === node.type ? old.get(node.id) : null;
      const p = {};
      for (const [key, range] of Object.entries(PARAMS[node.type])) p[key] = { base: node.params[key], value: previous?.p[key]?.value ?? node.params[key], min: range[0], max: range[1], edges: [] };
      return { ...node, p, paramList: Object.keys(p), v: {}, state: previous?.state || makeState(node, this.sampleRate), audio: [], effect: EFFECT_TYPES.has(node.type) };
    });
    if (preserveState && clockModeChanged) {
      const precedingTime = ((clean.performance?.mode === 'live' ? this.totalFrames : this.frame)-1)/this.sampleRate;
      for (const node of this.compiled) {
        if (node.type === 'response' || node.type === 'grain') node.state.lastTime = precedingTime;
        // A macro changes between an automated edge and a manual base. Rebase
        // that bookkeeping value; the control node's own response stays intact.
        if (node.id === clean.performance?.node) {
          const parameter=node.p[clean.performance.parameter];
          if(parameter) parameter.value=parameter.base;
        }
      }
    }
    for (const edge of clean.connections) {
      const n = this.compiled[indices.get(edge.to)], from = indices.get(edge.from);
      if (edge.target === 'audio') n.audio.push({ id: edge.id, from, amount: edge.amount, current: oldEdges.get(edge.id) ?? edge.amount });
      else n.p[edge.target].edges.push({ from, amount: edge.amount });
    }
    this.values = new Float64Array(this.compiled.length);
    this.leftValues = new Float64Array(this.compiled.length); this.rightValues = new Float64Array(this.compiled.length);
    this.outputIndices = this.compiled.map((n, i) => n.type === 'output' ? i : -1).filter(i => i >= 0);
    this.states = new Map(this.compiled.map(n => [n.id, n.state]));
    if (this.modulationRecorder) this.modulationRecorder.bind(this);
  }
  setPosition(seconds = 0) {
    this.frame = Math.floor(clamp(Number(seconds) || 0, 0, this.graph.duration) * this.sampleRate) % this.durationFrames;
    this.position = this.frame / this.sampleRate;
    this.totalFrames = this.frame; this.startFrames = 0; this.ended = false;
    this.dcInL = this.dcInR = this.dcOutL = this.dcOutR = 0; this.limiter = 1;
    if (this.modulationRecorder) this.modulationRecorder.bind(this);
  }
  process(left, right = left, length = left.length) {
    const sr = this.sampleRate, dt = 1 / sr, nodes = this.compiled, values = this.values, signalL = this.leftValues, signalR = this.rightValues;
    const smoothing = this.paramSmoothing;
    const continuousControl = this.graph.performance?.mode === 'live';
    for (let sample = 0; sample < length; sample++) {
      if (this.ended) { left[sample] = 0; right[sample] = 0; continue; }
      const time = this.frame / sr, normalizedTime = this.frame / this.durationFrames;
      const controlTime = continuousControl ? this.totalFrames / sr : time;
      for (let index = 0; index < nodes.length; index++) {
        const n = nodes[index], s = n.state, p = n.v;
        for (let parameter = 0; parameter < n.paramList.length; parameter++) {
          const key = n.paramList[parameter], q = n.p[key];
          q.value += (q.base - q.value) * smoothing;
          let value = q.value;
          for (let e = 0; e < q.edges.length; e++) value += values[q.edges[e].from] * q.edges[e].amount;
          p[key] = clamp(value, q.min, q.max);
        }
        let inputL = 0, inputR = 0;
        for (let e = 0; e < n.audio.length; e++) {
          const edge = n.audio[e]; edge.current += (edge.amount - edge.current) * smoothing;
          inputL += signalL[edge.from] * edge.current; inputR += signalR[edge.from] * edge.current;
        }
        if (n.muted) { values[index] = signalL[index] = signalR[index] = 0; continue; }
        if ((n.bypass || n.bypassed) && n.effect) { signalL[index] = inputL; signalR[index] = inputR; values[index] = (inputL + inputR) * .5; continue; }
        if (n.effect) {
          if (SIMPLE_EFFECT_TYPES.has(n.type)) processSimpleEffect(n.type, s, inputL, inputR, p);
          else processEffect(n.type, s, inputL, inputR, p, this.totalFrames, n.params.mode);
          signalL[index] = s.outL; signalR[index] = s.outR; values[index] = (s.outL + s.outR) * .5;
          continue;
        }
        let value = 0;
        switch (n.type) {
          case 'pulse': p.shape = n.params.shape; value = pulseSample(s, p, time, this.totalFrames); break;
          case 'grain': p.wave = n.params.wave; value = grainSample(s, p, controlTime); break;
          case 'response': p.mode = n.params.mode; value = controlResponseSample(s, p, controlTime); break;
          case 'motion': value = controlMotionSample(s, p, time, this.totalFrames); break;
          case 'math': value = mathSample(p, n.params.operation); break;
          case 'curve': value = curveAt(n.curve, normalizedTime) * p.amount; break;
          case 'lfo':
            s.phase = (s.phase + p.rate * dt) % 1;
            value = waveform(s.phase, p.rate * dt, n.params.wave) * p.depth;
            break;
          case 'random': {
            s.phase += p.rate * dt;
            if (s.phase >= 1) { s.phase -= 1; s.randomStart = s.randomTarget; s.randomTarget = random(s); }
            const u = p.smooth <= .001 ? 1 : Math.min(1, s.phase / p.smooth), blend = u * u * (3 - 2 * u);
            value = (s.randomStart + (s.randomTarget - s.randomStart) * blend) * p.depth;
            break;
          }
          case 'envelope': {
            const a = p.attack, d = p.decay, h = p.hold, r = p.release;
            if (time < a) value = time / a;
            else if (time < a + d) value = 1 - (1 - p.sustain) * (time - a) / d;
            else if (time < a + d + h) value = p.sustain;
            else value = p.sustain * Math.max(0, 1 - (time - a - d - h) / r);
            break;
          }
          case 'oscillator': {
            const step = Math.min(.45, p.frequency * dt);
            s.phase = (s.phase + step) % 1;
            value = waveform(s.phase, step, n.params.wave) * p.level * .65;
            break;
          }
          case 'noise': {
            value = noiseSample(s, p, n.params.color);
            break;
          }
          case 'engine': value = engineSample(s, p, time, this.totalFrames); break;
          case 'reed': case 'bubble': value = organicSample(n.type, s, p, time, this.totalFrames); break;
        }
        values[index] = Number.isFinite(value) ? value : 0;
        if(n.type === 'grain' || n.type === 'noise') { signalL[index] = Number.isFinite(s.outL) ? s.outL : 0; signalR[index] = Number.isFinite(s.outR) ? s.outR : 0; }
        else signalL[index] = signalR[index] = values[index];
      }
      if (this.modulationRecorder) this.modulationRecorder.capture(values, this.totalFrames);
      let mixedL = 0, mixedR = 0;
      for (let o = 0; o < this.outputIndices.length; o++) { mixedL += signalL[this.outputIndices[o]]; mixedR += signalR[this.outputIndices[o]]; }
      const l = mixedL - this.dcInL + this.dcCoefficient * this.dcOutL;
      const r = mixedR - this.dcInR + this.dcCoefficient * this.dcOutR;
      this.dcInL = mixedL; this.dcInR = mixedR; this.dcOutL = l; this.dcOutR = r;
      const peak = Math.max(Math.abs(l), Math.abs(r)), target = peak > .92 ? .92 / peak : 1;
      this.limiter = target < this.limiter ? target : this.limiter + (target - this.limiter) * this.limiterRelease;
      const fade = continuousControl && this.graph.loop
        ? Math.max(0,Math.min(1,this.startFrames/(sr*.005)))
        : Math.max(0, Math.min(1, this.startFrames / (sr * .005), this.frame / (sr * .003), (this.durationFrames - this.frame) / (sr * .006)));
      const gain = this.limiter * fade;
      if (right === left) left[sample] = (l + r) * .5 * gain;
      else { left[sample] = l * gain; right[sample] = r * gain; }
      this.frame++; this.totalFrames++; this.startFrames++;
      if (this.frame >= this.durationFrames) {
        if (this.graph.loop) this.frame = 0;
        else { this.frame = this.durationFrames; this.ended = true; }
      }
    }
    this.position = this.frame / sr;
    return !this.ended;
  }
}

export function renderSamples(graph, { sampleRate = 48000, duration = graph.duration ?? 8, master = .65 } = {}) {
  console.info('[SonaLab] Rendu audio déterministe', { duration, sampleRate });
  const checked = validateGraph({ ...graph, duration, loop: false });
  const dsp = new GraphDSP(checked, sampleRate);
  const length = Math.ceil(checked.duration * sampleRate), left = new Float32Array(length), right = new Float32Array(length);
  dsp.process(left, right);
  const gain = clamp(Number(master) || 0, 0, 1);
  for (let i = 0; i < length; i++) { left[i] *= gain; right[i] *= gain; }
  return { left, right, sampleRate, duration: checked.duration };
}
