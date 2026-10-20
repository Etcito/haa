// Royal Fiesta crowd core (module fiesta-content), DSP-safe and newly authored for HAA.
// Babble: seeded, panned talkers excite three gliding vowel formants with lowpassed
// glottal pulses and syllable AM. Murmur: two pink-noise bands sit underneath the voices.
// Excitement adds density, pitch, brightness and 6 dB; a 0.3 s swell adds up to 9 dB.
// Claps: above excitement 0.3, seeded velvet-noise bursts through 1..2.5 kHz bandpasses with
// exponential decays and seeded pans. Whistles: above 0.6, rare playbacks of FM-gliding
// harmonicStack shapes prerendered at construction. A private 'aww' mood (renderCrowdShot)
// moves vowels to o/u, lets the shot pitch fall and silences claps and whistles.
// A short internal FDN, stereo DC blockers and a sample-ramped level finish the bed.
// All voices and scratch buffers are constructed up front; inactive talkers do no work.
import { AudioRng, BLOCK, dsp, gen, hashSeed, type BedVoice } from '@etcito/haa'

/** Crowd bed parameters, read at p[offset..] in this order. */
export const CROWD_PARAMS = ['size', 'excitement', 'swell', 'level'] as const

const MAX_VOICES = 48
const BABBLE_GAIN = 0.5
const MURMUR_GAIN = 0.15
const REVERB_GAIN = 0.12
const SWELL_GAIN = dsp.dbToGain(9) - 1
// HAA tuning: -5 dB output trim after the room; with +6 dB excitement and the +9 dB swell the
// untrimmed bed reached +2.6 dBFS sample peak in the m6 12 s sweep; now -5.4 dBFS in the swell
// and at most -2.4 dBFS at size 1, excitement 1 with the swell on.
const OUTPUT_GAIN = dsp.dbToGain(-5)
// HAA tuning: a fast bus compressor before the trim lowers the crest factor of swell and clap
// peaks (claps stay audible: 1 ms attack only shaves the top, the 2 ms+ burst body survives);
// the swell still adds about 8 dB RMS of its 9 dB.
const PEAK_COMP = { thresholdDb: -6, ratio: 3, kneeDb: 6, attackMs: 1, releaseMs: 120, makeupDb: 0 }
/** Clap voices, bursts per second at excitement 1 and size 1, and the burst level. */
const CLAP_VOICES = 24
const CLAP_RATE = 34
const CLAP_GAIN = 0.13
/** Whistle shapes prerendered at construction, players, rate at excitement 1 and level. */
const WHISTLE_SHAPES = 4
const WHISTLE_PLAYERS = 2
const WHISTLE_RATE = 0.7
const WHISTLE_GAIN = 0.1
const WHISTLE_MAX_SECONDS = 0.9
/** Vowel indices of the 'aww' mood: o and u. */
const AWW_VOWELS = [3, 4] as const
/** renderCrowdShot fades (s) and nominal peak (dBFS). */
const SHOT_FADE_IN = 0.015
const SHOT_FADE_OUT = 0.04
const SHOT_PEAK_DB = -6
/** First three formants in Hz: a, e, i, o, u. */
const VOWELS = [
  [800, 1150, 2900],
  [500, 1750, 2450],
  [300, 2200, 3000],
  [450, 800, 2830],
  [325, 700, 2530],
] as const

/** A preallocated babble voice; its only output is the sum of excited formants. */
class BabbleVoice {
  private readonly sr: number
  private readonly rng: AudioRng
  private readonly rngState: number
  private readonly f0: number
  private readonly formantScale: number
  private readonly gainL: number
  private readonly gainR: number
  private readonly initialPulse: number
  private readonly initialAm: number
  private readonly initialVowel: number
  private readonly initialVowelSamples: number
  private readonly rotC: number
  private readonly rotS: number
  private readonly pulse = new dsp.Phasor()
  private readonly glottis = new dsp.OnePole()
  private readonly f1 = new dsp.Svf()
  private readonly f2 = new dsp.Svf()
  private readonly f3 = new dsp.Svf()
  private readonly hz = new Float64Array(3)
  private readonly targetHz = new Float64Array(3)
  private vowelLeft = 0
  private jitter = 0
  private amC = 1
  private amS = 0

