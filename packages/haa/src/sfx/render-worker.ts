// Worker entry (`@etcito/haa/render-worker`): synthesises the sfx voices of the slots it owns, off
// the audio thread, into the shared rings the `haa-sfx-rings` worklet plays (ring-protocol.ts). It
// keeps each active ring `lookahead` frames ahead of the worklet's read position, woken every audio
// quantum by the worklet's doorbell. A voice whose worker falls behind goes silent for a moment (the
// worklet fades it); the output never glitches. Same messages as the in-worklet pool.
import { HaaEngine } from '../engine.ts';
import type { HaaVoice } from '../engine.ts';
import { ctrlIndex, EPOCH, R_POS, W_POS } from './ring-protocol.ts';
import type { RenderWorkerIn, RenderWorkerOut } from './ring-protocol.ts';

const Q = 128;

interface Slot {
  voice: HaaVoice;
  key: string | null;
  id: number;
  active: boolean;
  oneShot: boolean;
  timeline: boolean;
  framesLeft: number;
  /** Write position where the voice ended: 'ended' is posted once the worklet has read past it. */
  endAt: number;
  /** A baked one-shot played from shared samples instead of the synthesis. */
  sample: { l: Float32Array; r: Float32Array; pos: number; gain: number } | null;
}

const scope = globalThis as unknown as {
  onmessage: ((e: MessageEvent<RenderWorkerIn>) => void) | null;
  postMessage(m: RenderWorkerOut): void;
};

let engine: HaaEngine | null = null;
let ctl: Int32Array;
let audio: Float32Array;
let frames = 0;
let lookahead = 1024;
let disposed = false;
const presets = new Map<string, Uint8Array>();
const slots = new Map<number, Slot>();
let sr = 48000;
let busyMs = 0;
let rendered = 0;
let since = performance.now();

function fill(): void {
  for (const [s, v] of slots) {
    if (v.endAt >= 0) {
      if (Atomics.load(ctl, ctrlIndex(s, R_POS)) >= v.endAt) {
        v.endAt = -1;
        scope.postMessage({ t: 'ended', slot: s, id: v.id });
      }
      continue;
    }
    if (!v.active) continue;
    const wi = ctrlIndex(s, W_POS);
    const read = Atomics.load(ctl, ctrlIndex(s, R_POS));
    let w = Atomics.load(ctl, wi);
    // Never write over what the worklet has not read; keep `lookahead` ahead of it.
    const target = Math.min(read + lookahead, read + frames - Q);
    const t0 = performance.now();
    const offset = s * frames * 2;
    while (w + Q <= target) {
      let playing = true;
      const base = offset + (w % frames) * 2;
      const sm = v.sample;
      if (sm) {
        // A baked one-shot: copy its samples, then silence to the end of the block.
        const n = Math.max(0, Math.min(Q, sm.l.length - sm.pos));
        for (let i = 0; i < Q; i++) {
          audio[base + i * 2] = i < n ? sm.l[sm.pos + i] * sm.gain : 0;
          audio[base + i * 2 + 1] = i < n ? sm.r[sm.pos + i] * sm.gain : 0;
        }
        sm.pos += Q;
        w += Q;
        if (sm.pos >= sm.l.length) {
          v.sample = null;
          v.active = false;
          v.endAt = w;
          break;
        }
        continue;
      }
      try {
        playing = v.voice.process(Q);
      } catch (error) {
        v.active = false;
        scope.postMessage({ t: 'error', message: error instanceof Error ? error.message : String(error) });
        break;
      }
      const l = v.voice.left();
      const r = v.voice.right();
      for (let i = 0; i < Q; i++) {
        audio[base + i * 2] = l[i];
        audio[base + i * 2 + 1] = r[i];
      }
      w += Q;
      rendered += Q;
      if (v.oneShot) {
        v.framesLeft -= Q;
        if (v.framesLeft <= 0) playing = false;
      } else if (!playing && v.timeline) {
        v.voice.restart();
        playing = true;
      }
      if (!playing) {
        v.active = false;
        v.endAt = w;
        break;
      }
    }
    Atomics.store(ctl, wi, w);
    if (!v.active) Atomics.store(ctl, ctrlIndex(s, EPOCH), 0);
    busyMs += performance.now() - t0;
  }
  const now = performance.now();
  if (now - since >= 1000) {
    scope.postMessage({ t: 'load', busy: busyMs / (now - since), frames: rendered });
    busyMs = 0;
    rendered = 0;
    since = now;
  }
}

