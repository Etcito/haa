// AudioWorkletGlobalScope only: the `haa-sfx-master` processor, the sfx mix's last stage. It runs the
// look-ahead true-peak limiter of haa.wasm (crates/haa-core dsp::master_limiter: ceiling -1 dBFS, 3 ms
// look-ahead, 4x oversampled peaks) on the stereo mix, so that no sum of voices reaches the device above
// the ceiling, and posts its overruns (input samples over the ceiling) about every 100 ms.
import { MASTER_PROCESSOR } from './master-protocol.ts';
import type { MasterProcessorOptions, MasterStatsMsg } from './master-protocol.ts';

const Q = 128;
const REPORT_QUANTA = 37; // ~100 ms at 48 kHz

interface MasterExports {
  memory: WebAssembly.Memory;
  haa_master_new(sampleRate: number): number;
  haa_master_left(): number;
  haa_master_right(): number;
  haa_master_process(n: number): void;
  haa_master_stats(): number;
}

class HaaSfxMasterProcessor extends AudioWorkletProcessor {
  private readonly x: MasterExports | null;
  private quanta = 0;

  constructor(options?: AudioWorkletNodeOptions) {
    super(options);
    const o = options?.processorOptions as MasterProcessorOptions;
    let x: MasterExports | null = null;
    try {
      x = new WebAssembly.Instance(o.module, {}).exports as unknown as MasterExports;
      x.haa_master_new(sampleRate);
    } catch {
      x = null;
    }
    this.x = x;
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const input = inputs[0];
    const out = outputs[0];
    if (!out || out.length === 0) return true;
    const inL = input?.[0];
    const inR = input?.[1] ?? inL;
    const x = this.x;
    if (!x) {
      // No wasm: pass through clamped at the ceiling (never louder than the limiter would allow).
      for (let c = 0; c < out.length; c++) {
        const src = c === 0 ? inL : inR;
        for (let i = 0; i < out[c].length; i++) out[c][i] = src ? Math.max(-0.891, Math.min(0.891, src[i])) : 0;
      }
      return true;
    }
    const n = Math.min(Q, out[0].length);
    // The memory can grow: views are taken per call.
    const mem = x.memory.buffer;
    const l = new Float32Array(mem, x.haa_master_left(), Q);
    const r = new Float32Array(mem, x.haa_master_right(), Q);
    if (inL) l.set(inL.subarray(0, n)); else l.fill(0);
    if (inR) r.set(inR.subarray(0, n)); else r.fill(0);
    x.haa_master_process(n);
    const l2 = new Float32Array(x.memory.buffer, x.haa_master_left(), Q);
    const r2 = new Float32Array(x.memory.buffer, x.haa_master_right(), Q);
    out[0].set(l2.subarray(0, n));
    if (out[1]) out[1].set(r2.subarray(0, n));
    if (++this.quanta % REPORT_QUANTA === 0) {
      const s = new Float32Array(x.memory.buffer, x.haa_master_stats(), 3);
      const msg: MasterStatsMsg = { t: 'stats', over: s[0], peakDb: s[1], gainDb: s[2] };
      this.port.postMessage(msg);
    }
    return true;
  }
}

const REGISTERED = '__haaSfxMasterRegistered';
const scope = globalThis as unknown as Record<string, boolean>;
if (!scope[REGISTERED]) {
  scope[REGISTERED] = true;
  registerProcessor(MASTER_PROCESSOR, HaaSfxMasterProcessor);
}
