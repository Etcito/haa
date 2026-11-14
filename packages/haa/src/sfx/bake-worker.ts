// Worker entry (`@etcito/haa/bake-worker`): the orchestrator's background renders, never on the
// main thread or the audio thread. One HAA engine instance renders a preset at fixed inputs into
// samples (a baked one-shot), or a reverb impulse response for the bank.
import { HaaEngine } from '../engine.ts';
import type { HaaVoice } from '../engine.ts';
import { renderReverbIrData } from './spatial3d.ts';
import type { BakeWorkerIn, BakeWorkerOut } from './orchestrator.ts';

let engine: HaaEngine | null = null;
const voices = new Map<number, { voice: HaaVoice; key: string }>();
const scope = globalThis as unknown as {
  onmessage: ((e: MessageEvent<BakeWorkerIn>) => void) | null;
  postMessage(m: BakeWorkerOut, transfer?: Transferable[]): void;
};

/** A voice per seed, keeping its last patch (a preset baked at many cells loads once per seed). */
function voiceFor(seed: number, sr: number, key: string, bytes: Uint8Array): HaaVoice | null {
  let v = voices.get(seed);
  if (!v) {
    v = { voice: engine!.createVoice(sr, seed), key: '' };
    voices.set(seed, v);
  }
  if (v.key !== key) {
    if (!v.voice.load(bytes).playable) return null;
    v.key = key;
  }
  return v.voice;
}

scope.onmessage = (e) => {
  const m = e.data;
  try {
    if (m.t === 'init') {
      engine = HaaEngine.fromModule(m.module);
      scope.postMessage({ t: 'ready' });
      return;
    }
    if (!engine) throw new Error('bake worker not initialised');
    const t0 = performance.now();
    if (m.t === 'ir') {
      const [l, r] = renderReverbIrData(engine, m.sampleRate, m.rt60, m.absorption);
      scope.postMessage({ t: 'done', id: m.id, l, r, rms: 0, ms: performance.now() - t0 }, [l.buffer, r.buffer]);
      return;
    }
    // A one-shot at fixed inputs, `seconds` long (the pool's own cut), trailing silence trimmed.
    const voice = voiceFor(m.seed, m.sampleRate, m.presetKey, m.bytes);
    if (!voice) {
      scope.postMessage({ t: 'error', id: m.id, message: 'not playable' });
      return;
    }
    for (const name in m.inputs) voice.set(name, m.inputs[name]);
    voice.restart();
    const frames = Math.max(128, Math.round(m.seconds * m.sampleRate));
    const l = new Float32Array(frames);
    const r = new Float32Array(frames);
    let sum = 0;
    let last = 0;
    for (let f = 0; f < frames; f += 128) {
      const n = Math.min(128, frames - f);
      voice.process(n);
      const a = voice.left();
      const b = voice.right();
      for (let i = 0; i < n; i++) {
        const x = a[i];
        const y = b[i];
        l[f + i] = x;
        r[f + i] = y;
        const e = x * x + y * y;
        sum += e;
        if (e > 1e-10) last = f + i;
      }
    }
    // Trim below -100 dB, with a 5 ms fade where the render was cut.
    const end = Math.min(frames, last + Math.round(0.005 * m.sampleRate) + 1);
    const fade = Math.min(end, Math.round(0.005 * m.sampleRate));
    for (let i = end - fade; i < end; i++) {
      const g = (end - i) / fade;
      l[i] *= g;
      r[i] *= g;
    }
    const lt = l.slice(0, end);
    const rt = r.slice(0, end);
    scope.postMessage({ t: 'done', id: m.id, l: lt, r: rt, rms: Math.sqrt(sum / (2 * frames)), ms: performance.now() - t0 }, [lt.buffer, rt.buffer]);
  } catch (error) {
    scope.postMessage({ t: 'error', id: 'id' in m ? m.id : -1, message: error instanceof Error ? error.message : String(error) });
  }
};
