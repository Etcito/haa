// The acoustic tracer behind the engine (docs/physics-sound.md, "Acoustic model" and "Compute"):
// what the engine asks of the geometry once per listener trace and once per voice. Three
// implementations share this interface:
// - `rayCastTracer(cast)`: the TS functions of raytrace.ts / acoustics.ts over any RayCast (Rapier
//   from JS, one cast at a time): the reference, and the engine's path when no tracer is set;
// - `createWasmTracer` (trace-wasm.ts): the same traces in Rust/wasm (crates/haa-trace) over its own
//   BVH of the level, one call per trace instead of one per ray, on the main thread;
// - `createWorkerTracer` (trace-worker-client.ts): that wasm in a worker, results a frame or two late
//   (every value is glided anyway), nothing on the main thread but the messages.
import type { RayCast, RaySegment, Reflection, Vec3 } from './acoustics.ts';
import { earlyReflections, sub, v3 } from './acoustics.ts';
import type { DirectPath, ListenerTrace, VoiceReverb } from './raytrace.ts';
import { directPath, sphereDirections, traceListener, voiceReverb } from './raytrace.ts';

export interface TracerListenerOptions {
  rays: number;
  bounces: number;
  /** Every n-th traced ray feeds a voice's diffuse field (the feet use twice that). */
  reverbStride: number;
  /** Reference distance of the direct sound (m). */
  ref: number;
}

export interface TracerVoiceOptions {
  extent: number;
  reverbStride: number;
  ref: number;
  /** Early-reflection taps kept. */
  taps: number;
}

export interface ListenerResult {
  /** `rays` and `candidates` may be empty when the trace lives in wasm: only `space` is read. */
  trace: ListenerTrace;
  /** Diffuse field of a source at the listener's feet (the room's own level). */
  feet: VoiceReverb;
}

export interface VoiceGeometry {
  path: DirectPath;
  reverb: VoiceReverb | null;
  reflections: Reflection[];
}

export interface AcousticTracer {
  /**
   * True when answers arrive later (a worker): the engine then also asks every frame with
   * `request` false, to pick up an answer as soon as it lands.
   */
  readonly async?: boolean;
  /** Trace from the listener. Null: nothing new (an async tracer still working); keep the last one. */
  listener(position: Vec3, o: TracerListenerOptions, segments?: RaySegment[], request?: boolean): ListenerResult | null;
  /**
   * One voice's geometry (`key` identifies the voice). `request` false only collects an answer
   * that arrived since (async). Null: nothing new; keep the last values.
   */
  voice(key: number, source: Vec3, listener: Vec3, o: TracerVoiceOptions, segments?: RaySegment[], request?: boolean): VoiceGeometry | null;
  /** The voice `key` ended: drop its pending answer. */
  forget?(key: number): void;
  /** Called once at the end of each engine update (an async tracer sends its batch then). */
  flush?(): void;
  dispose?(): void;
}

/** Directions of the image-source rays from each source (64, Fibonacci). */
export const SOURCE_DIRS: readonly Vec3[] = sphereDirections(64);
/** Options of the voices' early reflections (the engine's taps). */
export const REFLECTION_OPTIONS = { soft: 0.35, minDelay: 0.0015, scattering: 0.35 } as const;

/** The reference tracer: the TS traces over `cast`, synchronously. */
export function rayCastTracer(cast: RayCast): AcousticTracer {
  let trace: ListenerTrace | null = null;
  return {
    listener(position, o, segments, request = true) {
      if (!request) return null;
      trace = traceListener(position, cast, { rays: o.rays, bounces: o.bounces }, segments);
      const feet = voiceReverb(trace, sub(position, v3(0, 1.5, 0)), cast, { stride: o.reverbStride * 2, ref: o.ref });
      return { trace, feet };
    },
    voice(_key, source, listener, o, segments, request = true) {
      if (!request) return null;
      return {
        path: directPath(source, listener, trace, cast, { extent: o.extent }, segments),
        reverb: trace ? voiceReverb(trace, source, cast, { stride: o.reverbStride, ref: o.ref }) : null,
        reflections: earlyReflections(source, listener, cast, { max: o.taps, directions: SOURCE_DIRS, ...REFLECTION_OPTIONS }, segments),
      };
    },
  };
}
