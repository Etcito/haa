// AudioWorkletGlobalScope only: the `haa-sfx-rings` processor. It plays the voices the render
// workers synthesise: per slot, 128 frames copied from a shared ring, nothing else. Its work per
// quantum is fixed (slots x 256 floats) whatever the voices are. A ring that has fallen behind
// (its worker is late) plays its last block faded out once, then silence, and fades back in when
// it has caught up: a late worker silences its voice for a moment, it never glitches the output.
import { ctrlIndex, EPOCH, R_POS, SFX_RINGS_PROCESSOR, STARVED, W_POS } from './ring-protocol.ts';
import type { SfxRingsProcessorOptions } from './ring-protocol.ts';

const Q = 128;

class HaaSfxRingsProcessor extends AudioWorkletProcessor {
  private readonly ctl: Int32Array;
  private readonly audio: Float32Array;
  private readonly slots: number;
  private readonly frames: number;
  /** Per slot: 0 silent, 1 playing; and its last block (for the fade-out of a starved ring). */
  private readonly playing: Uint8Array;
  private readonly last: Float32Array[];

  constructor(options?: AudioWorkletNodeOptions) {
    super(options);
    const o = options?.processorOptions as SfxRingsProcessorOptions;
    this.ctl = new Int32Array(o.control);
    this.audio = new Float32Array(o.audio);
    this.slots = o.slots;
    this.frames = o.ringFrames;
    this.playing = new Uint8Array(o.slots);
    this.last = Array.from({ length: o.slots }, () => new Float32Array(Q * 2));
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const ctl = this.ctl;
    for (let s = 0; s < this.slots; s++) {
      const out = outputs[s];
      if (!out || !out[0]) continue;
      const l = out[0];
      const r = out.length > 1 ? out[1] : out[0];
      const w = Atomics.load(ctl, ctrlIndex(s, W_POS));
      const rd = Atomics.load(ctl, ctrlIndex(s, R_POS));
      const avail = w - rd;
      const last = this.last[s];
      if (avail >= Q) {
        const base = (rd % this.frames) * 2;
        const a = this.audio;
        const offset = s * this.frames * 2;
        const fadeIn = this.playing[s] === 0;
        for (let i = 0; i < Q; i++) {
          const g = fadeIn ? i / Q : 1;
          const x = a[offset + base + i * 2] * g;
          const y = a[offset + base + i * 2 + 1] * g;
          l[i] = x;
          r[i] = y;
          last[i * 2] = x;
          last[i * 2 + 1] = y;
        }
        this.playing[s] = 1;
        Atomics.store(ctl, ctrlIndex(s, R_POS), rd + Q);
      } else if (this.playing[s] === 1) {
        // Starved (or stopped): the last block once more, faded out.
        for (let i = 0; i < Q; i++) {
          const g = 1 - i / Q;
          l[i] = last[i * 2] * g;
          r[i] = last[i * 2 + 1] * g;
        }
        this.playing[s] = 0;
        // A voice still active (its worker has not finished it) that ran dry: its worker is late.
        if (Atomics.load(ctl, ctrlIndex(s, EPOCH)) === 1) Atomics.add(ctl, ctrlIndex(s, STARVED), 1);
      }
    }
    Atomics.add(ctl, 1, 1);
    Atomics.add(ctl, 0, 1);
    Atomics.notify(ctl, 0);
    return true;
  }
}

const REGISTERED = '__haaSfxRingsRegistered';
const scope = globalThis as unknown as Record<string, boolean>;
if (!scope[REGISTERED]) {
  scope[REGISTERED] = true;
  registerProcessor(SFX_RINGS_PROCESSOR, HaaSfxRingsProcessor);
}
