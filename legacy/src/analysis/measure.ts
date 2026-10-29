// Measurement of rendered audio for the proofs and reports (shared 'Measurement and reports').
// DSP-safe (no DOM, no Web Audio, no node builtins). Built on the dsp meter: BS.1770-4
// loudness, true peak, sample peak, RMS and windowed RMS. Octave bands are two cascaded RBJ
// bandpass biquads per center on the mono sum. Non-finite samples are counted, then measured
// as 0 by every other measure. safetyVerdict applies SAFETY and names each failure.

import { AUDIO_RATE, SAFETY } from '../contract.ts'
import { Biquad } from '../dsp/filters.ts'
import { gainToDb } from '../dsp/math.ts'
import { LoudnessMeter, countNonFinite, rmsDb, samplePeakDb, truePeakDb, windowRmsDb } from '../dsp/meter.ts'

/** The measures of one rendered signal (mono or stereo). Levels in dBFS, dBTP and LUFS. */
export interface Measure {
  frames: number
  seconds: number
  nonFinite: number
  samplePeakDb: number
  truePeakDb: number
  dc: number
  rmsDb: number
  minWindowRmsDb: number
  lufs: number
  shortTermMaxLufs: number
  bandsDb: number[]
}

/** Octave band centers (Hz) of Measure.bandsDb. */
export const OCTAVE_BANDS: readonly number[] = [31.5, 63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000]

/** Q of each of the two cascaded bandpass biquads of a band (one octave wide). */
const BAND_Q = Math.SQRT2
/** Level reported for silence and for a band at or above Nyquist (the gainToDb floor). */
const FLOOR_DB = -200
/** RMS window of minWindowRmsDb (s). */
const WINDOW_SECONDS = 1
/** Shorter signals report minWindowRmsDb = rmsDb. */
const MIN_WINDOWED_SECONDS = 2
/** Signals are metered for loudness over at least one 400 ms BS.1770 gating block (zero-padded), as oneShotLufs does. */
const LUFS_MIN_SECONDS = 0.4

/** A usable sample rate: non-finite or tiny rates fall back to AUDIO_RATE. */
function safeSr(sr: number): number {
  return sr > 1 && Number.isFinite(sr) ? sr : AUDIO_RATE
}

/** x as n finite frames: x itself when it already is, else a copy with non-finite and missing samples set to 0. */
function finiteChannel(x: Float32Array, n: number, nonFinite: number): Float32Array {
  if (x.length === n && nonFinite === 0) return x
  const y = new Float32Array(n)
  const m = Math.min(n, x.length)
  for (let i = 0; i < m; i++) {
    const v = x[i]
    y[i] = Number.isFinite(v) ? v : 0
  }
  return y
}

/** The RMS level over all channels from per-channel RMS levels (power mean), floored at -200. */
function powerMeanDb(levels: readonly number[]): number {
  if (levels.length === 1) return levels[0]
  let p = 0
  for (const db of levels) p += Math.pow(10, db / 10)
  return gainToDb(Math.sqrt(p / levels.length))
}

/** |mean| of x; 0 when empty. */
function absMean(x: Float32Array): number {
  if (x.length === 0) return 0
  let sum = 0
  for (let i = 0; i < x.length; i++) sum += x[i]
  return Math.abs(sum / x.length)
}

/** Minimum over consecutive 1 s windows of the RMS over all channels; rms when shorter than 2 s. */
function minWindowRms(chs: readonly Float32Array[], sr: number, rms: number): number {
  if (chs[0].length / sr < MIN_WINDOWED_SECONDS) return rms
  const wins = chs.map((c) => windowRmsDb(c, sr, WINDOW_SECONDS))
  let min = Infinity
  for (let w = 0; w < wins[0].length; w++) {
    const db = powerMeanDb(wins.map((x) => x[w]))
    if (db < min) min = db
  }
  return min
}

/** RMS dBFS of mono per OCTAVE_BANDS center: two cascaded RBJ bandpass biquads, Q √2. */
function octaveBandsDb(mono: Float32Array, sr: number): number[] {
  const n = mono.length
  const buf = new Float32Array(n)
  const a = new Biquad()
  const b = new Biquad()
  return OCTAVE_BANDS.map((hz) => {
    if (hz >= sr / 2) return FLOOR_DB
    a.set('bandpass', hz, BAND_Q, 0, sr)
    b.set('bandpass', hz, BAND_Q, 0, sr)
    a.reset()
    b.reset()
    buf.set(mono)
    a.processBlock(buf, n)
    b.processBlock(buf, n)
    return rmsDb(buf, 0, n)
  })
}

