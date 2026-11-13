// Browser runtime: wires the HAA AudioWorklet nodes to a Conductor and a BankLoader.
// Browser-only (Web Audio API, timers); the control brain itself stays in conductor.ts.
import { BUS_INDEX, DEFAULT_USER_VOLUMES, MIX_INPUTS, PROCESSOR_NAMES, REVERB_INPUT } from '../contract.ts'
import type { AudioOptions, BedDef, BedMsg, HaaAudio, MixMsg, MixPreset, SamplerMsg } from '../contract.ts'
import { registerBuiltins } from '../registry/builtins.ts'
import { activeMix, getMix } from '../registry/registry.ts'
import { BankLoader } from './bankLoader.ts'
import { Conductor } from './conductor.ts'
import type { ConductorBackend } from './conductor.ts'

const ANALYSER_FFT = 2048
const DISCONNECT_MS = 100

function unavailable(reason: unknown): Error {
  return new Error(`[haa] worklet unavailable: ${reason instanceof Error ? reason.message : String(reason)}`)
}

function resolveMix(mix: string | MixPreset | undefined): MixPreset {
  if (mix === undefined) return activeMix()
  if (typeof mix !== 'string') return mix
  const preset = getMix(mix)
  if (!preset) throw new Error(`[haa] unknown mix ${mix}`)
  return preset
}

function node(ctx: BaseAudioContext, name: string, options: AudioWorkletNodeOptions): AudioWorkletNode {
  try {
    return new AudioWorkletNode(ctx, name, options)
  } catch (error) {
    throw unavailable(error)
  }
}

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/**
 * Creates the browser audio engine on a context: mix, sampler and per-bed worklet nodes driven by a
 * Conductor. Rejects '[haa] worklet unavailable: <reason>' only when the worklet cannot be set up.
 */
export async function createAudio(ctx: BaseAudioContext, o: AudioOptions): Promise<HaaAudio> {
  registerBuiltins()
  const preset = resolveMix(o.mix)
  if (!ctx.audioWorklet) throw new Error('[haa] worklet unavailable: no audioWorklet')
  try {
    await ctx.audioWorklet.addModule(o.processorUrl)
  } catch (error) {
    throw unavailable(error)
  }

  const mix = node(ctx, PROCESSOR_NAMES.mix, {
    numberOfInputs: MIX_INPUTS, numberOfOutputs: 1, outputChannelCount: [2],
    channelCount: 2, channelCountMode: 'explicit', processorOptions: { preset },
  })
  const nodes: AudioNode[] = [mix]
  let meters: HaaAudio['meters'] = null
  if (typeof AudioContext !== 'undefined' && ctx instanceof AudioContext) {
    const analyser = () => {
      const a = ctx.createAnalyser()
      a.fftSize = ANALYSER_FFT
      nodes.push(a)
      return a
    }
    const splitter = ctx.createChannelSplitter(2)
    nodes.push(splitter)
    meters = { mix: analyser(), left: analyser(), right: analyser() }
    mix.connect(splitter)
    splitter.connect(meters.left, 0)
    splitter.connect(meters.right, 1)
    mix.connect(meters.mix)
  }
  mix.connect(ctx.destination)

  const sampler = node(ctx, PROCESSOR_NAMES.sampler, {
    numberOfInputs: 0, numberOfOutputs: MIX_INPUTS,
    outputChannelCount: Array.from({ length: MIX_INPUTS }, () => 2),
  })
  nodes.push(sampler)
  for (let i = 0; i < MIX_INPUTS; i++) sampler.connect(mix, i, i)

  const postMix = (m: MixMsg) => mix.port.postMessage(m)
  const postSampler = (m: SamplerMsg) => {
    if (m.t === 'load') sampler.port.postMessage(m, m.variants.map((v) => v.buffer as ArrayBuffer))
    else sampler.port.postMessage(m)
  }
  postMix({ t: 'preset', p: preset })
  postMix({ t: 'user', ...DEFAULT_USER_VOLUMES })

  const beds = new Map<number, AudioWorkletNode>()
  let bedWarned = false
  const dropBed = (handle: number) => {
    const bed = beds.get(handle)
    if (!bed) return
    beds.delete(handle)
    bed.disconnect()
  }
  const backend: ConductorBackend = {
    sampler: postSampler,
    mix: postMix,
    startBed(handle: number, def: BedDef, seed: number, trimDb: number, values: Float32Array): boolean {
      let bed: AudioWorkletNode
      try {
        bed = new AudioWorkletNode(ctx, PROCESSOR_NAMES.bed, {
          numberOfInputs: 0, numberOfOutputs: 2, outputChannelCount: [2, 2],
          processorOptions: { bed: def.id, seed, trimDb, values: Array.from(values) },
        })
      } catch (error) {
        if (!bedWarned) console.warn(`[haa] bed node failed ${def.id}: ${String(error)}`)
        bedWarned = true
        return false
      }
      bed.connect(mix, 0, BUS_INDEX[def.bus])
      bed.connect(mix, 1, REVERB_INPUT)
      beds.set(handle, bed)
      return true
    },
    bed(handle: number, m: BedMsg): void {
      const bed = beds.get(handle)
      if (!bed) return
      bed.port.postMessage(m)
      if (m.t === 'end') setTimeout(() => dropBed(handle), m.fade * 1000 + DISCONNECT_MS)
    },
  }

  const loader = new BankLoader(ctx, o.manifestUrl, preset, (id, variants, rate) => {
    postSampler({ t: 'load', id, variants, rate })
  })
  const conductor = new Conductor({
    preset,
    maxListeners: o.maxListeners ?? 1,
    seed: o.seed ?? 1,
    spatial: o.spatial ?? null,
    backend,
    now: () => ctx.currentTime,
    loaded: (id) => loader.loaded(id),
    variantLength: (id, v) => loader.variantLength(id, v),
    loadRate: ctx.sampleRate,
  })
  const ready = loader.ensure(o.families).catch((error: unknown) => {
    console.warn(`[haa] bank load failed: ${String(error)}`)
  })

  let closing: Promise<void> | null = null
  const close = async (): Promise<void> => {
    postSampler({ t: 'stopAll', fade: 0.05 })
    for (const bed of beds.values()) bed.port.postMessage({ t: 'end', fade: 0.05 } satisfies BedMsg)
    await wait(DISCONNECT_MS)
    for (const bed of beds.values()) bed.disconnect()
    beds.clear()
    for (const n of nodes) n.disconnect()
  }

  return {
    ctx,
    meters,
    ready: () => ready,
    close: () => (closing ??= close().catch(() => {})),
    play: (id, opts) => conductor.play(id, opts),
    startBed: (id, opts) => conductor.startBed(id, opts),
    setListeners: (ls) => conductor.setListeners(ls),
    setSpatial: (m) => conductor.setSpatial(m),
    setOcclusion: (fn) => conductor.setOcclusion(fn),
    duck: (kind, strength) => conductor.duck(kind, strength),
    setVolumes: (v) => conductor.setVolumes(v),
    setReverb: (p) => conductor.setReverb(p),
    setMuffle: (v) => conductor.setMuffle(v),
    get music() { return conductor.music },
  }
}
