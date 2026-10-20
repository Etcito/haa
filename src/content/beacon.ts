// The HAA positional demo source (module spatial-demo): the 'beacon' bed. DSP-safe.
// A continuous, easy-to-locate sound for the spatial demo and the M5 sweep/flyby renders:
//   - a glass-like ping, a three-mode ModalBank (around 880, 2215 and 3960 Hz, each with a
//     seeded +/-2..3 % jitter and its own decay, shorter for higher modes) excited `rate` times
//     per second by a short Hann-enveloped white-noise burst;
//   - a soft pink-noise hum through an Svf band-pass at 300 Hz, about 15 dB under the ping
//     peak, so the energy never drops out between pings;
//   - every mode frequency x doppler (written by the bed host, since the bed is positional).
// The output is mono, ADDED equally to l and r; the bed host spatializes it. No raw oscillator.
import { BLOCK } from '../contract.ts'
import type { BedDef, BedVoice } from '../contract.ts'
import { AudioRng } from '../dsp/rng.ts'
import { DcBlocker, Svf } from '../dsp/filters.ts'
import { PinkNoise, WhiteNoise } from '../dsp/noise.ts'
import { clamp, finiteOr } from '../dsp/math.ts'
import { ModalBank } from '../dsp/modal.ts'
import type { Mode } from '../dsp/modal.ts'

/** Param indices at p[offset..]. */
const P_LEVEL = 0
const P_RATE = 1
const P_DOPPLER = 2
/** Ping rate clamp (pings per second). */
const RATE_MIN = 0.5
const RATE_MAX = 20
/** Above this rate the burst level falls as sqrt(RATE_REF / rate), so dense pings stay in range. */
const RATE_REF = 4
/** Doppler ratio clamp. */
const DOPPLER_MIN = 0.5
const DOPPLER_MAX = 2
/** Level clamp (linear). */
const LEVEL_MAX = 4
/** Base mode frequencies (Hz), their t60 (s) and gains. */
const MODE_HZ = [880, 2215, 3960] as const
const MODE_T60 = [0.9, 0.5, 0.28] as const
const MODE_GAIN = [1, 0.55, 0.35] as const
/** Seeded frequency jitter: a magnitude in 2..3 % with a random sign. */
const JITTER_LO = 0.02
const JITTER_HI = 0.03
/** Seeded t60 spread (+/- fraction). */
const T60_SPREAD = 0.1
/** Excitation burst length (s). */
const BURST_SEC = 0.003
/** Ping gain after the modal bank, tuned for a nominal -6 dBFS peak at level 1. */
const PING_GAIN = 0.045
/** Hum band-pass centre (Hz), Q and gain (about 15 dB under the ping peak). */
const HUM_HZ = 300
const HUM_Q = 2
const HUM_GAIN = 0.3

/**
 * The beacon voice. Params (level, rate, doppler) are copied from p[offset..] every block;
 * every buffer and mode record is preallocated, so process() allocates nothing.
 */
export class BeaconModel implements BedVoice {
  private readonly sr: number
  private readonly bank: ModalBank
  private readonly modes: Mode[]
  private readonly whiteRng: AudioRng
  private readonly pinkRng: AudioRng
  private readonly whiteSeed: number
  private readonly pinkSeed: number
  private readonly white: WhiteNoise
  private readonly pink: PinkNoise
  private readonly hum = new Svf()
  private readonly dc = new DcBlocker()
  private readonly buf = new Float32Array(BLOCK)
  private readonly burstLen: number
  private doppler = Number.NaN
  /** Samples since the last ping, and the samples left in the current burst. */
  private sinceBurst = Number.POSITIVE_INFINITY
  private burstPos = 0

