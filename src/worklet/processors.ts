// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: no game-specific voice, HAA hosts and messages, guarded registration
//
// AudioWorkletGlobalScope processors: one thin AudioWorkletProcessor subclass per DSP host
// ('haa-mix', 'haa-sampler', 'haa-bed'). The globals come from ./globals.d.ts; a harmless base
// class stands in when the module is imported outside a worklet.
//
// Every process() handles up to BLOCK frames, copies from and to preallocated host buffers,
// never allocates and catches every exception by outputting silence and resetting its host.
// The bed returns false once its host is done after 'end'; the sampler and mix return true.
import { BLOCK, MIX_INPUTS, PROCESSOR_NAMES } from '../contract.ts'
import type { BedMsg, MixMsg, MixPreset, SamplerMsg } from '../contract.ts'
import { BedHost } from '../host/bedHost.ts'
import { Sampler } from '../host/sampler.ts'
import { MixChain } from '../mix/mixChain.ts'
import { activeMix, getBed } from '../registry/registry.ts'

type ProcessorCtor = typeof AudioWorkletProcessor

/** AudioWorkletProcessor from the global scope, with a harmless base for non-worklet imports. */
const WorkletBase: ProcessorCtor =
  (globalThis as { AudioWorkletProcessor?: ProcessorCtor }).AudioWorkletProcessor ??
  (class {
    readonly port = { onmessage: null } as unknown as MessagePort
  } as unknown as ProcessorCtor)

/** The context sample rate from the global scope; 48000 only as a last resort. */
function workletRate(): number {
  const v = (globalThis as { sampleRate?: number }).sampleRate
  return typeof v === 'number' && v > 0 ? v : 48000
}

/** processorOptions read defensively: every field is optional. */
interface HaaProcessorOptions {
  preset?: MixPreset
  bed?: string
  seed?: number
  trimDb?: number
  values?: ArrayLike<number>
}
function readOptions(options?: AudioWorkletNodeOptions): HaaProcessorOptions {
  const po = options?.processorOptions as HaaProcessorOptions | undefined
  return po && typeof po === 'object' ? po : {}
}

// --- Preallocated block helpers (no allocation in process()) ------------------------------

function blockLen(outputs: Float32Array[][]): number {
  const ch = outputs[0]?.[0]
  return ch ? Math.min(ch.length, BLOCK) : 0
}

function zeroOutputs(outputs: Float32Array[][]): void {
  for (let o = 0; o < outputs.length; o++) {
    const ch = outputs[o]
    if (!ch) continue
    for (let c = 0; c < ch.length; c++) ch[c].fill(0)
  }
}

function copyChannel(dst: Float32Array | undefined, src: Float32Array, n: number): void {
  if (!dst) return
  const len = Math.min(n, dst.length, src.length)
  for (let i = 0; i < len; i++) dst[i] = src[i]
  for (let i = len; i < dst.length; i++) dst[i] = 0
}

/** Copy a host pair to an output; a 1-channel output gets the left channel only. */
function copyPair(out: Float32Array[] | undefined, src: readonly [Float32Array, Float32Array], n: number): void {
  if (!out) return
  copyChannel(out[0], src[0], n)
  if (out.length > 1) copyChannel(out[1], src[1], n)
}

function makePairs(count: number): [Float32Array, Float32Array][] {
  const out: [Float32Array, Float32Array][] = []
  for (let i = 0; i < count; i++) out.push([new Float32Array(BLOCK), new Float32Array(BLOCK)])
  return out
}

// --- 'haa-mix': one per context, 7 stereo inputs and 1 stereo output ----------------------

class MixProcessor extends WorkletBase {
  private readonly chain: MixChain
  private readonly inputs = makePairs(MIX_INPUTS)
  private readonly silence = makePairs(1)[0]
  private readonly bound: (readonly [Float32Array, Float32Array])[] = []
  private readonly out = makePairs(1)[0]

  constructor(options?: AudioWorkletNodeOptions) {
    super(options)
    const o = readOptions(options)
    this.chain = new MixChain(workletRate(), o.preset ?? activeMix())
    for (let i = 0; i < MIX_INPUTS; i++) this.bound.push(this.silence)
    this.port.onmessage = (e) => {
      try {
        this.onMsg(e.data as MixMsg)
      } catch {
        this.chain.reset()
      }
    }
  }

  private onMsg(m: MixMsg): void {
    if (!m) return
    switch (m.t) {
      case 'preset': this.chain.setPreset(m.p); break
      case 'user': this.chain.setUser(m.master, m.music, m.sfx); break
      case 'duck': this.chain.duck(m.kind, m.strength); break
      case 'reverb': this.chain.setReverb(m.p); break
      case 'muffle': this.chain.setMuffle(m.v); break
    }
  }

