// AudioWorkletGlobalScope only: the `haa-sfx-mixbus` processor. It sums the render workers'
// pre-mixed buses (8 channels each) into its one 8-channel output: per quantum, workers x 8 x 128
// samples added, whatever the scene. A worker that is late (its ring or queue dry) plays its last
// block faded out once, then silence until it catches up: its voices vanish for a moment, the output
// never glitches. Transport 'sab' reads shared rings; transport 'port' queues the blocks each worker
// posts on its own MessagePort and posts its read position back.
import { ctrlIndex, R_POS, STARVED, W_POS } from './ring-protocol.ts';
import { MIX_BUSES, MIX_PROCESSOR } from './mix-protocol.ts';
import type { MixProcessorOptions } from './mix-protocol.ts';

const Q = 128;
const C = MIX_BUSES;

class HaaSfxMixBusProcessor extends AudioWorkletProcessor {
  private readonly o: MixProcessorOptions;
  private readonly ctl: Int32Array | null;
  private readonly audio: Float32Array | null;
  private readonly playing: Uint8Array;
  private readonly last: Float32Array[];
  /** Transport 'port': per worker, the queued blocks, the offset in the first, frames consumed. */
  private readonly queues: Array<Array<Float32Array>> = [];
  private readonly offsets: number[] = [];
  private readonly consumed: number[] = [];
  private readonly ports: Array<MessagePort | null> = [];
  private quanta = 0;
  starved = 0;

  constructor(options?: AudioWorkletNodeOptions) {
    super(options);
    this.o = options?.processorOptions as MixProcessorOptions;
    this.ctl = this.o.control ? new Int32Array(this.o.control) : null;
    this.audio = this.o.audio ? new Float32Array(this.o.audio) : null;
    this.playing = new Uint8Array(this.o.workers);
    this.last = Array.from({ length: this.o.workers }, () => new Float32Array(Q * C));
    for (let w = 0; w < this.o.workers; w++) {
      this.queues.push([]);
      this.offsets.push(0);
      this.consumed.push(0);
      this.ports.push(null);
    }
    this.port.onmessage = (e: MessageEvent<{ t: 'port'; worker: number; port: MessagePort }>) => {
      const m = e.data;
      if (m?.t !== 'port') return;
      this.ports[m.worker] = m.port;
      m.port.onmessage = (ev: MessageEvent<{ t: 'block'; frames: number; data: Float32Array }>) => {
        if (ev.data?.t === 'block') this.queues[m.worker].push(ev.data.data);
      };
    };
  }

  /** The next Q frames of worker `w` into `blk` (interleaved), or false when it has not got them. */
  private take(w: number, blk: Float32Array): boolean {
    if (this.ctl && this.audio) {
      const rd = Atomics.load(this.ctl, ctrlIndex(w, R_POS));
      if (Atomics.load(this.ctl, ctrlIndex(w, W_POS)) - rd < Q) return false;
      const base = w * this.o.ringFrames * C + (rd % this.o.ringFrames) * C;
      blk.set(this.audio.subarray(base, base + Q * C));
      Atomics.store(this.ctl, ctrlIndex(w, R_POS), rd + Q);
      return true;
    }
    const q = this.queues[w];
    let have = -this.offsets[w];
    for (const b of q) {
      have += b.length / C;
      if (have >= Q) break;
    }
    if (have < Q) return false;
    let need = Q;
    let o = 0;
    while (need > 0) {
      const b = q[0];
      const off = this.offsets[w];
      const n = Math.min(need, b.length / C - off);
      blk.set(b.subarray(off * C, (off + n) * C), o * C);
      o += n;
      need -= n;
      if (off + n >= b.length / C) {
        q.shift();
        this.offsets[w] = 0;
      } else this.offsets[w] = off + n;
    }
    this.consumed[w] += Q;
    return true;
  }

  private readonly blk = new Float32Array(Q * C);

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const out = outputs[0];
    if (!out || out.length < C) return true;
    for (let c = 0; c < C; c++) out[c].fill(0);
    const blk = this.blk;
    for (let w = 0; w < this.o.workers; w++) {
      const last = this.last[w];
      if (this.take(w, blk)) {
        const fadeIn = this.playing[w] === 0;
        for (let i = 0; i < Q; i++) {
          const g = fadeIn ? i / Q : 1;
          for (let c = 0; c < C; c++) {
            const x = blk[i * C + c] * g;
            out[c][i] += x;
            last[i * C + c] = x;
          }
        }
        this.playing[w] = 1;
      } else if (this.playing[w] === 1) {
        for (let i = 0; i < Q; i++) {
          const g = 1 - i / Q;
          for (let c = 0; c < C; c++) out[c][i] += last[i * C + c] * g;
        }
        this.playing[w] = 0;
        if (this.ctl) Atomics.add(this.ctl, ctrlIndex(w, STARVED), 1);
        else this.starved++;
      }
    }
    this.quanta++;
    if (this.ctl) {
      Atomics.add(this.ctl, 1, 1);
      Atomics.add(this.ctl, 0, 1);
      Atomics.notify(this.ctl, 0);
    } else if (this.quanta % 4 === 0) {
      for (let w = 0; w < this.o.workers; w++) this.ports[w]?.postMessage({ t: 'read', pos: this.consumed[w] });
      if (this.quanta % 400 === 0) this.port.postMessage({ t: 'starved', n: this.starved });
    }
    return true;
  }
}

const REGISTERED = '__haaSfxMixBusRegistered';
const scope = globalThis as unknown as Record<string, boolean>;
if (!scope[REGISTERED]) {
  scope[REGISTERED] = true;
  registerProcessor(MIX_PROCESSOR, HaaSfxMixBusProcessor);
}
