// Offline scene: the shared conductor, sampler, beds and mix driven by a frame clock.
import { AUDIO_RATE, BLOCK, BUS_INDEX, DEFAULT_USER_VOLUMES, MIX_INPUTS, REVERB_INPUT } from '../contract.ts'
import type { MixPreset, OfflineAudio, SpatialModel, UserVolumes } from '../contract.ts'
import { renderFamilyVariants } from '../bank/render.ts'
import { BedHost } from '../host/bedHost.ts'
import { Sampler } from '../host/sampler.ts'
import { MixChain } from '../mix/mixChain.ts'
import { registerBuiltins } from '../registry/builtins.ts'
import { activeMix, getFamily, getMix } from '../registry/registry.ts'
import { Conductor } from '../runtime/conductor.ts'
import type { ConductorBackend } from '../runtime/conductor.ts'

const familyCache = new Map<string, Float32Array[]>()
type StereoPair = [Float32Array, Float32Array]
const blockPair = (): StereoPair => [new Float32Array(BLOCK), new Float32Array(BLOCK)]

/** Creates a deterministic offline graph; families render synchronously on first play. */
export function createOfflineAudio(o: {
  sampleRate?: number; mix?: string | MixPreset; maxListeners?: 1 | 2 | 3 | 4
  spatial?: SpatialModel; seed?: number; volumes?: Partial<UserVolumes>
} = {}): OfflineAudio {
  registerBuiltins()
  const preset = typeof o.mix === 'string' ? getMix(o.mix) : o.mix ?? activeMix()
  if (!preset) throw new Error(`[haa] unknown mix ${o.mix}`)
  const sr = o.sampleRate ?? AUDIO_RATE
  const mix = new MixChain(sr, preset)
  const sampler = new Sampler(sr)
  const inputs = Array.from({ length: MIX_INPUTS }, blockPair)
  const busScratch = blockPair()
  const sendScratch = blockPair()
  const output = blockPair()
  const beds = new Map<number, { host: BedHost; bus: number }>()
  const families = new Map<string, Float32Array[]>()
  let left = new Float32Array(BLOCK)
  let right = new Float32Array(BLOCK)
  let frames = 0

  const backend: ConductorBackend = {
    sampler(m) {
      switch (m.t) {
        case 'load': sampler.load(m.id, m.variants, m.rate); break
        case 'play': sampler.play(m); break
        case 'upd': sampler.update(m.voice, m.rate, m.sp); break
        case 'stop': sampler.stop(m.voice, m.fade); break
        case 'stopAll': sampler.stopAll(m.fade); break
      }
    },
    mix(m) {
      switch (m.t) {
        case 'user': mix.setUser(m.master, m.music, m.sfx); break
        case 'duck': mix.duck(m.kind, m.strength); break
        case 'reverb': mix.setReverb(m.p); break
        case 'muffle': mix.setMuffle(m.v); break
        case 'preset': mix.setPreset(m.p); break
      }
    },
    startBed(handle, def, seed, trimDb, values) {
      const host = new BedHost(sr, def, seed, trimDb)
      host.setParams(values)
      beds.set(handle, { host, bus: BUS_INDEX[def.bus] })
      return true
    },
    bed(handle, m) {
      const host = beds.get(handle)?.host
      if (!host) return
      switch (m.t) {
        case 'p': host.setParams(m.v); break
        case 'sp': host.setSpatial(m.rate, m.sp); break
        case 'end': host.end(m.fade); break
      }
    },
  }
  const conductor = new Conductor({
    preset, maxListeners: o.maxListeners ?? 1, seed: o.seed ?? 1,
    spatial: o.spatial ?? null, backend, now: () => frames / sr, loadRate: sr,
    loaded(id) {
      if (!sampler.has(id)) {
        const def = getFamily(id)
        if (!def) return 0
        const key = JSON.stringify([id, sr, preset.id])
        let variants = familyCache.get(key)
        if (!variants) {
          variants = renderFamilyVariants(def, sr, preset)
          familyCache.set(key, variants)
        }
        families.set(id, variants)
        backend.sampler({ t: 'load', id, variants, rate: sr })
      }
      return sampler.variantCount(id)
    },
    variantLength: (id, variant) => families.get(id)?.[variant]?.length ?? 0,
  })
  conductor.setVolumes({ ...DEFAULT_USER_VOLUMES, ...o.volumes })

  return {
    play: conductor.play.bind(conductor),
    startBed: conductor.startBed.bind(conductor),
    setListeners: conductor.setListeners.bind(conductor),
    setSpatial: conductor.setSpatial.bind(conductor),
    setOcclusion: conductor.setOcclusion.bind(conductor),
    duck: conductor.duck.bind(conductor),
    setVolumes: conductor.setVolumes.bind(conductor),
    setReverb: conductor.setReverb.bind(conductor),
    setMuffle: conductor.setMuffle.bind(conductor),
    music: conductor.music,
    get stats() { return conductor.stats },
    get time() { return frames / sr },
    advance(seconds) {
      const count = Number.isFinite(seconds) && seconds > 0 ? Math.round(seconds * sr) : 0
      const end = frames + count
      if (end > left.length) {
        let capacity = left.length
        while (capacity < end) capacity *= 2
        const l = new Float32Array(capacity)
        const r = new Float32Array(capacity)
        l.set(left); r.set(right)
        left = l; right = r
      }
      while (frames < end) {
        const n = Math.min(BLOCK, end - frames)
        sampler.process(inputs, n)
        for (const [handle, bed] of beds) {
          bed.host.process(busScratch, sendScratch, n)
          const bus = inputs[bed.bus]
          const send = inputs[REVERB_INPUT]
          for (let i = 0; i < n; i++) {
            bus[0][i] += busScratch[0][i]; bus[1][i] += busScratch[1][i]
            send[0][i] += sendScratch[0][i]; send[1][i] += sendScratch[1][i]
          }
          if (bed.host.done) beds.delete(handle)
        }
        mix.process(inputs, output[0], output[1], n)
        for (let i = 0; i < n; i++) {
          left[frames + i] = output[0][i]
          right[frames + i] = output[1][i]
        }
        frames += n
      }
    },
    take(name) {
      return { name, left: left.slice(0, frames), right: right.slice(0, frames), sampleRate: sr }
    },
  }
}
