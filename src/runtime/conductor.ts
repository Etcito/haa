// Realm-independent audio controls, written for HAA (not ported from the origin engine's director).
// Time and transport are injected; spatial work reuses state owned by each source.
import { BUS_INDEX, DEFAULT_USER_VOLUMES, MAX_LISTENERS, SAMPLER_VOICES, SP, VOICE_LIMITS, newSpatialResult } from '../contract.ts'
import type { AudioControls, BedDef, BedHandle, BedMsg, Listener, MixMsg, MixPreset, MusicControl, OcclusionFn, PlayOptions, ReverbParams, SamplerMsg, Shot, SourcePos, SpatialModel, SpatialResult, UserVolumes } from '../contract.ts'
import { clamp, dbToGain } from '../dsp/math.ts'
import { hashSeed } from '../dsp/rng.ts'
import { MUSIC_PARAMS } from '../music/musicSynth.ts'
import { bedTrimDb, familyTrimDb, getBed, getFamily, styleIndex } from '../registry/registry.ts'
import { FLAT_SPATIAL } from './flatSpatial.ts'
import { VariationPicker, VoiceAllocator } from './voices.ts'

/** Transport supplied by the browser or offline host. */
export interface ConductorBackend {
  sampler(m: SamplerMsg): void
  mix(m: MixMsg): void
  startBed(handle: number, def: BedDef, seed: number, trimDb: number, values: Float32Array): boolean
  bed(handle: number, m: BedMsg): void
}

type Options = {
  preset: MixPreset; maxListeners: number; seed: number; spatial: SpatialModel | null
  backend: ConductorBackend; now: () => number; loaded: (id: string) => number
  variantLength: (id: string, v: number) => number; loadRate: number
}
type BedOptions = NonNullable<Parameters<AudioControls['startBed']>[1]>
type SpatialState = { src: SourcePos; positioned: boolean; spatial: SpatialResult }
type ShotState = SpatialState & { live: boolean; generation: number; end: number; baseRate: number; listener?: number }

function spatialState(): SpatialState {
  return { src: { pos: { x: 0, y: 0, z: 0 } }, positioned: false, spatial: newSpatialResult() }
}

