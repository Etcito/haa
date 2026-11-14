// Worker entry (`@etcito/haa/mix-worker`): synthesises the voices of the slots it owns, spatialises
// each one in wasm (crates/haa-mix: distance delay and Doppler, air, occlusion bands, early
// reflections, extent, reverb sends and the FDN bank) and pre-mixes them onto the 8 virtual-speaker
// buses, kept `lookahead` frames ahead of the audio thread (mix-protocol.ts). No voice limit here
// but time: when the worker falls behind (its bus ring under half full) it renders only its
// highest-priority voices for that block; the others are frozen (a loop resumes later) or, for a
// one-shot thinned more than 0.25 s, silenced. The output never glitches.
import { HaaEngine } from '../engine.ts';
import type { HaaVoice } from '../engine.ts';
import { ctrlIndex, EPOCH, R_POS, W_POS } from './ring-protocol.ts';
import { MIX_BUSES, MIX_PARAMS, MIX_PRIORITY, P, STAT_STRIDE } from './mix-protocol.ts';
import type { MixWorkerIn, MixWorkerOut } from './mix-protocol.ts';

const Q = 128;

interface MixExports {
  memory: WebAssembly.Memory;
  mix_init(sr: number, voices: number): void;
  mix_params(v: number): number;
  mix_input(): number;
  mix_bus(): number;
  mix_reset_voice(v: number, fadeIn: number): void;
  mix_voice(v: number, n: number): void;
  mix_finish(n: number): void;
  mix_clear(): void;
}

interface Voice {
  slot: number;
  local: number;
  voice: HaaVoice;
  key: string | null;
  id: number;
  active: boolean;
  oneShot: boolean;
  timeline: boolean;
  framesLeft: number;
  /** Samples still mixed with silence after the voice's end (its delay line drains). */
  drain: number;
  sample: { l: Float32Array; r: Float32Array; pos: number; gain: number } | null;
  thinned: number;
  endAt: number;
  /** What a block of this voice costs (ms, moving average): synthesis + spatialisation. */
  costMs: number;
}

const scope = globalThis as unknown as { onmessage: ((e: MessageEvent<MixWorkerIn>) => void) | null; postMessage(m: MixWorkerOut): void };

let engine: HaaEngine | null = null;
let mx: MixExports;
let sr = 48000;
let lookahead = 2048;
let frames = 0;
let transport: 'sab' | 'port' = 'sab';
let index = 0;
let ctl: Int32Array | null = null;
let audio: Float32Array | null = null;
let params: Float32Array | null = null;
let stats: Int32Array | null = null;
let port: MessagePort | null = null;
/** Transport 'port': frames posted, and the worklet's read position. */
let sent = 0;
let readPort = 0;
/** Transport 'port': the parameters received from the main thread. */
const portParams = new Map<number, Float32Array>();
let disposed = false;
const presets = new Map<string, Uint8Array>();
const voices = new Map<number, Voice>();
const counters = { rendered: 0, thinned: 0, silenced: 0 };
let busyMs = 0;
let since = performance.now();
/** The reverb bank's cost per block (ms, moving average). */
let mixMs = 0;

function paramsOf(slot: number): Float32Array | null {
  if (params) return params.subarray(slot * MIX_PARAMS, slot * MIX_PARAMS + MIX_PARAMS);
  return portParams.get(slot) ?? null;
}

function readPos(): number {
  return transport === 'sab' ? Atomics.load(ctl!, ctrlIndex(index, R_POS)) : readPort;
}

function writePos(): number {
  return transport === 'sab' ? Atomics.load(ctl!, ctrlIndex(index, W_POS)) : sent;
}

function count(field: 'rendered' | 'thinned' | 'silenced', n = 1): void {
  counters[field] += n;
  if (stats) Atomics.add(stats, index * STAT_STRIDE + (field === 'rendered' ? 0 : field === 'thinned' ? 1 : 2), n);
}

function finish(v: Voice, w: number): void {
  v.active = false;
  v.sample = null;
  v.drain = 0;
  v.endAt = w;
}