  constructor(sr: number, seed: number) {
    this.sr = sr
    this.rng = new AudioRng(seed)
    this.f0 = this.rng.range(110, 260)
    this.formantScale = this.rng.range(0.88, 1.12)
    const pan = this.rng.range(-0.95, 0.95)
    const gain = this.rng.range(0.75, 1)
    this.gainL = gain * Math.cos((pan + 1) * Math.PI / 4)
    this.gainR = gain * Math.sin((pan + 1) * Math.PI / 4)
    this.initialPulse = this.rng.next()
    this.initialAm = dsp.TAU * this.rng.next()
    const amStep = dsp.TAU * this.rng.range(3, 6) / sr
    this.rotC = Math.cos(amStep)
    this.rotS = Math.sin(amStep)
    this.initialVowel = this.rng.int(0, VOWELS.length - 1)
    this.initialVowelSamples = Math.round(sr * this.rng.range(0.15, 0.5))
    this.rngState = this.rng.state
    this.reset()
  }

  // --- Babble control: coefficients change only at block boundaries. ---
  render(l: Float32Array, r: Float32Array, n: number, excitement: number,
    brightness: number, gain: number, glide: number, pitch: number, aww: boolean): void {
    if (this.vowelLeft <= 0) {
      const vowel = aww ? VOWELS[AWW_VOWELS[this.rng.int(0, 1)]]
        : VOWELS[this.rng.int(0, VOWELS.length - 1)]
      for (let j = 0; j < 3; j++) this.targetHz[j] = vowel[j] * this.formantScale
      this.vowelLeft += Math.round(this.sr * this.rng.range(0.15, 0.5))
    }
    this.vowelLeft -= n
    for (let j = 0; j < 3; j++) this.hz[j] += (this.targetHz[j] - this.hz[j]) * glide
    this.f1.set(this.hz[0], 5, this.sr)
    this.f2.set(this.hz[1], 7, this.sr)
    this.f3.set(this.hz[2], 9, this.sr)
    this.jitter += (this.rng.range(-0.018, 0.018) - this.jitter) * glide
    this.pulse.set(this.f0 * pitch * (1 + 0.24 * excitement) * (1 + this.jitter), this.sr)
    this.glottis.setLowpass(1500 + 1900 * brightness, this.sr)
    // Renormalize the AM rotator once per block; no trig or coefficient work per sample.
    const norm = 1 / Math.sqrt(this.amC * this.amC + this.amS * this.amS)
    let c = this.amC * norm
    let s = this.amS * norm
    // An 'aww' is one sustained, sighing vowel rather than separate syllables.
    const floor = aww ? 0.55 : 0.04 + 0.22 * excitement
    // Mean square of floor + (1 - floor) * ((1 + sin(phase)) / 2)^2.
    // Hold syllable energy steady as excitement fills in the gaps between words.
    const amPower = floor * floor + 0.75 * floor * (1 - floor)
      + (35 / 128) * (1 - floor) * (1 - floor)
    const upper2 = 0.42 + 0.20 * brightness
    const upper3 = 0.16 + 0.16 * brightness
    const gl = gain * this.gainL / Math.sqrt(amPower)
    const gr = gain * this.gainR / Math.sqrt(amPower)
    const f1 = this.f1, f2 = this.f2, f3 = this.f3
    for (let i = 0; i < n; i++) {
      const phase = this.pulse.next()
      // A short glottal closure pulse ALWAYS passes through the excitation lowpass.
      const excitation = this.glottis.process(phase < 0.12 ? 1 - phase / 0.12 : 0)
      f1.process(excitation)
      f2.process(excitation)
      f3.process(excitation)
      const syllable = 0.5 + 0.5 * s
      const am = floor + (1 - floor) * syllable * syllable
      const voice = (f1.bp + upper2 * f2.bp + upper3 * f3.bp) * am
      l[i] += voice * gl
      r[i] += voice * gr
      const nextC = c * this.rotC - s * this.rotS
      s = s * this.rotC + c * this.rotS
      c = nextC
    }
    this.amC = c
    this.amS = s
  }