  constructor(sr: number, seed: number) {
    this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000
    const rng = new AudioRng(seed)
    this.modes = []
    for (let i = 0; i < MODE_HZ.length; i++) {
      const sign = rng.next() < 0.5 ? -1 : 1
      const jitter = sign * rng.range(JITTER_LO, JITTER_HI)
      const t60 = MODE_T60[i] * (1 + rng.range(-T60_SPREAD, T60_SPREAD))
      this.modes.push({ hz: MODE_HZ[i] * (1 + jitter), t60, gain: MODE_GAIN[i] })
    }
    this.bank = new ModalBank(this.modes.length, this.sr)
    this.whiteRng = rng.fork(0x9e3779b9)
    this.pinkRng = rng.fork(0x85ebca6b)
    this.whiteSeed = this.whiteRng.state
    this.pinkSeed = this.pinkRng.state
    this.white = new WhiteNoise(this.whiteRng)
    this.pink = new PinkNoise(this.pinkRng)
    this.hum.set(HUM_HZ, HUM_Q, this.sr)
    this.burstLen = Math.max(2, Math.round(BURST_SEC * this.sr))
    this.setDoppler(1)
  }

  private setDoppler(d: number): void {
    if (d === this.doppler) return
    this.doppler = d
    this.bank.setModes(this.modes, d)
  }

  process(p: Float32Array, offset: number, l: Float32Array, r: Float32Array, n: number): void {
    if (offset < 0) return
    const len = Math.max(0, Math.min(n | 0, l.length, r.length, BLOCK))
    if (len === 0) return
    const level = clamp(offset + P_LEVEL < p.length ? p[offset + P_LEVEL] : 0, 0, LEVEL_MAX)
    const rate = clamp(offset + P_RATE < p.length ? p[offset + P_RATE] : RATE_REF, RATE_MIN, RATE_MAX)
    const dop = clamp(
      finiteOr(offset + P_DOPPLER < p.length ? p[offset + P_DOPPLER] : 1, 1), DOPPLER_MIN, DOPPLER_MAX)
    this.setDoppler(dop)

    const sr = this.sr
    const period = sr / rate
    const burstAmp = rate > RATE_REF ? Math.sqrt(RATE_REF / rate) : 1
    const bl = this.burstLen
    const buf = this.buf
    let ok = true
    for (let i = 0; i < len; i++) {
      if (this.sinceBurst >= period) {
        this.sinceBurst = 0
        this.burstPos = 0
      }
      this.sinceBurst++
      let exc = 0
      if (this.burstPos < bl) {
        // Hann-enveloped white-noise burst.
        const env = 0.5 - 0.5 * Math.cos((2 * Math.PI * (this.burstPos + 0.5)) / bl)
        exc = burstAmp * env * this.white.next()
        this.burstPos++
      }
      const ping = PING_GAIN * this.bank.process(exc)
      this.hum.process(this.pink.next())
      const s = this.dc.process(ping + HUM_GAIN * this.hum.bp) * level
      if (!Number.isFinite(s)) ok = false
      buf[i] = s
    }
    // Guards: a non-finite sample drops the block and resets the model.
    if (!ok) {
      this.reset()
      return
    }
    for (let i = 0; i < len; i++) {
      l[i] += buf[i]
      r[i] += buf[i]
    }
  }

  reset(): void {
    this.bank.reset()
    this.hum.reset()
    this.dc.reset()
    this.pink.reset()
    this.white.reset()
    this.whiteRng.state = this.whiteSeed
    this.pinkRng.state = this.pinkSeed
    this.buf.fill(0)
    this.sinceBurst = Number.POSITIVE_INFINITY
    this.burstPos = 0
  }
}

/** The beacon bed: a positional effects-bus source whose 'doppler' param the bed host writes. */
export const BEACON_BED: BedDef = {
  id: 'beacon',
  bus: 'effects',
  params: ['level', 'rate', 'doppler'],
  defaults: [1, 4, 1],
  positional: true,
  create: (sr: number, seed: number): BedVoice => new BeaconModel(sr, seed),
}