/**
 * The shared core. chs are the finite channels (1 or 2, equal length) measured for peaks,
 * dc and RMS; mono feeds the bands; (loudL, loudR) is the pair metered for loudness.
 */
function measureFinite(
  chs: readonly Float32Array[],
  mono: Float32Array,
  loudL: Float32Array,
  loudR: Float32Array,
  nonFinite: number,
  sr: number,
): Measure {
  const n = mono.length
  const l = chs[0]
  const r = chs.length > 1 ? chs[1] : null
  let dc = 0
  for (const c of chs) dc = Math.max(dc, absMean(c))
  const rms = powerMeanDb(chs.map((c) => rmsDb(c, 0, n)))

  const meter = new LoudnessMeter(sr)
  meter.push(loudL, loudR, n)
  const pad = Math.ceil(LUFS_MIN_SECONDS * sr) - n
  if (pad > 0) {
    const zeros = new Float32Array(pad)
    meter.push(zeros, zeros, pad)
  }

  return {
    frames: n,
    seconds: n / sr,
    nonFinite,
    samplePeakDb: samplePeakDb(l, r),
    truePeakDb: truePeakDb(l, r, sr),
    dc,
    rmsDb: rms,
    minWindowRmsDb: minWindowRms(chs, sr, rms),
    lufs: meter.integrated(),
    shortTermMaxLufs: meter.shortTermMax(),
    bandsDb: octaveBandsDb(mono, sr),
  }
}

/**
 * Measures a stereo signal. A shorter channel counts as zero-padded to the longer one. dc is
 * the max over channels of |mean|, rmsDb the RMS over both channels, and the bands use the
 * mono sum (l + r) / 2. lufs is the BS.1770-4 integrated loudness of (l, r), metered over at
 * least 0.4 s; -Infinity when gated out (shortTermMaxLufs: shorter than 3 s).
 */
export function measureStereo(l: Float32Array, r: Float32Array, sr: number): Measure {
  const rate = safeSr(sr)
  const n = Math.max(l.length, r.length)
  const badL = countNonFinite(l)
  const badR = countNonFinite(r)
  const cl = finiteChannel(l, n, badL)
  const cr = finiteChannel(r, n, badR)
  const mono = new Float32Array(n)
  for (let i = 0; i < n; i++) mono[i] = 0.5 * (cl[i] + cr[i])
  return measureFinite([cl, cr], mono, cl, cr, badL + badR, rate)
}

/**
 * Measures a mono signal. Loudness is that of the centered equal-power pair (x√½, x√½), so
 * lufs equals oneShotLufs(x); the peaks, dc, RMS and bands use x itself.
 */
export function measureMono(x: Float32Array, sr: number): Measure {
  const rate = safeSr(sr)
  const n = x.length
  const bad = countNonFinite(x)
  const cx = finiteChannel(x, n, bad)
  const half = new Float32Array(n)
  for (let i = 0; i < n; i++) half[i] = cx[i] * Math.SQRT1_2
  return measureFinite([cx], cx, half, half, bad, rate)
}

/**
 * Applies SAFETY to a measure. Each failed rule is named: 'nonFinite' (any NaN/Inf sample),
 * 'samplePeak' (above -1 dBFS), 'dc' (|DC| of 1e-3 or more) and 'silence' (the minimum window
 * RMS below -60 dBFS). A NaN measure fails its rule.
 */
export function safetyVerdict(m: Measure): { ok: boolean; failures: string[] } {
  const failures: string[] = []
  if (m.nonFinite !== 0) failures.push('nonFinite')
  if (!(m.samplePeakDb <= SAFETY.maxSamplePeakDb)) failures.push('samplePeak')
  if (!(Math.abs(m.dc) < SAFETY.maxAbsDc)) failures.push('dc')
  if (!(m.minWindowRmsDb >= SAFETY.minRmsDb)) failures.push('silence')
  return { ok: failures.length === 0, failures }
}