  /** Pick a new vowel target at the next block (a mood change). */
  retarget(): void {
    this.vowelLeft = 0
  }

  reset(): void {
    this.rng.state = this.rngState
    this.pulse.phase = this.initialPulse
    this.glottis.reset()
    this.f1.reset()
    this.f2.reset()
    this.f3.reset()
    const vowel = VOWELS[this.initialVowel]
    for (let j = 0; j < 3; j++) this.hz[j] = this.targetHz[j] = vowel[j] * this.formantScale
    this.vowelLeft = this.initialVowelSamples
    this.jitter = 0
    this.amC = Math.cos(this.initialAm)
    this.amS = Math.sin(this.initialAm)
  }
}

/** One preallocated hand clap: a velvet-noise burst through a bandpass under an exp decay. */
class ClapVoice {
  private readonly velvet: dsp.VelvetNoise
  private readonly band = new dsp.Svf()
  private readonly env = new dsp.ExpDecay()
  private density = 0
  private gainL = 0
  private gainR = 0
  private burst = 0
  /** Samples until the decay is inaudible; 0 means the voice is idle and costs nothing. */
  left = 0
  /** Triggered and partly rendered in the current block. */
  fresh = false

  constructor(rng: AudioRng) {
    this.velvet = new dsp.VelvetNoise(rng)
  }

  // --- Clap trigger: every coefficient is set once per burst, from the model's rng. ---
  trigger(rng: AudioRng, sr: number, level: number): void {
    this.band.reset()
    this.velvet.reset()
    this.band.set(rng.range(1000, 2500), rng.range(1.1, 2.2), sr)
    const t60 = rng.range(0.05, 0.11)
    this.env.trigger(level * rng.range(0.55, 1), t60, sr)
    this.density = rng.range(2500, 6000)
    this.burst = Math.round(sr * rng.range(0.006, 0.016))
    const pan = rng.range(-0.9, 0.9)
    this.gainL = Math.cos((pan + 1) * Math.PI / 4)
    this.gainR = Math.sin((pan + 1) * Math.PI / 4)
    this.left = Math.round(t60 * sr)
  }

  /** Add frames from..n of the block; no subarray views, so nothing is allocated. */
  render(l: Float32Array, r: Float32Array, from: number, n: number, sr: number): void {
    const m = Math.min(n, from + this.left)
    for (let i = from; i < m; i++) {
      const x = this.burst > 0 ? this.velvet.next(this.density, sr) : 0
      this.burst--
      this.band.process(x * this.env.next())
      const y = this.band.bp
      l[i] += y * this.gainL
      r[i] += y * this.gainR
    }
    this.left -= Math.max(0, m - from)
  }

  reset(): void {
    this.velvet.reset()
    this.band.reset()
    this.env.reset()
    this.burst = 0
    this.left = 0
    this.fresh = false
  }
}

/** A whistle player reading one prerendered shape with a gain envelope. */
interface WhistlePlayer { shape: number; pos: number; gainL: number; gainR: number }

/**
 * Prerender one whistle shape (construction time only; fxKit allocates at call entry).
 * harmonicStack draws the gliding partials; a seeded vibrato then warps the read rate, which
 * frequency-modulates the glide. The shape is scaled to a unit peak; returns its length.
 */
