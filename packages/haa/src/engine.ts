// Realm-neutral wrapper over the HAA wasm voice ABI (ARCHITECTURE §13).
// Text crosses linear memory as UTF-8 bytes so this module also runs in an AudioWorklet.

import type { CatalogJson, HaaPatch, LoadResult, PatchIssue, VoiceInfo } from './types.ts';
import { BLOCK } from './types.ts';
import { decodeUtf8, encodeUtf8 } from './utf8.ts';

function readResult<T>(exports: HaaExports): T {
  const ptr = exports.haa_result_ptr();
  const len = exports.haa_result_len();
  const bytes = new Uint8Array(exports.memory.buffer, ptr, len);
  return JSON.parse(decodeUtf8(bytes)) as T;
}

/** The plain C exports supplied by `crates/haa-wasm`. */
export interface HaaExports {
  memory: WebAssembly.Memory;
  haa_version(): number;
  haa_alloc(len: number): number;
  haa_free(ptr: number, len: number): void;
  haa_result_ptr(): number;
  haa_result_len(): number;
  haa_catalog(): number;
  haa_voice_new(sr: number, seed: number): number;
  haa_voice_free(h: number): void;
  haa_voice_load(h: number, ptr: number, len: number, keep: number): number;
  haa_voice_info(h: number): number;
  haa_voice_input_index(h: number, ptr: number, len: number): number;
  haa_voice_set_input(h: number, i: number, v: number): void;
  haa_voice_trigger(h: number, i: number): void;
  haa_voice_restart(h: number): void;
  haa_voice_process(h: number, frames: number): number;
  haa_voice_left(h: number): number;
  haa_voice_right(h: number): number;
  haa_voice_trace(h: number): number;
  haa_voice_trace_len(h: number): number;
  haa_voice_time(h: number): number;
  haa_voice_seconds(h: number): number;
}

/** Instantiated wasm module and factory for realm-neutral voices. */
export class HaaEngine {
  /** Instantiate a compiled module with the ABI's empty import object. */
  static fromModule(module: WebAssembly.Module): HaaEngine {
    const instance = new WebAssembly.Instance(module, {});
    return new HaaEngine(instance.exports as unknown as HaaExports);
  }

  /** The wasm exports, exposed for consumers that need the raw ABI. */
  readonly exports: HaaExports;

  private constructor(exports: HaaExports) {
    this.exports = exports;
  }

  /** Read the node type catalog from wasm JSON memory. */
  catalog(): CatalogJson {
    this.exports.haa_catalog();
    return readResult<CatalogJson>(this.exports);
  }

  /** Create a voice at the requested sample rate and optional seed. */
  createVoice(sampleRate: number, seed = 0): HaaVoice {
    return new HaaVoice(this.exports, this.exports.haa_voice_new(sampleRate, seed));
  }

}

/** One voice instance and its views into wasm-owned audio and trace buffers. */
export class HaaVoice {
  /** The ABI handle; zero indicates that voice creation failed. */
  readonly handle: number;
  private readonly exports: HaaExports;
  private readonly inputIndices = new Map<string, number>();
  private cachedInfo: VoiceInfo = { nodes: [], inputs: [], mode: 'timeline', duration: 0, loop: false };
  private freed = false;
  private audioBuffer: ArrayBuffer | null = null;
  private traceBuffer: ArrayBuffer | null = null;
  private leftView: Float32Array | null = null;
  private rightView: Float32Array | null = null;
  private traceView: Float32Array | null = null;

  /** Constructed by `HaaEngine.createVoice`. */
  constructor(exports: HaaExports, handle: number) {
    this.exports = exports;
    this.handle = handle;
  }

  /** Load UTF-8 patch JSON and refresh info and all published input indices. */
  load(patch: Uint8Array, keepState = false): LoadResult {
    if (this.freed || this.handle === 0) return { playable: false, issues: [] };
    const ptr = this.copyIn(patch);
    let code: number;
    try {
      code = this.exports.haa_voice_load(this.handle, ptr, patch.length, keepState ? 1 : 0);
    } finally {
      this.exports.haa_free(ptr, patch.length);
    }
    const issues = readResult<PatchIssue[]>(this.exports);
    this.refreshInfoAndIndices();
    return { playable: code === 1, issues };
  }

  /** Read info cached after the most recent load. */
  info(): VoiceInfo {
    return this.cachedInfo;
  }

  /** Set a published input by name; enum values may be passed by their string name. */
  set(name: string, value: number | string): boolean {
    if (this.freed || this.handle === 0) return false;
    const index = this.inputIndices.get(name);
    if (index === undefined) return false;
    let numericValue: number;
    if (typeof value === 'string') {
      const input = this.cachedInfo.inputs.find((item) => item.name === name);
      const enumIndex = input?.kind === 'enum' ? input.values?.indexOf(value) ?? -1 : -1;
      if (enumIndex < 0) return false;
      numericValue = enumIndex;
    } else {
      numericValue = value;
    }
    this.exports.haa_voice_set_input(this.handle, index, numericValue);
    return true;
  }

