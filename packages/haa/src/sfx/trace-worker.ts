// Worker entry (`@etcito/haa/trace-worker`): runs the wasm acoustic tracer off the main thread.
// Protocol in trace-worker-client.ts. One frame = the listener trace (when due) and the voices
// that are due, answered in one message. No SharedArrayBuffer: works without cross-origin isolation.
import { createWasmTracer } from './trace-wasm.ts';
import type { WasmTracer } from './trace-wasm.ts';
import type { TraceWorkerIn, TraceWorkerOut } from './trace-worker-client.ts';

let tracer: WasmTracer | null = null;
const scope = globalThis as unknown as { onmessage: ((e: MessageEvent<TraceWorkerIn>) => void) | null; postMessage(m: TraceWorkerOut): void };

scope.onmessage = (e) => {
  const m = e.data;
  try {
    if (m.t === 'init') {
      tracer = createWasmTracer(m.module, m.geometry);
      scope.postMessage({ t: 'ready', nodes: tracer.nodes });
    } else if (m.t === 'geometry') {
      tracer?.setGeometry(m.geometry);
    } else if (m.t === 'frame' && tracer) {
      const t0 = performance.now();
      const casts0 = tracer.casts();
      const listener = m.listener ? tracer.listener(m.listener.position, m.listener.o) : null;
      const voices = m.voices.map((v) => ({ key: v.key, geo: tracer!.voice(v.key, v.source, v.listener, v.o)! }));
      scope.postMessage({ t: 'frame', id: m.id, listener, voices, ms: performance.now() - t0, casts: tracer.casts() - casts0 });
    }
  } catch (error) {
    scope.postMessage({ t: 'error', message: error instanceof Error ? error.message : String(error) });
  }
};