function prerenderWhistle(buf: Float32Array, kind: number, sr: number, rng: AudioRng): number {
  const seconds = rng.range(0.45, WHISTLE_MAX_SECONDS * 0.9)
  const raw = new Float32Array(Math.ceil(seconds * sr) + 4)
  const stack = (at: number, sec: number, f0: number, f1: number): void => {
    gen.harmonicStack(raw, Math.round(at * sr), {
      seconds: sec, f0, f1, partials: 3, tiltDb: -20, inharm: 0, detuneCents: 3,
      attack: 0.03, decay: 1.2, level: 1,
    }, sr, rng)
  }
  if (kind === 0) stack(0, seconds, rng.range(800, 1100), rng.range(2000, 2600))
  else if (kind === 1) stack(0, seconds, rng.range(2200, 2700), rng.range(1000, 1300))
  else if (kind === 2) {
    // The wolf whistle: a rising 'fweet' then a falling 'fwoo'.
    stack(0, 0.38 * seconds, rng.range(1000, 1200), rng.range(2300, 2700))
    stack(0.46 * seconds, 0.54 * seconds, rng.range(1900, 2200), rng.range(900, 1100))
  } else {
    stack(0, 0.42 * seconds, rng.range(1300, 1600), rng.range(2400, 2800))
    stack(0.52 * seconds, 0.48 * seconds, rng.range(1300, 1600), rng.range(2500, 2900))
  }
  const vib = dsp.TAU * rng.range(5, 8) / sr
  const depth = rng.range(0.015, 0.035)
  let pos = 0
  let len = 0
  while (len < buf.length && pos < raw.length - 2) {
    const i = Math.floor(pos)
    const f = pos - i
    buf[len] = raw[i] + (raw[i + 1] - raw[i]) * f
    pos += 1 + depth * Math.sin(vib * len)
    len++
  }
  gen.normalizePeak(buf.subarray(0, len), 0)
  return len
}

/** Continuous stereo crowd core. process() adds to the outputs without allocating. */
export class CrowdModel implements BedVoice {
  private readonly sr: number
  private readonly voices: BabbleVoice[] = []
  private readonly pinkRng: AudioRng
  private readonly pinkSeed: number
  private readonly pink: dsp.PinkNoise
  private readonly murmurLow = new dsp.Svf()
  private readonly murmurHigh = new dsp.Svf()
  private readonly reverb: dsp.FdnReverb
  private readonly dcL = new dsp.DcBlocker()
  private readonly dcR = new dsp.DcBlocker()
  private readonly comp: dsp.Compressor
  private readonly level: dsp.Smoother
  private readonly bufL = new Float32Array(BLOCK)
  private readonly bufR = new Float32Array(BLOCK)
  private readonly swellAttack: number
  private readonly swellRelease: number
  private readonly events: AudioRng
  private readonly eventsSeed: number
  private readonly claps: ClapVoice[] = []
  private readonly whistleBufs: Float32Array[] = []
  private readonly whistleLens = new Int32Array(WHISTLE_SHAPES)
  private readonly whistles: WhistlePlayer[] = []
  private readonly whistleFade: number
  private swell = 0
  private clapWait = -1
  private aww = false
  private pitch = 1

  constructor(sr: number, seed: number) {
    this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000
    for (let i = 0; i < MAX_VOICES; i++) {
      this.voices.push(new BabbleVoice(this.sr, hashSeed(seed, 'crowd-voice', i)))
    }
    this.pinkSeed = hashSeed(seed, 'crowd-murmur')
    this.pinkRng = new AudioRng(this.pinkSeed)
    this.pink = new dsp.PinkNoise(this.pinkRng)
    this.murmurLow.set(400, 0.9, this.sr)
    this.murmurHigh.set(1500, 1.2, this.sr)
    this.reverb = new dsp.FdnReverb(this.sr)
    this.reverb.set({ size: 0.7, decay: 0.5, damping: 3400, predelay: 0.008, returnDb: 0 })
    this.level = new dsp.Smoother(0.015, this.sr)
    this.comp = new dsp.Compressor(this.sr)
    this.comp.set(PEAK_COMP)
    this.swellAttack = 1 / (0.3 * this.sr)
    this.swellRelease = 1 / (0.6 * this.sr)
    this.eventsSeed = hashSeed(seed, 'crowd-events')
    this.events = new AudioRng(this.eventsSeed)
    for (let i = 0; i < CLAP_VOICES; i++) this.claps.push(new ClapVoice(this.events))
    const shapeRng = new AudioRng(hashSeed(seed, 'crowd-whistle'))
    for (let k = 0; k < WHISTLE_SHAPES; k++) {
      const buf = new Float32Array(Math.ceil(WHISTLE_MAX_SECONDS * this.sr))
      this.whistleLens[k] = prerenderWhistle(buf, k, this.sr, shapeRng)
      this.whistleBufs.push(buf)
    }
    for (let i = 0; i < WHISTLE_PLAYERS; i++) {
      this.whistles.push({ shape: -1, pos: 0, gainL: 0, gainR: 0 })
    }
    this.whistleFade = 1 / Math.round(0.03 * this.sr)
    this.reset()
  }

