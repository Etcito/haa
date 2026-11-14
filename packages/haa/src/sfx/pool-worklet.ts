// AudioWorkletGlobalScope only: the `haa-sfx-pool` processor. One wasm instance, `slots` voices,
// one stereo output per slot. A slot keeps its last patch loaded, so restarting the same preset
// costs no load; only idle slots are skipped, so the audio thread pays for playing voices only.
import { HaaEngine } from '../engine.ts';
import type { HaaVoice } from '../engine.ts';
import { BLOCK } from '../types.ts';
import { SFX_POOL_PROCESSOR } from './pool-protocol.ts';
import type { SfxPoolInMsg, SfxPoolOutMsg, SfxPoolProcessorOptions } from './pool-protocol.ts';

interface Slot {
  voice: HaaVoice;
  key: string | null;
  id: number;
  active: boolean;
  oneShot: boolean;
  /** Frames left before a one-shot stops (a looping timeline plays one pass). */
  framesLeft: number;
  timeline: boolean;
  loop: boolean;
}

const now = (): number => (globalThis as { performance?: { now(): number } }).performance?.now() ?? Date.now();

class HaaSfxPoolProcessor extends AudioWorkletProcessor {
  private readonly engine: HaaEngine;
  private readonly slots: Slot[] = [];
  private readonly presets = new Map<string, Uint8Array>();
  private disposed = false;

  constructor(options?: AudioWorkletNodeOptions) {
    super(options);
    const o = options?.processorOptions as SfxPoolProcessorOptions;
    this.engine = HaaEngine.fromModule(o.module);
    for (let i = 0; i < o.slots; i++) {
      this.slots.push({
        voice: this.engine.createVoice(sampleRate, (o.seed + i * 7919) >>> 0),
        key: null,
        id: 0,
        active: false,
        oneShot: true,
        framesLeft: 0,
        timeline: true,
        loop: false,
      });
    }
    this.port.onmessage = (event: MessageEvent<SfxPoolInMsg>) => this.onMessage(event.data);
    this.send({ t: 'ready', slots: o.slots });
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    if (this.disposed) return false;
    for (let i = 0; i < this.slots.length; i++) {
      const out = outputs[i];
      if (!out || !out[0]) continue;
      const left = out[0];
      const right = out.length > 1 ? out[1] : null;
      const s = this.slots[i];
      if (!s.active) continue; // outputs start zeroed
      const n = Math.min(BLOCK, left.length);
      try {
        let playing = s.voice.process(n);
        const l = s.voice.left();
        const r = s.voice.right();
        left.set(l.subarray(0, n));
        if (right) right.set(r.subarray(0, n));
        if (s.oneShot) {
          s.framesLeft -= n;
          if (s.framesLeft <= 0) playing = false;
        } else if (!playing && s.timeline) {
          // A continuous slot loops a timeline that has no loop of its own.
          s.voice.restart();
          playing = true;
        }
        if (!playing) {
          s.active = false;
          this.send({ t: 'ended', slot: i, id: s.id });
        }
      } catch (error) {
        s.active = false;
        s.key = null;
        this.send({ t: 'error', message: error instanceof Error ? error.message : String(error) });
        this.send({ t: 'ended', slot: i, id: s.id });
      }
    }
    return true;
  }

  private send(msg: SfxPoolOutMsg): void {
    this.port.postMessage(msg);
  }

  private onMessage(msg: SfxPoolInMsg): void {
    try {
      switch (msg.t) {
        case 'preset': {
          this.presets.set(msg.key, msg.bytes);
          for (const s of this.slots) {
            if (s.key !== msg.key) continue;
            if (s.active && !s.oneShot) {
              // A live edit of a looping preset: hot-swap, keeping the node state.
              s.voice.load(msg.bytes, true);
              this.readInfo(s);
            } else {
              s.key = null;
            }
          }
          break;
        }
        case 'start': {
          const s = this.slots[msg.slot];
          const bytes = this.presets.get(msg.key);
          if (!s || !bytes) {
            this.send({ t: 'started', slot: msg.slot, id: msg.id, playable: false, loadMs: 0 });
            if (s) this.send({ t: 'ended', slot: msg.slot, id: msg.id });
            break;
          }
          let loadMs = 0;
          let playable = true;
          if (s.key !== msg.key) {
            const t0 = now();
            playable = s.voice.load(bytes, false).playable;
            loadMs = now() - t0;
            s.key = playable ? msg.key : null;
            this.readInfo(s);
          }
          s.id = msg.id;
          s.oneShot = msg.oneShot;
          for (const name in msg.inputs) s.voice.set(name, msg.inputs[name]);
          s.voice.restart();
          const info = s.voice.info();
          const seconds = msg.oneShot ? Math.min(msg.maxSeconds, info.mode === 'timeline' ? info.duration : msg.maxSeconds) : Infinity;
          s.framesLeft = Math.max(1, Math.round(seconds * sampleRate));
          s.active = playable;
          this.send({ t: 'started', slot: msg.slot, id: msg.id, playable, loadMs });
          if (!playable) this.send({ t: 'ended', slot: msg.slot, id: msg.id });
          break;
        }
        case 'set': {
          const s = this.slots[msg.slot];
          if (!s || s.id !== msg.id || !s.active) break;
          for (const name in msg.inputs) s.voice.set(name, msg.inputs[name]);
          break;
        }
        case 'stop': {
          const s = this.slots[msg.slot];
          if (!s || s.id !== msg.id || !s.active) break;
          s.active = false;
          this.send({ t: 'ended', slot: msg.slot, id: msg.id });
          break;
        }
        case 'dispose':
          this.disposed = true;
          for (const s of this.slots) s.voice.free();
          break;
      }
    } catch (error) {
      this.send({ t: 'error', message: error instanceof Error ? error.message : String(error) });
    }
  }

  private readInfo(s: Slot): void {
    const info = s.voice.info();
    s.timeline = info.mode === 'timeline';
    s.loop = info.loop;
  }
}

const REGISTERED = '__haaSfxPoolRegistered';
const scope = globalThis as unknown as Record<string, boolean>;
if (!scope[REGISTERED]) {
  scope[REGISTERED] = true;
  registerProcessor(SFX_POOL_PROCESSOR, HaaSfxPoolProcessor);
}