  /** Raise a named trigger input; returns false if its name is unknown. */
  trigger(name: string): boolean {
    if (this.freed || this.handle === 0) return false;
    const index = this.inputIndices.get(name);
    if (index === undefined) return false;
    this.exports.haa_voice_trigger(this.handle, index);
    return true;
  }

  /** Restart the patch clock and node state. */
  restart(): void {
    if (!this.freed && this.handle !== 0) this.exports.haa_voice_restart(this.handle);
  }

  /** Render up to BLOCK frames; returns true while the voice is playing. */
  process(frames: number): boolean {
    if (this.freed || this.handle === 0) return false;
    return this.exports.haa_voice_process(this.handle, frames) !== 0;
  }

  /** Left output buffer as a BLOCK-length view into wasm memory. */
  left(): Float32Array {
    this.refreshAudioViews();
    return this.leftView ?? new Float32Array(0);
  }

  /** Right output buffer as a BLOCK-length view into wasm memory. */
  right(): Float32Array {
    this.refreshAudioViews();
    return this.rightView ?? new Float32Array(0);
  }

  /** Last control value for each plan node, viewed directly in wasm memory. */
  trace(): Float32Array {
    if (this.freed || this.handle === 0) return new Float32Array(0);
    const buffer = this.exports.memory.buffer;
    const length = this.exports.haa_voice_trace_len(this.handle);
    if (this.traceBuffer !== buffer || this.traceView === null || this.traceView.length !== length) {
      this.traceBuffer = buffer;
      this.traceView = new Float32Array(buffer, this.exports.haa_voice_trace(this.handle), length);
    }
    return this.traceView;
  }

  /** Normalized patch time of the next sample. */
  time(): number {
    return this.freed || this.handle === 0 ? 0 : this.exports.haa_voice_time(this.handle);
  }

  /** Patch clock in seconds. */
  seconds(): number {
    return this.freed || this.handle === 0 ? 0 : this.exports.haa_voice_seconds(this.handle);
  }

  /** Free this voice; subsequent calls are safe no-ops. */
  free(): void {
    if (this.freed) return;
    if (this.handle !== 0) this.exports.haa_voice_free(this.handle);
    this.freed = true;
    this.inputIndices.clear();
    this.leftView = null;
    this.rightView = null;
    this.traceView = null;
    this.audioBuffer = null;
    this.traceBuffer = null;
  }

  /** Allocate and copy bytes into wasm memory. */
  private copyIn(bytes: Uint8Array): number {
    const ptr = this.exports.haa_alloc(bytes.length);
    new Uint8Array(this.exports.memory.buffer, ptr, bytes.length).set(bytes);
    return ptr;
  }

  /** Refresh cached info and resolve header and reserved input indices. */
  private refreshInfoAndIndices(): void {
    this.exports.haa_voice_info(this.handle);
    this.cachedInfo = readResult<VoiceInfo>(this.exports);
    this.inputIndices.clear();
    for (const input of this.cachedInfo.inputs) {
      const index = this.lookupInputIndex(input.name);
      if (index >= 0) this.inputIndices.set(input.name, index);
    }
    for (const name of ['_dist', '_pan', '_doppler']) {
      const index = this.lookupInputIndex(name);
      if (index >= 0) this.inputIndices.set(name, index);
    }
  }

  /** Resolve one ABI input index using a temporary UTF-8 allocation. */
  private lookupInputIndex(name: string): number {
    const bytes = encodeUtf8(name);
    const ptr = this.copyIn(bytes);
    try {
      return this.exports.haa_voice_input_index(this.handle, ptr, bytes.length);
    } finally {
      this.exports.haa_free(ptr, bytes.length);
    }
  }

  /** Recreate audio views only when wasm memory has grown or changed. */
  private refreshAudioViews(): void {
    if (this.freed || this.handle === 0) {
      this.leftView = null;
      this.rightView = null;
      return;
    }
    const buffer = this.exports.memory.buffer;
    if (this.audioBuffer !== buffer || this.leftView === null || this.rightView === null) {
      this.audioBuffer = buffer;
      this.leftView = new Float32Array(buffer, this.exports.haa_voice_left(this.handle), BLOCK);
      this.rightView = new Float32Array(buffer, this.exports.haa_voice_right(this.handle), BLOCK);
    }
  }
}

/** Convert patch data to UTF-8 JSON bytes; byte arrays are returned unchanged. */
export function patchBytes(patch: HaaPatch | string | Uint8Array): Uint8Array {
  if (patch instanceof Uint8Array) return patch;
  return encodeUtf8(typeof patch === 'string' ? patch : JSON.stringify(patch));
}