  /** Read size, excitement, swell and level from p[offset..]; blocks are at most BLOCK. */
  process(p: Float32Array, offset: number, l: Float32Array, r: Float32Array, n: number): void {
    const len = Math.max(0, Math.min(n | 0, l.length, r.length, BLOCK))
    if (!Number.isInteger(offset) || offset < 0 || len === 0) return
    try {
      this.processBlock(p, offset, l, r, len)
    } catch {
      // A failed render contributes silence; pre-existing bus audio is untouched.
      this.reset()
    }
  }

  private processBlock(p: Float32Array, offset: number, l: Float32Array,
    r: Float32Array, n: number): void {
    const size = dsp.clamp(dsp.finiteOr(p[offset], 0.6), 0, 1)
    const excitement = dsp.clamp(dsp.finiteOr(p[offset + 1], 0.3), 0, 1)
    const swellTarget = dsp.clamp(dsp.finiteOr(p[offset + 2], 0), 0, 1)
    this.level.target = dsp.clamp(dsp.finiteOr(p[offset + 3], 1), 0, 4)
    this.bufL.fill(0, 0, n)
    this.bufR.fill(0, 0, n)
    const swellEnd = this.swell + dsp.clamp(swellTarget - this.swell,
      -n * this.swellRelease, n * this.swellAttack)
    const brightness = excitement + 0.7 * (this.swell + swellEnd) / 2
    this.renderBabble(n, size, excitement, brightness)
    this.renderMurmur(n, size, brightness)
    this.renderClaps(n, size, excitement)
    this.renderWhistles(n, excitement)
    if (!this.finishBlock(n, excitement, swellTarget)) {
      this.reset()
      return
    }
    for (let i = 0; i < n; i++) {
      l[i] += this.bufL[i]
      r[i] += this.bufR[i]
    }
  }

  // --- Babble layer: only the size-selected voices advance or update coefficients. ---
  private renderBabble(n: number, size: number, excitement: number, brightness: number): void {
    const active = Math.round(6 + 42 * size)
    // Slightly stronger than sqrt(N) compensation keeps a large, dense crowd in range.
    const gain = BABBLE_GAIN / Math.pow(active, 0.65)
    const glide = 1 - Math.exp(-n / (0.045 * this.sr))
    for (let v = 0; v < active; v++) {
      this.voices[v].render(this.bufL, this.bufR, n, excitement, brightness, gain, glide,
        this.pitch, this.aww)
    }
  }

  // --- Murmur layer: a quiet, continuous body below the articulated babble. ---
  private renderMurmur(n: number, size: number, brightness: number): void {
    const gain = MURMUR_GAIN * (0.65 + 0.35 * size)
    const highGain = 0.3 + 0.2 * brightness
    for (let i = 0; i < n; i++) {
      const pink = this.pink.next()
      this.murmurLow.process(pink)
      this.murmurHigh.process(pink)
      const murmur = gain * (this.murmurLow.bp + highGain * this.murmurHigh.bp)
      this.bufL[i] += murmur * Math.SQRT1_2
      this.bufR[i] += murmur * Math.SQRT1_2
    }
  }

  // --- Clap layer: Poisson onsets from the model rng, zero rate at excitement <= 0.3. ---
  private renderClaps(n: number, size: number, excitement: number): void {
    const drive = this.aww ? 0 : Math.max(0, (excitement - 0.3) / 0.7)
    const rate = CLAP_RATE * drive * (0.35 + 0.65 * size) * (1 + 0.5 * this.swell)
    if (rate > 0) {
      if (this.clapWait < 0) this.clapWait = this.clapInterval(rate)
      // Onsets land on the block grid plus a seeded offset inside the block.
      while (this.clapWait < n) {
        const voice = this.freeClap()
        if (voice !== null) {
          const start = Math.floor(this.clapWait)
          voice.trigger(this.events, this.sr, CLAP_GAIN)
          voice.render(this.bufL, this.bufR, start, n, this.sr)
        }
        this.clapWait += this.clapInterval(rate)
      }
      this.clapWait -= n
    } else {
      this.clapWait = -1
    }
    for (let c = 0; c < CLAP_VOICES; c++) {
      const clap = this.claps[c]
      if (clap.left > 0 && !clap.fresh) clap.render(this.bufL, this.bufR, 0, n, this.sr)
      clap.fresh = false
    }
  }

