// AudioWorkletGlobalScope only: the `haa-player` processor. It runs one wasm voice at the context
// rate, renders at most BLOCK frames per call and talks to player.ts through the port (ARCHITECTURE §9).
import { HaaEngine } from './engine.ts';
import type { HaaVoice } from './engine.ts';
import { BLOCK, PLAYER_PROCESSOR, TRACE_HZ } from './types.ts';
import type { PatchIssue, PlayerInMsg, PlayerOutMsg, PlayerProcessorOptions, VoiceInfo } from './types.ts';

const EMPTY_INFO: VoiceInfo = { nodes: [], inputs: [], mode: 'timeline', duration: 1, loop: false };

class HaaPlayerProcessor extends AudioWorkletProcessor {
  private readonly module: WebAssembly.Module;
  private readonly seed: number;
  private readonly traceEvery = Math.round(sampleRate / TRACE_HZ);
  private engine: HaaEngine | null = null;
  private voice: HaaVoice | null = null;
  private alive = false;
  private playable = false;
  private trapped = false;
  private disposed = false;
  private tracing: boolean;
  private endedPosted = false;
  private frames = 0;

  constructor(options?: AudioWorkletNodeOptions) {
    super(options);
    const o = options?.processorOptions as PlayerProcessorOptions;
    this.module = o.module;
    this.seed = o.seed;
    this.tracing = o.trace;
    let playable = false;
    let issues: PatchIssue[] = [];
    let info = EMPTY_INFO;
    try {
      this.rebuild();
      const voice = this.voice!;
      const result = voice.load(o.patch, false);
      playable = result.playable;
      issues = result.issues;
      for (const name of Object.keys(o.inputs)) voice.set(name, o.inputs[name]);
      info = voice.info();
      this.playable = playable;
      this.alive = playable;
    } catch (error) {
      this.trap(error);
    }
    this.send({ t: 'ready', playable, issues, info });
    this.port.onmessage = (event: MessageEvent<PlayerInMsg>) => this.onMessage(event.data);
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    if (this.disposed) return false;
    const out = outputs[0];
    const left = out[0];
    if (!left) return true;
    const right = out.length > 1 ? out[1] : null;
    const n = Math.min(BLOCK, left.length);
    let rendered = false;
    if (this.alive && this.voice) {
      try {
        const playing = this.voice.process(n);
        const l = this.voice.left();
        const r = this.voice.right();
        for (let i = 0; i < n; i++) left[i] = l[i];
        if (right) for (let i = 0; i < n; i++) right[i] = r[i];
        rendered = true;
        if (!playing) {
          this.alive = false;
          if (!this.endedPosted && this.voice.info().mode === 'timeline') {
            this.endedPosted = true;
            this.send({ t: 'ended' });
          }
        }
        this.frames += n;
        if (this.tracing && this.frames >= this.traceEvery) {
          this.frames -= this.traceEvery;
          this.send({
            t: 'trace',
            time: this.voice.time(),
            seconds: this.voice.seconds(),
            values: this.voice.trace().slice(),
          });
        }
      } catch (error) {
        this.trap(error);
      }
    }
    if (!rendered) {
      left.fill(0);
      if (right) right.fill(0);
    }
    return true;
  }

  private rebuild(): void {
    this.voice?.free();
    this.voice = null;
    this.engine = HaaEngine.fromModule(this.module);
    this.voice = this.engine.createVoice(sampleRate, this.seed);
    this.trapped = false;
    this.endedPosted = false;
    this.frames = 0;
  }

  private trap(error: unknown): void {
    this.alive = false;
    const first = !this.trapped;
    this.trapped = true;
    if (first) this.send({ t: 'error', message: error instanceof Error ? error.message : String(error) });
  }

  private send(msg: PlayerOutMsg): void {
    this.port.postMessage(msg);
  }

  private onMessage(msg: PlayerInMsg): void {
    try {
      switch (msg.t) {
        case 'set':
          if (this.alive) this.voice?.set(msg.name, msg.value);
          break;
        case 'trigger':
          if (this.alive) this.voice?.trigger(msg.name);
          break;
        case 'patch': {
          if (this.trapped || !this.voice) this.rebuild();
          const voice = this.voice!;
          const result = voice.load(msg.bytes, msg.keepState);
          this.playable = result.playable;
          this.alive = result.playable;
          this.endedPosted = false;
          this.send({ t: 'loaded', req: msg.req, playable: result.playable, issues: result.issues, info: voice.info() });
          break;
        }
        case 'restart':
          if (this.voice) {
            this.voice.restart();
            this.endedPosted = false;
            this.alive = !this.trapped && this.playable;
          }
          break;
        case 'trace':
          this.tracing = msg.on;
          break;
        case 'dispose':
          this.disposed = true;
          this.alive = false;
          this.voice?.free();
          this.voice = null;
          break;
      }
    } catch (error) {
      this.trap(error);
    }
  }
}

const REGISTERED = '__haaPlayerRegistered';
const scope = globalThis as unknown as Record<string, boolean>;
if (!scope[REGISTERED]) {
  scope[REGISTERED] = true;
  registerProcessor(PLAYER_PROCESSOR, HaaPlayerProcessor);
}