/** Render one block of every owned voice into the bus; returns the 8-channel block (planar). */
function renderBlock(w: number, fill: number): Float32Array {
  mx.mix_clear();
  const live = [...voices.values()].filter((v) => v.active);
  live.sort((a, b) => (paramsOf(b.slot)?.[MIX_PRIORITY] ?? 0) - (paramsOf(a.slot)?.[MIX_PRIORITY] ?? 0));
  // A time budget per block: the voices, loudest first, while their measured costs fit in 75 % of
  // the block's real time (less when the ring is low); the others are thinned this block. The
  // worker then keeps pace by construction instead of falling behind and catching up.
  const blockMs = (Q / sr) * 1000;
  const budget = blockMs * 0.75 * Math.min(1, Math.max(0.25, fill / 0.5)) - mixMs;
  let spent = 0;
  const input = new Float32Array(mx.memory.buffer, mx.mix_input(), Q);
  for (let i = 0; i < live.length; i++) {
    const v = live[i];
    const fits = i === 0 || spent + v.costMs <= budget;
    if (fits) spent += v.costMs;
    if (!fits) {
      count('thinned');
      v.thinned += Q;
      if (v.oneShot && v.thinned > 0.25 * sr) {
        finish(v, w + Q);
        count('silenced');
      }
      continue;
    }
    // Resumed after thinning: its delay line is stale; clear it and fade in.
    if (v.thinned > 0) mx.mix_reset_voice(v.local, 1);
    v.thinned = 0;
    const p = paramsOf(v.slot);
    if (!p) continue;
    const t0 = performance.now();
    new Float32Array(mx.memory.buffer, mx.mix_params(v.local), MIX_PARAMS).set(p);
    let ended = false;
    if (v.drain > 0) {
      input.fill(0);
      v.drain -= Q;
      if (v.drain <= 0) ended = true;
    } else if (v.sample) {
      const sm = v.sample;
      const n = Math.max(0, Math.min(Q, sm.l.length - sm.pos));
      for (let k = 0; k < Q; k++) input[k] = k < n ? 0.5 * (sm.l[sm.pos + k] + sm.r[sm.pos + k]) * sm.gain : 0;
      sm.pos += Q;
      if (sm.pos >= sm.l.length) {
        v.sample = null;
        v.drain = Math.ceil(((p[P.DELAY] + 0.06) * sr) / Q) * Q;
      }
    } else {
      let playing = true;
      try {
        playing = v.voice.process(Q);
      } catch (error) {
        scope.postMessage({ t: 'error', message: error instanceof Error ? error.message : String(error) });
        playing = false;
      }
      const l = v.voice.left();
      const r = v.voice.right();
      for (let k = 0; k < Q; k++) input[k] = 0.5 * (l[k] + r[k]);
      if (v.oneShot) {
        v.framesLeft -= Q;
        if (v.framesLeft <= 0) playing = false;
      } else if (!playing && v.timeline) {
        v.voice.restart();
        playing = true;
      }
      if (!playing) v.drain = Math.ceil(((p[P.DELAY] + 0.06) * sr) / Q) * Q;
    }
    mx.mix_voice(v.local, Q);
    v.costMs += (performance.now() - t0 - v.costMs) * 0.1;
    count('rendered');
    if (ended) finish(v, w + Q);
  }
  const t1 = performance.now();
  mx.mix_finish(Q);
  mixMs += (performance.now() - t1 - mixMs) * 0.1;
  return new Float32Array(mx.memory.buffer, mx.mix_bus(), MIX_BUSES * Q);
}

function fill(): void {
  const t0 = performance.now();
  const read = readPos();
  let w = writePos();
  for (const [s, v] of voices) {
    if (v.endAt >= 0 && read >= v.endAt) {
      v.endAt = -1;
      scope.postMessage({ t: 'ended', slot: s, id: v.id });
    }
  }
  const target = transport === 'sab' ? Math.min(read + lookahead, read + frames - Q) : read + lookahead;
  let out: Float32Array | null = null;
  let outFrames = 0;
  if (transport === 'port') {
    const blocks = Math.max(0, Math.floor((target - w) / Q));
    if (blocks) out = new Float32Array(blocks * Q * MIX_BUSES);
  }
  while (w + Q <= target) {
    const bus = renderBlock(w, (w - read) / lookahead);
    if (transport === 'sab') {
      const base = index * frames * MIX_BUSES + (w % frames) * MIX_BUSES;
      for (let i = 0; i < Q; i++) for (let c = 0; c < MIX_BUSES; c++) audio![base + i * MIX_BUSES + c] = bus[c * Q + i];
      Atomics.store(ctl!, ctrlIndex(index, W_POS), w + Q);
    } else if (out) {
      const base = outFrames * MIX_BUSES;
      for (let i = 0; i < Q; i++) for (let c = 0; c < MIX_BUSES; c++) out[base + i * MIX_BUSES + c] = bus[c * Q + i];
      outFrames += Q;
    }
    w += Q;
  }
  if (out && outFrames) {
    port!.postMessage({ t: 'block', frames: outFrames, data: out }, [out.buffer]);
    sent = w;
  }
  if (ctl) Atomics.store(ctl, ctrlIndex(index, EPOCH), 1);
  busyMs += performance.now() - t0;
  const now = performance.now();
  if (now - since >= 1000) {
    scope.postMessage({ t: 'load', busy: busyMs / (now - since), ...counters });
    busyMs = 0;
    since = now;
  }
}

type WaitAsync = (a: Int32Array, i: number, v: number, t: number) => { async: boolean; value: Promise<string> | string };