  private clapInterval(rate: number): number {
    return -Math.log(1 - this.events.next()) * this.sr / rate
  }

  private freeClap(): ClapVoice | null {
    for (let c = 0; c < CLAP_VOICES; c++) {
      const clap = this.claps[c]
      if (clap.left <= 0) {
        clap.fresh = true
        return clap
      }
    }
    return null
  }

  // --- Whistle layer: rare prerendered glides above excitement 0.6, read with a gain ramp. ---
  private renderWhistles(n: number, excitement: number): void {
    const rate = this.aww ? 0 : WHISTLE_RATE * Math.max(0, (excitement - 0.6) / 0.4)
    if (rate > 0 && this.events.next() < rate * n / this.sr) {
      for (let w = 0; w < WHISTLE_PLAYERS; w++) {
        const player = this.whistles[w]
        if (player.shape >= 0) continue
        player.shape = this.events.int(0, WHISTLE_SHAPES - 1)
        player.pos = 0
        const pan = this.events.range(-0.8, 0.8)
        const gain = WHISTLE_GAIN * this.events.range(0.6, 1)
        player.gainL = gain * Math.cos((pan + 1) * Math.PI / 4)
        player.gainR = gain * Math.sin((pan + 1) * Math.PI / 4)
        break
      }
    }
    for (let w = 0; w < WHISTLE_PLAYERS; w++) {
      const player = this.whistles[w]
      if (player.shape < 0) continue
      const buf = this.whistleBufs[player.shape]
      const len = this.whistleLens[player.shape]
      const m = Math.min(n, len - player.pos)
      for (let i = 0; i < m; i++) {
        const at = player.pos + i
        const env = Math.min(1, at * this.whistleFade, (len - 1 - at) * this.whistleFade)
        const x = buf[at] * env
        this.bufL[i] += x * player.gainL
        this.bufR[i] += x * player.gainR
      }
      player.pos += m
      if (player.pos >= len) player.shape = -1
    }
  }

  /**
   * Shot mood, used by renderCrowdShot: 'aww' (true) moves every vowel target to o/u, mutes
   * new claps and whistles and multiplies the babble f0 by pitch (the falling shot pitch).
   */
  setShotMood(aww: boolean, pitch: number): void {
    if (aww !== this.aww) {
      this.aww = aww
      for (let v = 0; v < MAX_VOICES; v++) this.voices[v].retarget()
    }
    this.pitch = dsp.clamp(dsp.finiteOr(pitch, 1), 0.25, 4)
  }

  // --- Shared envelope, short room, DC removal and transactional output guard. ---
  private finishBlock(n: number, excitement: number, swellTarget: number): boolean {
    const excitementGain = dsp.dbToGain(6 * excitement)
    let ok = true
    for (let i = 0; i < n; i++) {
      this.swell += dsp.clamp(swellTarget - this.swell, -this.swellRelease, this.swellAttack)
      const gain = excitementGain * (1 + SWELL_GAIN * this.swell)
      const dryL = this.bufL[i] * gain
      const dryR = this.bufR[i] * gain
      this.reverb.process(dryL, dryR)
      this.bufL[i] = this.dcL.process(dryL + REVERB_GAIN * this.reverb.outL)
      this.bufR[i] = this.dcR.process(dryR + REVERB_GAIN * this.reverb.outR)
    }
    // HAA tuning: compress the summed block (crest control), then trim and apply the level.
    this.comp.processStereo(this.bufL, this.bufR, n)
    for (let i = 0; i < n; i++) {
      const level = this.level.next() * OUTPUT_GAIN
      this.bufL[i] *= level
      this.bufR[i] *= level
      // Test the Float32 values as stored, including any overflow during conversion.
      if (!Number.isFinite(this.bufL[i]) || !Number.isFinite(this.bufR[i])) ok = false
    }
    return ok
  }

