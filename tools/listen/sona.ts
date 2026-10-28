// Sona A-side adapter. Protocol: reference/sona/audio-worklet.js,
// SonaGraphProcessor.constructor/process; reference/sona/audio.js,
// AudioEngine.start/setGraph/setMaster/stop; reference/sona/app.js,
// setPerformanceValue; reference/sona/modulation-trace.js, ModulationRecorder.drain.

/** Playback controls and control-node observations for the Sona reference. */
export interface SonaSide {
  setPedal(v: number): void;
  onTrace(cb: (seconds: number, values: Record<string, number>) => void): () => void;
  dispose(): void;
}

type TraceListener = (seconds: number, values: Record<string, number>) => void;
type ModulationPacket = {
  type: 'modulation';
  sampleRate: number;
  time: number;
  sequence: number;
  traces: { id: string; points: [number, number, number, number][] }[];
};

const MODULES = new WeakMap<AudioContext, Promise<void>>();

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Create Sona playback; null means “Sona: n/a (HAA-only nodes)”. */
export async function createSonaSide(
  ctx: AudioContext,
  graph: Record<string, unknown>,
  out: GainNode,
  master = 1,
): Promise<SonaSide | null> {
  let node: AudioWorkletNode | undefined;
  let masterNode: GainNode | undefined;
  let disposed = false;
  const listeners = new Set<TraceListener>();

  function dispose(): void {
    if (disposed) return;
    disposed = true;
    listeners.clear();
    const active = node;
    const gain = masterNode;
    if (active) {
      active.port.onmessage = null;
      active.onprocessorerror = null;
      try { active.port.postMessage({ type: 'stop' }); } catch { /* Port already closed. */ }
    }
    // Allow Sona's 12 ms stop fade to finish, as AudioEngine.stop does.
    setTimeout(() => {
      active?.disconnect();
      active?.port.close();
      gain?.disconnect();
    }, 25);
  }

  try {
    const url = '/reference/sona/catalog.js';
    const { NODE_TYPES } = await import(url) as {
      NODE_TYPES: Record<string, {
        category: string;
        params: { key: string; modulatable?: boolean; options?: unknown; min: number; max: number }[];
      }>;
    };
    if (!Array.isArray(graph.nodes) || !graph.nodes.every(isRecord)) {
      throw new Error('Invalid graph nodes');
    }
    if (graph.nodes.some(n => typeof n.type !== 'string' || !Object.hasOwn(NODE_TYPES, n.type))) {
      return null;
    }

    // Own the editable snapshot so pedal gestures do not mutate the caller's patch.
    const current = structuredClone(graph);
    const nodes = current.nodes as Record<string, unknown>[];
    const connections = (current.connections ?? []) as Record<string, unknown>[];
    let sequence = 0;
    let pending = MODULES.get(ctx);
    if (!pending) {
      pending = ctx.audioWorklet.addModule('/reference/sona/audio-worklet.js');
      MODULES.set(ctx, pending);
      // Keep even a rejected load cached: exactly one attempt per context.
      void pending.catch(() => {});
    }
    await pending;

    masterNode = ctx.createGain();
    masterNode.gain.value = Math.min(1, Math.max(0, Number(master) || 0));
    node = new AudioWorkletNode(ctx, 'sonalab-graph', {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
      processorOptions: { graph: current, offset: 0, sequence },
    });
    const active = node;
    active.port.onmessage = ({ data }: MessageEvent<ModulationPacket | { type: 'ended' | 'error' }>) => {
      if (disposed) return;
      if (data.type === 'ended' || data.type === 'error') {
        dispose();
        return;
      }
      if (data.type !== 'modulation' || data.sequence !== sequence) return;
      // All traces share bucket timestamps; preserve each bucket, including flushes.
      const buckets = new Map<number, Record<string, number>>();
      for (const trace of data.traces) {
        for (const [seconds, , , value] of trace.points) {
          let values = buckets.get(seconds);
          if (!values) {
            values = Object.create(null) as Record<string, number>;
            buckets.set(seconds, values);
          }
          values[trace.id] = value;
        }
      }
      for (const [seconds, values] of buckets) {
        for (const listener of listeners) {
          try { listener(seconds, values); } catch { /* Keep other observers running. */ }
        }
      }
    };
    active.onprocessorerror = dispose;
    active.connect(masterNode);
    masterNode.connect(out);

    return {
      setPedal(v): void {
        const performance = current.performance;
        if (disposed || !Number.isFinite(v) || !isRecord(performance) || performance.mode !== 'live') return;
        const target = nodes.find(n => n.id === performance.node);
        if (!target || target.muted || !isRecord(target.params)) return;
        const schema = NODE_TYPES[target.type as string];
        const param = schema.params.find(p => p.key === performance.parameter);
        if (schema.category !== 'control' || !param?.modulatable || param.options || param.min > 0 || param.max < 1) return;
        const curve = nodes.find(n => n.id === performance.curveNode && n.type === 'curve');
        const edge = curve && connections.find(c => c.id === performance.edge
          && c.from === curve.id && c.to === target.id && c.target === param.key);
        const value = Math.min(1, Math.max(0, v));
        performance.manualValue = value;
        target.params[param.key] = value;
        if (edge) edge.amount = 0;
        active.port.postMessage({ type: 'graph', graph: current, sequence: ++sequence });
      },
      onTrace(cb): () => void {
        if (!disposed) listeners.add(cb);
        return () => { listeners.delete(cb); };
      },
      dispose,
    };
  } catch (error) {
    dispose();
    throw new Error(`[listen] Sona unavailable: ${error instanceof Error ? error.message : String(error)}`);
  }
}
