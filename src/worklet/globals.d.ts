// AudioWorkletGlobalScope declarations for src/worklet/*. lib.dom (TypeScript 5.9) declares
// AudioWorkletNodeOptions but none of these worklet-realm globals, so this plain ambient file
// (no import or export) declares all four without a conflict.

/** Base class of every processor; its port talks to the AudioWorkletNode on the main thread. */
declare class AudioWorkletProcessor {
  readonly port: MessagePort
  constructor(options?: AudioWorkletNodeOptions)
}

/** Register a processor class under a name; throws when the name is already registered. */
declare function registerProcessor(
  name: string,
  processorCtor: new (options?: AudioWorkletNodeOptions) => AudioWorkletProcessor,
): void

/** Sample rate of the owning BaseAudioContext, in Hz. */
declare var sampleRate: number

/** Context time of the current render quantum, in seconds. */
declare var currentTime: number