  /** Restore the initial phases, vowel controls, filter histories and every RNG stream. */
  reset(): void {
    for (let v = 0; v < MAX_VOICES; v++) this.voices[v].reset()
    this.pinkRng.state = this.pinkSeed
    this.pink.reset()
    this.murmurLow.reset()
    this.murmurHigh.reset()
    this.reverb.reset()
    this.dcL.reset()
    this.dcR.reset()
    this.comp.reset()
    this.level.jump(0)
    this.swell = 0
    this.events.state = this.eventsSeed
    for (let c = 0; c < CLAP_VOICES; c++) this.claps[c].reset()
    for (let w = 0; w < WHISTLE_PLAYERS; w++) this.whistles[w].shape = -1
    this.clapWait = -1
    this.bufL.fill(0)
    this.bufR.fill(0)
  }
}

/**
 * Render a mono crowd one-shot (bake/fallback time; it allocates its model and scratch).
 * A fresh CrowdModel seeded from rng runs block by block at o.size and o.excitement; the
 * swell is 1 from o.swellAt for o.swellSeconds (seconds), then releases. 'aww' moves the
 * vowels to o/u, lets the pitch fall over the shot and has no claps or whistles. The (L+R)/2
 * mix is DC-blocked, gets a 15 ms fade-in and a 40 ms cosine fade-out, is scaled to a -6 dBFS
 * peak and is ADDED into out. Returns the frames written (at most out.length).
 */
export function renderCrowdShot(
  out: Float32Array,
  o: { seconds: number; excitement: number; swellAt: number; swellSeconds: number; size: number; mood: 'cheer' | 'aww' },
  sr: number,
  rng: AudioRng,
): number {
  if (!(sr > 1) || !Number.isFinite(sr) || !(o.seconds > 0) || !Number.isFinite(o.seconds)) return 0
  const frames = Math.min(out.length, Math.round(o.seconds * sr))
  if (frames <= 0) return 0
  const model = new CrowdModel(sr, Math.floor(rng.next() * 4294967296))
  const aww = o.mood === 'aww'
  const swellAt = dsp.finiteOr(o.swellAt, 0) * sr
  const swellEnd = swellAt + Math.max(0, dsp.finiteOr(o.swellSeconds, 0)) * sr
  const fall = aww ? rng.range(0.62, 0.75) : 1
  const p = new Float32Array([o.size, o.excitement, 0, 1])
  const l = new Float32Array(BLOCK)
  const r = new Float32Array(BLOCK)
  const mono = new Float32Array(frames)
  const dc = new dsp.DcBlocker()
  for (let at = 0; at < frames; at += BLOCK) {
    const n = Math.min(BLOCK, frames - at)
    p[2] = at >= swellAt && at < swellEnd ? 1 : 0
    // The 'aww' pitch falls on an ease-in curve from 1 to `fall` over the shot.
    const x = at / frames
    model.setShotMood(aww, aww ? 1 + (fall - 1) * x * x : 1)
    l.fill(0)
    r.fill(0)
    model.process(p, 0, l, r, n)
    for (let i = 0; i < n; i++) mono[at + i] = dc.process(0.5 * (l[i] + r[i]))
  }
  const fadeIn = Math.max(1, Math.round(SHOT_FADE_IN * sr))
  const fadeOut = Math.max(1, Math.round(SHOT_FADE_OUT * sr))
  for (let i = 0; i < frames; i++) {
    let g = 1
    if (i < fadeIn) g = i / fadeIn
    const remain = frames - 1 - i
    if (remain < fadeOut) g *= 0.5 - 0.5 * Math.cos(Math.PI * remain / fadeOut)
    mono[i] = Number.isFinite(mono[i]) ? mono[i] * g : 0
  }
  gen.normalizePeak(mono, SHOT_PEAK_DB)
  for (let i = 0; i < frames; i++) out[i] += mono[i]
  return frames
}