async function loop(): Promise<void> {
  const waitAsync = (Atomics as unknown as { waitAsync?: WaitAsync }).waitAsync;
  while (!disposed) {
    const bell = ctl ? Atomics.load(ctl, 0) : 0;
    fill();
    if (ctl && waitAsync) {
      const r = waitAsync(ctl, 0, bell, 5);
      if (r.async) await r.value;
    } else {
      await new Promise((resolve) => setTimeout(resolve, transport === 'port' ? 4 : 2));
    }
  }
}

function startVoice(v: Voice, id: number, oneShot: boolean): void {
  v.id = id;
  v.oneShot = oneShot;
  v.endAt = -1;
  v.drain = 0;
  v.thinned = 0;
  v.active = true;
  mx.mix_reset_voice(v.local, 0);
}

scope.onmessage = (e) => {
  const m = e.data;
  try {
    switch (m.t) {
      case 'init': {
        engine = HaaEngine.fromModule(m.module);
        mx = new WebAssembly.Instance(m.mixModule, {}).exports as unknown as MixExports;
        sr = m.sampleRate;
        lookahead = m.lookahead;
        frames = m.ringFrames;
        transport = m.transport;
        index = m.index;
        mx.mix_init(sr, m.owned.length);
        if (m.transport === 'sab') {
          ctl = new Int32Array(m.control!);
          audio = new Float32Array(m.audio!);
          params = new Float32Array(m.params!);
          stats = new Int32Array(m.stats!);
        } else {
          port = m.port!;
          port.onmessage = (ev: MessageEvent<{ t: 'read'; pos: number }>) => {
            if (ev.data?.t === 'read') readPort = ev.data.pos;
          };
        }
        m.owned.forEach((slot, local) => {
          voices.set(slot, {
            slot, local, voice: engine!.createVoice(sr, (m.seed + slot * 7919) >>> 0), key: null, id: 0, active: false, oneShot: true, timeline: true,
            framesLeft: 0, drain: 0, sample: null, thinned: 0, endAt: -1, costMs: 0.05,
          });
        });
        scope.postMessage({ t: 'ready' });
        void loop();
        break;
      }
      case 'preset': {
        presets.set(m.key, m.bytes);
        for (const v of voices.values()) {
          if (v.key !== m.key) continue;
          if (v.active && !v.oneShot && !v.sample) v.voice.load(m.bytes, true);
          else v.key = null;
        }
        break;
      }
      case 'start': {
        const v = voices.get(m.slot);
        const bytes = presets.get(m.key);
        if (!v || !bytes) {
          scope.postMessage({ t: 'started', slot: m.slot, id: m.id, playable: false, loadMs: 0 });
          scope.postMessage({ t: 'ended', slot: m.slot, id: m.id });
          break;
        }
        let loadMs = 0;
        let playable = true;
        if (v.key !== m.key) {
          const t0 = performance.now();
          playable = v.voice.load(bytes, false).playable;
          loadMs = performance.now() - t0;
          v.key = playable ? m.key : null;
        }
        for (const name in m.inputs) v.voice.set(name, m.inputs[name]);
        v.voice.restart();
        const info = v.voice.info();
        v.timeline = info.mode === 'timeline';
        const seconds = m.oneShot ? Math.min(m.maxSeconds, info.mode === 'timeline' ? info.duration : m.maxSeconds) : Infinity;
        v.framesLeft = Math.max(1, Math.round(seconds * sr));
        v.sample = null;
        startVoice(v, m.id, m.oneShot);
        v.active = playable;
        scope.postMessage({ t: 'started', slot: m.slot, id: m.id, playable, loadMs });
        if (!playable) scope.postMessage({ t: 'ended', slot: m.slot, id: m.id });
        break;
      }
      case 'play': {
        const v = voices.get(m.slot);
        if (!v) break;
        startVoice(v, m.id, true);
        v.sample = { l: m.l, r: m.r, pos: 0, gain: m.gain };
        scope.postMessage({ t: 'started', slot: m.slot, id: m.id, playable: true, loadMs: 0 });
        break;
      }
      case 'set': {
        const v = voices.get(m.slot);
        if (!v || v.id !== m.id || !v.active || v.sample) break;
        for (const name in m.inputs) v.voice.set(name, m.inputs[name]);
        break;
      }
      case 'stop': {
        const v = voices.get(m.slot);
        if (!v || v.id !== m.id || (!v.active && v.endAt < 0)) break;
        v.active = false;
        v.sample = null;
        v.endAt = -1;
        scope.postMessage({ t: 'ended', slot: m.slot, id: m.id });
        break;
      }
      case 'params': {
        m.slots.forEach((s, i) => portParams.set(s, m.data.slice(i * MIX_PARAMS, i * MIX_PARAMS + MIX_PARAMS)));
        break;
      }
      case 'dispose':
        disposed = true;
        for (const v of voices.values()) v.voice.free();
        break;
    }
  } catch (error) {
    scope.postMessage({ t: 'error', message: error instanceof Error ? error.message : String(error) });
  }
};