  /** Point input i at its host pair; a missing or empty input is the shared silent pair. */
  private bindInput(src: Float32Array[] | undefined, i: number, n: number): void {
    const c0 = src?.[0]
    if (!c0 || c0.length < n) {
      this.bound[i] = this.silence
      return
    }
    const c1 = src[1] && src[1].length >= n ? src[1] : c0
    const pair = this.inputs[i]
    for (let j = 0; j < n; j++) {
      pair[0][j] = c0[j]
      pair[1][j] = c1[j]
    }
    this.bound[i] = pair
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const n = blockLen(outputs)
    try {
      if (n > 0) {
        for (let i = 0; i < MIX_INPUTS; i++) this.bindInput(inputs[i], i, n)
        this.chain.process(this.bound, this.out[0], this.out[1], n)
        copyPair(outputs[0], this.out, n)
      }
    } catch {
      zeroOutputs(outputs)
      this.chain.reset()
    }
    return true
  }
}

// --- 'haa-sampler': one per context, 0 inputs and 7 stereo outputs ------------------------

class SamplerProcessor extends WorkletBase {
  private readonly sampler: Sampler
  private readonly host = makePairs(MIX_INPUTS)

  constructor(options?: AudioWorkletNodeOptions) {
    super(options)
    this.sampler = new Sampler(workletRate())
    this.port.onmessage = (e) => {
      try {
        this.onMsg(e.data as SamplerMsg)
      } catch {
        this.sampler.stopAll(0)
      }
    }
  }

  private onMsg(m: SamplerMsg): void {
    if (!m) return
    switch (m.t) {
      case 'load': this.sampler.load(m.id, m.variants, m.rate); break
      case 'play':
        this.sampler.play({
          voice: m.voice, id: m.id, variant: m.variant, bus: m.bus, gain: m.gain, rate: m.rate, sp: m.sp,
        })
        break
      case 'upd': this.sampler.update(m.voice, m.rate, m.sp); break
      case 'stop': this.sampler.stop(m.voice, m.fade); break
      case 'stopAll': this.sampler.stopAll(m.fade); break
    }
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const n = blockLen(outputs)
    try {
      if (n > 0) {
        this.sampler.process(this.host, n)
        for (let i = 0; i < MIX_INPUTS && i < outputs.length; i++) copyPair(outputs[i], this.host[i], n)
      }
    } catch {
      zeroOutputs(outputs)
      this.sampler.stopAll(0)
    }
    return true
  }
}

// --- 'haa-bed': one per started bed, 0 inputs, output 0 = bus, output 1 = send ------------

class BedProcessor extends WorkletBase {
  private readonly host: BedHost | null
  private readonly bus = makePairs(1)[0]
  private readonly send = makePairs(1)[0]
  private ending = false

  constructor(options?: AudioWorkletNodeOptions) {
    super(options)
    const o = readOptions(options)
    const def = typeof o.bed === 'string' ? getBed(o.bed) : undefined
    let host: BedHost | null = null
    if (def) {
      try {
        host = new BedHost(workletRate(), def, o.seed ?? 0, o.trimDb ?? 0)
        if (o.values) host.setParams(o.values)
      } catch {
        host = null
      }
    }
    this.host = host
    this.port.onmessage = (e) => {
      try {
        this.onMsg(e.data as BedMsg)
      } catch {
        this.host?.reset()
      }
    }
  }

  private onMsg(m: BedMsg): void {
    const h = this.host
    if (!m || !h) return
    switch (m.t) {
      case 'p': h.setParams(m.v); break
      case 'sp': h.setSpatial(m.rate, m.sp); break
      case 'end':
        this.ending = true
        h.end(m.fade)
        break
    }
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const h = this.host
    if (!h) {
      zeroOutputs(outputs)
      return false
    }
    const n = blockLen(outputs)
    try {
      if (n > 0) {
        h.process(this.bus, this.send, n)
        copyPair(outputs[0], this.bus, n)
        copyPair(outputs[1], this.send, n)
      }
    } catch {
      zeroOutputs(outputs)
      h.reset()
      if (this.ending) h.end(0)
    }
    return !(this.ending && h.done)
  }
}

// --- Registration, guarded against a second call -------------------------------------------

const registered = new Set<string>()

/** Register 'haa-mix', 'haa-sampler' and 'haa-bed'; names already registered are skipped. */
export function registerHaaProcessors(): void {
  const register = (globalThis as { registerProcessor?: typeof registerProcessor }).registerProcessor
  if (typeof register !== 'function') return
  const entries: [string, ProcessorCtor][] = [
    [PROCESSOR_NAMES.mix, MixProcessor],
    [PROCESSOR_NAMES.sampler, SamplerProcessor],
    [PROCESSOR_NAMES.bed, BedProcessor],
  ]
  for (const [name, ctor] of entries) {
    if (registered.has(name)) continue
    registered.add(name)
    try {
      register.call(globalThis, name, ctor)
    } catch (err) {
      console.warn('[haa] registerProcessor ' + name, err)
    }
  }
}