type WaitAsync = (a: Int32Array, i: number, v: number, t: number) => { async: boolean; value: Promise<string> | string };

async function loop(): Promise<void> {
  const waitAsync = (Atomics as unknown as { waitAsync?: WaitAsync }).waitAsync;
  while (!disposed) {
    const bell = Atomics.load(ctl, 0);
    fill();
    // Wake on the next quantum (or within 5 ms); messages are handled between two waits.
    if (waitAsync) {
      const r = waitAsync(ctl, 0, bell, 5);
      if (r.async) await r.value;
    } else {
      await new Promise((resolve) => setTimeout(resolve, 2));
    }
  }
}

function reset(s: number): void {
  // Drop what is queued: the worklet stops at the current read position.
  Atomics.store(ctl, ctrlIndex(s, W_POS), Atomics.load(ctl, ctrlIndex(s, R_POS)));
}

scope.onmessage = (e) => {
  const m = e.data;
  try {
    switch (m.t) {
      case 'init': {
        engine = HaaEngine.fromModule(m.module);
        ctl = new Int32Array(m.control);
        audio = new Float32Array(m.audio);
        frames = m.ringFrames;
        lookahead = m.lookahead;
        for (const s of m.owned) {
          sr = m.sampleRate;
          slots.set(s, { voice: engine.createVoice(sr, (m.seed + s * 7919) >>> 0), key: null, id: 0, active: false, oneShot: true, timeline: true, framesLeft: 0, endAt: -1, sample: null });
        }
        scope.postMessage({ t: 'ready' });
        void loop();
        break;
      }
      case 'preset': {
        presets.set(m.key, m.bytes);
        for (const v of slots.values()) {
          if (v.key !== m.key) continue;
          if (v.active && !v.oneShot) v.voice.load(m.bytes, true);
          else v.key = null;
        }
        break;
      }
      case 'start': {
        const v = slots.get(m.slot);
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
        v.id = m.id;
        v.oneShot = m.oneShot;
        v.endAt = -1;
        v.sample = null;
        v.active = playable;
        reset(m.slot);
        Atomics.store(ctl, ctrlIndex(m.slot, EPOCH), playable ? 1 : 0);
        scope.postMessage({ t: 'started', slot: m.slot, id: m.id, playable, loadMs });
        if (!playable) scope.postMessage({ t: 'ended', slot: m.slot, id: m.id });
        else fill();
        break;
      }
      case 'play': {
        const v = slots.get(m.slot);
        if (!v) break;
        v.id = m.id;
        v.oneShot = true;
        v.endAt = -1;
        v.sample = { l: m.l, r: m.r, pos: 0, gain: m.gain };
        v.active = true;
        reset(m.slot);
        Atomics.store(ctl, ctrlIndex(m.slot, EPOCH), 1);
        scope.postMessage({ t: 'started', slot: m.slot, id: m.id, playable: true, loadMs: 0 });
        fill();
        break;
      }
      case 'set': {
        const v = slots.get(m.slot);
        if (!v || v.id !== m.id || !v.active) break;
        for (const name in m.inputs) v.voice.set(name, m.inputs[name]);
        break;
      }
      case 'stop': {
        const v = slots.get(m.slot);
        if (!v || v.id !== m.id || (!v.active && v.endAt < 0)) break;
        v.active = false;
        v.endAt = -1;
        v.sample = null;
        Atomics.store(ctl, ctrlIndex(m.slot, EPOCH), 0);
        reset(m.slot);
        scope.postMessage({ t: 'ended', slot: m.slot, id: m.id });
        break;
      }
      case 'dispose':
        disposed = true;
        for (const v of slots.values()) v.voice.free();
        break;
    }
  } catch (error) {
    scope.postMessage({ t: 'error', message: error instanceof Error ? error.message : String(error) });
  }
};