/** Shared control brain: seeded variation, voice admission, spatial updates and music. */
export class Conductor implements AudioControls {
  #o: Options
  #allocator = new VoiceAllocator(VOICE_LIMITS)
  #picker: VariationPicker
  #listeners: readonly Listener[] = []
  #spatial: SpatialModel | null
  #occlusion: OcclusionFn | null = null
  #scratch = spatialState()
  #jitter = { rate: 1, gainDb: 0 }
  #shots: ShotState[] = Array.from({ length: SAMPLER_VOICES }, () => ({
    ...spatialState(), live: false, generation: 0, end: 0, baseRate: 1,
  }))
  #beds = new Map<number, { state: SpatialState; positional: boolean }>()
  #nextBed = 0
  #warned = new Set<string>()
  #volumes = { ...DEFAULT_USER_VOLUMES }
  #played = 0
  #unloaded = 0
  #musicHandle: BedHandle | null = null
  #musicValues = new Float32Array(MUSIC_PARAMS.length)
  #style: string | null = null
  /** A single managed music bed. */
  readonly music: MusicControl

  /** Uses the supplied clock and transport in any realm. */
  constructor(o: Options) {
    this.#o = o
    this.#spatial = o.spatial
    this.#picker = new VariationPicker(hashSeed(o.seed, 'shots'))
    const self = this
    this.music = {
      get style() { return self.#style },
      play: (style, options = {}) => this.#playMusic(style, options),
      crossfade: (style, bars) => {
        const index = styleIndex(style)
        if (!this.#musicHandle || index < 0) return
        this.#musicValues[MUSIC_PARAMS.indexOf('style')] = index
        this.#musicValues[MUSIC_PARAMS.indexOf('fadeBars')] = clamp(bars, 1, 16)
        this.#style = style
        this.#musicHandle.set(this.#musicValues)
      },
      set: (name, value) => {
        if (!this.#musicHandle) return
        this.#musicValues[MUSIC_PARAMS.indexOf(name)] = value
        this.#musicHandle.set(this.#musicValues)
      },
      stop: (fade) => {
        this.#musicHandle?.stop(fade)
        this.#musicHandle = null
        this.#style = null
      },
    }
  }

  /** Successful plays and admission failures, including unloaded families. */
  get stats(): { played: number; dropped: number; stolen: number } {
    return { played: this.#played, dropped: this.#unloaded + this.#allocator.dropped, stolen: this.#allocator.stolenCount }
  }

  /** Selects a variant, applies variation and spatial gain, then admits a voice. */
  play(id: string, o: PlayOptions = {}): Shot | null {
    const def = getFamily(id)
    if (!def) { this.#warn(id); return null }
    const count = this.#o.loaded(id)
    if (count === 0) { this.#unloaded++; return null }
    const now = this.#o.now()
    this.#expire(now)
    const variant = o.variant ?? this.#picker.pick(id, count)
    this.#picker.jitter(this.#jitter)
    const baseRate = 2 ** ((o.pitch ?? 0) / 12) * this.#jitter.rate
    const gain = (o.gain ?? 1) * dbToGain(familyTrimDb(this.#o.preset, id) + this.#jitter.gainDb)
    const s = this.#scratch
    s.positioned = o.pos !== undefined
    s.src.pos = o.pos ?? s.src.pos
    s.src.vel = o.vel; s.src.range = o.range; s.src.occlusion = o.occlusion
    this.#evaluate(s, o.listener)
    const rate = baseRate * s.spatial.rate
    let maxGain = 0
    for (let l = 0; l < MAX_LISTENERS; l++) maxGain = Math.max(maxGain, s.spatial.sp[SP.GAIN + l])
    const duration = this.#o.variantLength(id, variant) / (this.#o.loadRate * rate)
    const voice = this.#allocator.request(def.category, gain * maxGain, now, duration)
    if (!voice) return null
    if (voice.stolen >= 0) this.#o.backend.sampler({ t: 'stop', voice: voice.stolen, fade: 0.03 })
    const live = this.#shots[voice.id]
    live.generation++
    live.live = true; live.end = now + duration; live.baseRate = baseRate; live.listener = o.listener
    live.positioned = s.positioned
    live.src.pos = s.src.pos; live.src.vel = s.src.vel; live.src.range = s.src.range; live.src.occlusion = s.src.occlusion
    live.spatial.sp.set(s.spatial.sp); live.spatial.rate = s.spatial.rate
    this.#o.backend.sampler({ t: 'play', voice: voice.id, id, variant, bus: BUS_INDEX[o.bus ?? def.bus], gain, rate, sp: live.spatial.sp.slice() })
    this.#played++
    const generation = live.generation
    return {
      voice: voice.id,
      setPos: (pos, vel) => {
        this.#expire(this.#o.now())
        if (!live.live || live.generation !== generation) return
        live.positioned = true; live.src.pos = pos; live.src.vel = vel
        this.#updateShot(voice.id, live)
      },
      stop: (fade = 0.03) => {
        this.#expire(this.#o.now())
        if (!live.live || live.generation !== generation) return
        live.live = false
        this.#o.backend.sampler({ t: 'stop', voice: voice.id, fade })
      },
    }
  }

  /** Starts an independently seeded bed with defaults overridden by supplied values. */
  startBed(id: string, o: BedOptions = {}): BedHandle | null {
    const def = getBed(id)
    if (!def) { this.#warn(id); return null }
    const handle = this.#nextBed++
    const values = new Float32Array(def.defaults)
    if (o.values) for (let i = 0; i < Math.min(values.length, o.values.length); i++) values[i] = o.values[i]
    const seed = o.seed ?? hashSeed(this.#o.seed, 'bed', id, handle)
    if (!this.#o.backend.startBed(handle, def, seed, bedTrimDb(this.#o.preset, id), values.slice())) return null
    const state = spatialState()
    state.positioned = o.pos !== undefined; state.src.pos = o.pos ?? state.src.pos; state.src.vel = o.vel
    this.#beds.set(handle, { state, positional: def.positional })
    if (def.positional) this.#updateBed(handle, state)
    const post = () => this.#o.backend.bed(handle, { t: 'p', v: values.slice() })
    return {
      id,
      set: (v) => {
        if (!this.#beds.has(handle)) return
        for (let i = 0; i < Math.min(values.length, v.length); i++) values[i] = v[i]
        post()
      },
      setParam: (name, value) => {
        const index = def.params.indexOf(name)
        if (!this.#beds.has(handle) || index < 0) return
        values[index] = value; post()
      },
      setPos: (pos, vel) => {
        if (!this.#beds.has(handle) || !def.positional) return
        state.positioned = true; state.src.pos = pos; state.src.vel = vel
        this.#updateBed(handle, state)
      },
      stop: (fade = 0.03) => {
        if (!this.#beds.delete(handle)) return
        this.#o.backend.bed(handle, { t: 'end', fade })
      },
    }
  }

  /** Caps listeners and refreshes live positional sources after expiry. */
  setListeners(ls: readonly Listener[]): void {
    this.#listeners = ls.slice(0, clamp(Math.floor(this.#o.maxListeners), 1, MAX_LISTENERS))
    this.#refresh(false)
  }
  /** Replaces the spatial model and immediately re-evaluates sources. */
  setSpatial(m: SpatialModel | null): void { this.#spatial = m; this.#refresh(true) }
  /** Replaces the occlusion callback and immediately re-evaluates sources. */
  setOcclusion(fn: OcclusionFn | null): void { this.#occlusion = fn; this.#refresh(true) }
  /** Triggers a named preset duck; unknown kinds are ignored. */
  duck(kind: string, strength = 1): void {
    if (Object.hasOwn(this.#o.preset.ducks, kind)) this.#o.backend.mix({ t: 'duck', kind, strength })
  }
  /** Merges and clamps user volume multipliers. */
  setVolumes(v: Partial<UserVolumes>): void {
    for (const key of ['master', 'music', 'sfx'] as const) {
      if (v[key] !== undefined) this.#volumes[key] = clamp(v[key], 0, 1)
    }
    this.#o.backend.mix({ t: 'user', ...this.#volumes })
  }
  /** Changes the mix reverb. */
  setReverb(p: ReverbParams): void { this.#o.backend.mix({ t: 'reverb', p }) }
  /** Changes the mix muffle amount. */
  setMuffle(v: number): void { this.#o.backend.mix({ t: 'muffle', v }) }

  #warn(id: string): void {
    if (this.#warned.has(id)) return
    this.#warned.add(id)
    console.warn(`[haa] unknown family ${id}`)
  }
  #expire(now: number): void {
    this.#allocator.expire(now)
    for (const s of this.#shots) if (s.live && s.end <= now) s.live = false
  }
  #evaluate(s: SpatialState, listener?: number): void {
    (this.#spatial ?? FLAT_SPATIAL).evaluate(this.#listeners, s.positioned ? s.src : null, this.#occlusion, s.spatial)
    if (listener !== undefined) {
      for (let l = 0; l < MAX_LISTENERS; l++) if (l !== listener) s.spatial.sp[SP.GAIN + l] = 0
    }
  }
  #updateShot(voice: number, s: ShotState): void {
    this.#evaluate(s, s.listener)
    this.#o.backend.sampler({ t: 'upd', voice, rate: s.baseRate * s.spatial.rate, sp: s.spatial.sp.slice() })
  }
  #updateBed(handle: number, s: SpatialState): void {
    this.#evaluate(s)
    this.#o.backend.bed(handle, { t: 'sp', rate: s.spatial.rate, sp: s.spatial.sp.slice() })
  }
  #refresh(all: boolean): void {
    this.#expire(this.#o.now())
    for (let voice = 0; voice < this.#shots.length; voice++) {
      const s = this.#shots[voice]
      if (s.live && (all || s.positioned)) this.#updateShot(voice, s)
    }
    for (const [handle, bed] of this.#beds) if (bed.positional) this.#updateBed(handle, bed.state)
  }
  #playMusic(style: string, o: { seed?: number; intensity?: number }): void {
    const index = styleIndex(style)
    if (index < 0) return
    if (!this.#musicHandle) {
      const def = getBed('music')
      if (!def) { this.#warn('music'); return }
      this.#musicValues.set(def.defaults)
    }
    this.#musicValues[MUSIC_PARAMS.indexOf('style')] = index
    if (o.intensity !== undefined) this.#musicValues[MUSIC_PARAMS.indexOf('intensity')] = o.intensity
    if (this.#musicHandle) this.#musicHandle.set(this.#musicValues)
    else this.#musicHandle = this.startBed('music', { seed: o.seed, values: this.#musicValues })
    if (this.#musicHandle) this.#style = style
  }
}
