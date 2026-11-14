// Main-thread side of the worker tracer: an `AcousticTracer` whose answers arrive asynchronously.
// The engine asks each frame; a request is sent when a trace is due, the newest answer for each
// voice is handed over once when it lands. At most one frame is in flight: requests made meanwhile
// are merged (the newest wins) and sent when the answer comes back, so a slow worker never queues.
import type { Vec3 } from './acoustics.ts';
import type { SfxGeometry } from './trace-wasm.ts';
import type { AcousticTracer, ListenerResult, TracerListenerOptions, TracerVoiceOptions, VoiceGeometry } from './tracer.ts';

export type TraceWorkerIn =
  | { t: 'init'; module: WebAssembly.Module; geometry: SfxGeometry }
  | { t: 'geometry'; geometry: SfxGeometry }
  | {
    t: 'frame';
    id: number;
    listener: { position: Vec3; o: TracerListenerOptions } | null;
    voices: Array<{ key: number; source: Vec3; listener: Vec3; o: TracerVoiceOptions }>;
  };

export type TraceWorkerOut =
  | { t: 'ready'; nodes: number }
  | { t: 'error'; message: string }
  | { t: 'frame'; id: number; listener: ListenerResult | null; voices: Array<{ key: number; geo: VoiceGeometry }>; ms: number; casts: number };

export interface WorkerTracerStats {
  frames: number;
  /** Request-to-answer time (ms): mean and worst. */
  latencyMs: number;
  maxLatencyMs: number;
  /** Time the worker spent tracing per frame (ms): mean and worst. */
  workMs: number;
  maxWorkMs: number;
  casts: number;
}

export interface WorkerTracer extends AcousticTracer {
  readonly async: true;
  setGeometry(g: SfxGeometry): void;
  stats(): WorkerTracerStats;
}

/**
 * `worker`: a Worker running `@etcito/haa/trace-worker` (the host creates it so its bundler resolves
 * the URL); `module`: the compiled `haa-trace.wasm`.
 */
export async function createWorkerTracer(worker: Worker, module: WebAssembly.Module, geometry: SfxGeometry): Promise<WorkerTracer> {
  const post = (m: TraceWorkerIn): void => worker.postMessage(m);
  let listenerReq: { position: Vec3; o: TracerListenerOptions } | null = null;
  const voiceReq = new Map<number, { key: number; source: Vec3; listener: Vec3; o: TracerVoiceOptions }>();
  let latestListener: ListenerResult | null = null;
  let freshListener = false;
  const latest = new Map<number, VoiceGeometry>();
  const fresh = new Set<number>();
  let inFlight: { id: number; at: number } | null = null;
  let nextId = 1;
  const st = { frames: 0, latSum: 0, maxLat: 0, workSum: 0, maxWork: 0, casts: 0 };
  const send = (): void => {
    if (inFlight || (!listenerReq && voiceReq.size === 0)) return;
    const id = nextId++;
    post({ t: 'frame', id, listener: listenerReq, voices: [...voiceReq.values()] });
    inFlight = { id, at: performance.now() };
    listenerReq = null;
    voiceReq.clear();
  };
  await new Promise<void>((resolve, reject) => {
    worker.onmessage = (e: MessageEvent<TraceWorkerOut>) => {
      const m = e.data;
      if (m.t === 'ready') resolve();
      else if (m.t === 'error') {
        console.warn('[haa] trace worker:', m.message);
        reject(new Error(m.message));
      } else if (m.t === 'frame') {
        if (m.listener) {
          latestListener = m.listener;
          freshListener = true;
        }
        for (const v of m.voices) {
          latest.set(v.key, v.geo);
          fresh.add(v.key);
        }
        if (inFlight && inFlight.id === m.id) {
          const lat = performance.now() - inFlight.at;
          st.frames++;
          st.latSum += lat;
          st.maxLat = Math.max(st.maxLat, lat);
          st.workSum += m.ms;
          st.maxWork = Math.max(st.maxWork, m.ms);
          st.casts += m.casts;
          inFlight = null;
        }
        send();
      }
    };
    worker.onerror = (e) => reject(new Error(String(e.message)));
    post({ t: 'init', module, geometry });
  });
  return {
    async: true,
    listener(position, o, _segments, request = true) {
      if (request) listenerReq = { position, o };
      if (!freshListener) return null;
      freshListener = false;
      return latestListener;
    },
    voice(key, source, listener, o, _segments, request = true) {
      if (request) voiceReq.set(key, { key, source, listener, o });
      if (!fresh.has(key)) return null;
      fresh.delete(key);
      return latest.get(key) ?? null;
    },
    flush: send,
    forget(key) {
      latest.delete(key);
      fresh.delete(key);
      voiceReq.delete(key);
    },
    setGeometry(g) {
      post({ t: 'geometry', geometry: g });
    },
    stats() {
      const n = Math.max(1, st.frames);
      return { frames: st.frames, latencyMs: st.latSum / n, maxLatencyMs: st.maxLat, workMs: st.workSum / n, maxWorkMs: st.maxWork, casts: st.casts };
    },
    dispose() {
      worker.terminate();
    },
  };
}
