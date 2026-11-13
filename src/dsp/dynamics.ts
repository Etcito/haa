// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe dynamics: bus/glue compressor, lookahead true-peak limiter, ducker and transient
// shaper. Sample-rate agnostic (sr passed in). No allocation after construction; every class
// has reset(). Runs unchanged in node, on the main thread and in AudioWorkletGlobalScope.

import type { CompPreset } from '../contract.ts'
import { clamp, finiteOr, gainToDb } from './math.ts'
import { TruePeakDetector } from './meter.ts'

/** dB to natural-log gain exponent: gain = exp(db * DB_TO_LN). */
const DB_TO_LN = Math.LN10 / 20

/** A usable sample rate: non-finite or tiny rates fall back to 48 kHz. */
function safeSr(sr: number): number {
  return sr > 1 && Number.isFinite(sr) ? sr : 48000
}

/** One-pole approach coefficient for a time constant in ms (0 = instant). */
function coefMs(ms: number, sr: number): number {
  const t = finiteOr(ms, 0)
  if (!(t > 0)) return 0
  return Math.exp(-1000 / (t * sr))
}

/** Smallest power of two >= n. */
function pow2(n: number): number {
  let p = 1
  while (p < n) p <<= 1
  return p
}

// ---------------------------------------------------------------------------------------
// Compressor
// ---------------------------------------------------------------------------------------

/**
 * Stereo-linked feed-forward compressor. The instantaneous peak max(|l|, |r|) feeds a
 * log-domain gain computer with a quadratic soft knee; the gain reduction (dB) is smoothed
 * with separate attack/release one-poles, then the makeup gain is applied.
 */
export class Compressor {
  private readonly sr: number
  private threshold = 0
  private slope = 0 // 1/ratio - 1 (<= 0)
  private knee = 0
  private kneeLoLin = 1 // below this linear peak the computer returns 0 dB
  private attackCoef = 0
  private releaseCoef = 0
  private makeupDb = 0
  private env = 0 // smoothed gain reduction, dB >= 0
  private maxRed = 0

  constructor(sr: number) {
    this.sr = safeSr(sr)
    this.set({ thresholdDb: 0, ratio: 1, kneeDb: 0, attackMs: 10, releaseMs: 100, makeupDb: 0 })
  }

  /** Last block's maximum gain reduction, in dB (>= 0). */
  get reductionDb(): number {
    return this.maxRed
  }

  set(p: CompPreset): void {
    const ratio = Math.max(1, finiteOr(p.ratio, 1))
    this.threshold = clamp(finiteOr(p.thresholdDb, 0), -120, 24)
    this.slope = 1 / ratio - 1
    this.knee = clamp(finiteOr(p.kneeDb, 0), 0, 48)
    this.kneeLoLin = Math.exp((this.threshold - this.knee / 2) * DB_TO_LN)
    this.attackCoef = coefMs(p.attackMs, this.sr)
    this.releaseCoef = coefMs(p.releaseMs, this.sr)
    this.makeupDb = clamp(finiteOr(p.makeupDb, 0), -48, 48)
  }

  /** Static gain computer: the desired gain reduction (dB >= 0) for a peak in dB. */
  private computeReduction(xDb: number): number {
    const over = xDb - this.threshold
    const w = this.knee
    if (2 * over < -w) return 0
    if (w > 0 && 2 * over <= w) {
      const k = over + w / 2
      return (-this.slope * k * k) / (2 * w)
    }
    return -this.slope * over
  }

  processStereo(l: Float32Array, r: Float32Array, n: number): void {
    let env = this.env
    let maxRed = 0
    const aC = this.attackCoef
    const rC = this.releaseCoef
    const makeup = this.makeupDb
    const lo = this.kneeLoLin
    const idle = this.slope === 0
    for (let i = 0; i < n; i++) {
      const a = l[i]
      const b = r[i]
      let x = a < 0 ? -a : a
      const y = b < 0 ? -b : b
      if (y > x) x = y
      if (!(x < Infinity)) x = 0 // NaN / Inf never enter the detector state
      const target = idle || x <= lo ? 0 : this.computeReduction(gainToDb(x))
      const c = target > env ? aC : rC
      env = target + c * (env - target)
      if (env < 1e-9) env = 0
      if (env > maxRed) maxRed = env
      const gDb = makeup - env
      if (gDb !== 0) {
        const g = Math.exp(gDb * DB_TO_LN)
        l[i] = a * g
        r[i] = b * g
      }
    }
    this.env = env
    this.maxRed = maxRed
  }

  reset(): void {
    this.env = 0
    this.maxRed = 0
  }
}

// ---------------------------------------------------------------------------------------
// TruePeakLimiter
// ---------------------------------------------------------------------------------------

/** Maximum lookahead the rings are sized for. */
const LIMITER_MAX_LOOKAHEAD_SEC = 0.02
/**
 * The TruePeakDetector reports, at input time p, the interpolated points between x[p-6]
 * and x[p-5]; the output meter's interpolation there uses samples p-11..p. The gain must be
 * fully down over [p-6-M, p-5+M] with M = TP_MARGIN, hence DETECT_SPAN = 5 + M.
 */
const TP_MARGIN = 6
const DETECT_SPAN = 5 + TP_MARGIN
/** Exact recomputation period of the sliding raised-cosine sums (float drift control). */
const RESYNC_PERIOD = 16384
const INDEX_MASK = 0x3fffffff

/**
 * Lookahead true-peak limiter (stereo-linked). Per sample:
 * 1. the input is delayed by the lookahead (one delay line per channel);
 * 2. the peak is detected on the 4x oversampled input (meter.ts TruePeakDetector);
 * 3. the required gain min(1, ceiling / peak) goes through a running minimum over the
 *    lookahead window (monotonic deque in preallocated Int32/Float32 rings);
 * 4. it is smoothed by a raised-cosine (Hann) attack across the lookahead, computed with
 *    O(1) sliding sums, and an exponential release;
 * 5. the delayed sample is multiplied by the gain, then clamped at the ceiling (sample peak).
 * The Hann average of a held minimum reaches the required gain exactly when the peak leaves
 * the delay, so the output true peak stays <= ceiling + 0.2 dB.
 */
export class TruePeakLimiter {
  private readonly sr: number
  private readonly detL = new TruePeakDetector()
  private readonly detR = new TruePeakDetector()
  // Delay lines (power-of-two rings).
  private readonly dl: Float32Array
  private readonly dr: Float32Array
  private readonly dMask: number
  private dPos = 0
  // Running-minimum deque over the hold span.
  private readonly qIdx: Int32Array
  private readonly qVal: Float32Array
  private readonly qMask: number
  private qHead = 0
  private qSize = 0
  private t = 0
  // Raised-cosine smoother over the held minimum.
  private readonly hRing: Float64Array
  private readonly cosT: Float64Array
  private readonly sinT: Float64Array
  private readonly maxAttack: number
  private hPos = 0
  private sumB = 0
  private sumCr = 0
  private sumCi = 0
  private resync = 0
  // Parameters.
  private ceiling = 1
  private attackLen = 1 // Hann length A
  private delay = DETECT_SPAN + 1 // D = A + DETECT_SPAN
  private hold = DETECT_SPAN + 1 + TP_MARGIN // H = D + M
  private relCoef = 0
  private gain = 1
  private minGain = 1

  constructor(sr: number) {
    this.sr = safeSr(sr)
    const maxL = Math.max(1, Math.ceil(LIMITER_MAX_LOOKAHEAD_SEC * this.sr))
    const maxD = maxL + DETECT_SPAN
    const dSize = pow2(maxD + 1)
    this.dl = new Float32Array(dSize)
    this.dr = new Float32Array(dSize)
    this.dMask = dSize - 1
    const qSize = pow2(maxD + TP_MARGIN + 1)
    this.qIdx = new Int32Array(qSize)
    this.qVal = new Float32Array(qSize)
    this.qMask = qSize - 1
    this.hRing = new Float64Array(maxL)
    this.cosT = new Float64Array(maxL + 1)
    this.sinT = new Float64Array(maxL + 1)
    this.maxAttack = maxL
    this.set(-1.2, 5, 100)
  }

  /** Last block's maximum gain reduction, in dB (>= 0). */
  get reductionDb(): number {
    return -gainToDb(this.minGain)
  }

  set(ceilingDbtp: number, lookaheadMs: number, releaseMs: number): void {
    this.ceiling = Math.exp(clamp(finiteOr(ceilingDbtp, -1.2), -60, 0) * DB_TO_LN)
    this.relCoef = coefMs(releaseMs, this.sr)
    const la = Math.round((clamp(finiteOr(lookaheadMs, 5), 0, LIMITER_MAX_LOOKAHEAD_SEC * 1000) * this.sr) / 1000)
    const a = clamp(la - DETECT_SPAN, 1, this.maxAttack)
    if (a === this.attackLen && this.cosT[0] === 1) return
    this.attackLen = a
    this.delay = a + DETECT_SPAN
    this.hold = this.delay + TP_MARGIN
    const w = (2 * Math.PI) / (a + 1)
    for (let j = 0; j <= a; j++) {
      this.cosT[j] = Math.cos(w * j)
      this.sinT[j] = Math.sin(w * j)
    }
    this.reset()
  }

  /** Recompute the sliding sums exactly from the ring (drift control). */
  private resyncSums(): void {
    const a = this.attackLen
    const h = this.hRing
    let b = 0
    let cr = 0
    let ci = 0
    for (let k = 0; k < a; k++) {
      let idx = this.hPos - 1 - k
      if (idx < 0) idx += a
      const v = h[idx]
      b += v
      cr += v * this.cosT[k + 1]
      ci += v * this.sinT[k + 1]
    }
    this.sumB = b
    this.sumCr = cr
    this.sumCi = ci
    this.resync = 0
  }

  processStereo(l: Float32Array, r: Float32Array, n: number): void {
    const ceil = this.ceiling
    const dl = this.dl
    const dr = this.dr
    const dMask = this.dMask
    const qIdx = this.qIdx
    const qVal = this.qVal
    const qMask = this.qMask
    const hRing = this.hRing
    const a = this.attackLen
    const norm = 1 / (a + 1)
    const c1 = this.cosT[1]
    const s1 = this.sinT[1]
    const hold = this.hold
    const delay = this.delay
    const rel = this.relCoef
    let g = this.gain
    let minGain = 1
    for (let i = 0; i < n; i++) {
      let xl = l[i]
      let xr = r[i]
      if (!(xl - xl === 0)) xl = 0
      if (!(xr - xr === 0)) xr = 0
      // 1. Delay line.
      const w = this.dPos
      dl[w] = xl
      dr[w] = xr
      this.dPos = (w + 1) & dMask
      const rd = (w - delay) & dMask
      // 2. 4x true-peak detection (stereo-linked).
      let pk = this.detL.push(xl)
      const pr = this.detR.push(xr)
      if (pr > pk) pk = pr
      // 3. Required gain through the running minimum over the hold span.
      const req = pk > ceil ? ceil / pk : 1
      const t = this.t
      let size = this.qSize
      let head = this.qHead
      while (size > 0 && qVal[(head + size - 1) & qMask] >= req) size--
      const tail = (head + size) & qMask
      qIdx[tail] = t
      qVal[tail] = req
      size++
      while (((t - qIdx[head]) & INDEX_MASK) >= hold) {
        head = (head + 1) & qMask
        size--
      }
      this.qHead = head
      this.qSize = size
      this.t = (t + 1) & INDEX_MASK
      const h = qVal[head]
      // 4. Raised-cosine (Hann) average of the held minimum over the attack length A:
      //    s = sum_k h[n-k] (1 - cos(w (k+1))) / (A+1), with O(1) sliding sums.
      const hp = this.hPos
      const hOld = hRing[hp]
      hRing[hp] = h
      this.hPos = hp + 1 === a ? 0 : hp + 1
      this.sumB += h - hOld
      const u = h + this.sumCr
      const ci = this.sumCi
      this.sumCr = c1 * u - s1 * ci - hOld
      this.sumCi = s1 * u + c1 * ci
      if (++this.resync >= RESYNC_PERIOD) this.resyncSums()
      let s = (this.sumB - this.sumCr) * norm
      if (s > 1) s = 1
      else if (s < 0) s = 0
      //    Instant attack along the Hann curve, exponential release toward it.
      g = s < g ? s : s + rel * (g - s)
      if (g < minGain) minGain = g
      // 5. Apply to the delayed sample, then the sample-peak safety clamp.
      let yl = dl[rd] * g
      let yr = dr[rd] * g
      if (yl > ceil) yl = ceil
      else if (yl < -ceil) yl = -ceil
      if (yr > ceil) yr = ceil
      else if (yr < -ceil) yr = -ceil
      l[i] = yl
      r[i] = yr
    }
    this.gain = g
    this.minGain = minGain
  }

  reset(): void {
    this.detL.reset()
    this.detR.reset()
    this.dl.fill(0)
    this.dr.fill(0)
    this.dPos = 0
    this.qHead = 0
    this.qSize = 0
    this.t = 0
    this.hRing.fill(1)
    this.hPos = 0
    this.resyncSums()
    this.gain = 1
    this.minGain = 1
  }
}

// ---------------------------------------------------------------------------------------
// Ducker
// ---------------------------------------------------------------------------------------

const DUCK_SLOTS = 8

/**
 * Sidechain-free ducker. Each trigger runs its own envelope, linear in dB: attack from 0 dB
 * to -depth, hold, release back to 0 dB. Overlapping triggers take the deepest (the minimum
 * over the active envelopes). next() returns the linear gain for one sample.
 */
export class Ducker {
  private readonly sr: number
  private readonly phase = new Int8Array(DUCK_SLOTS) // 0 idle, 1 attack, 2 hold, 3 release
  private readonly level = new Float64Array(DUCK_SLOTS) // dB <= 0
  private readonly depth = new Float64Array(DUCK_SLOTS) // dB < 0
  private readonly attackStep = new Float64Array(DUCK_SLOTS) // dB per sample (< 0)
  private readonly releaseStep = new Float64Array(DUCK_SLOTS) // dB per sample (> 0)
  private readonly holdLeft = new Int32Array(DUCK_SLOTS)
  private active = 0

  constructor(sr: number) {
    this.sr = safeSr(sr)
  }

  trigger(depthDb: number, attackMs: number, holdMs: number, releaseMs: number): void {
    const d = -clamp(Math.abs(finiteOr(depthDb, 0)), 0, 96)
    if (d > -1e-3) return
    // A free slot, else the shallowest one.
    let slot = 0
    let best = -Infinity
    for (let i = 0; i < DUCK_SLOTS; i++) {
      if (this.phase[i] === 0) {
        slot = i
        break
      }
      if (this.level[i] > best) {
        best = this.level[i]
        slot = i
      }
    }
    if (this.phase[slot] === 0) this.active++
    const ms = this.sr / 1000
    const aN = Math.max(1, Math.round(clamp(finiteOr(attackMs, 0), 0, 60000) * ms))
    const rN = Math.max(1, Math.round(clamp(finiteOr(releaseMs, 0), 0, 60000) * ms))
    this.phase[slot] = 1
    this.level[slot] = 0
    this.depth[slot] = d
    this.attackStep[slot] = d / aN
    this.releaseStep[slot] = -d / rN
    this.holdLeft[slot] = Math.round(clamp(finiteOr(holdMs, 0), 0, 600000) * ms)
  }

  next(): number {
    if (this.active === 0) return 1
    let minDb = 0
    for (let i = 0; i < DUCK_SLOTS; i++) {
      const ph = this.phase[i]
      if (ph === 0) continue
      let lv = this.level[i]
      if (ph === 1) {
        lv += this.attackStep[i]
        if (lv <= this.depth[i]) {
          lv = this.depth[i]
          this.phase[i] = 2
        }
      } else if (ph === 2) {
        if (this.holdLeft[i]-- <= 0) this.phase[i] = 3
      } else {
        lv += this.releaseStep[i]
        if (lv >= 0) {
          lv = 0
          this.phase[i] = 0
          this.active--
        }
      }
      this.level[i] = lv
      if (lv < minDb) minDb = lv
    }
    return minDb < 0 ? Math.exp(minDb * DB_TO_LN) : 1
  }

  reset(): void {
    this.phase.fill(0)
    this.level.fill(0)
    this.holdLeft.fill(0)
    this.active = 0
  }
}

// ---------------------------------------------------------------------------------------
// TransientShaper
// ---------------------------------------------------------------------------------------

const TS_FAST_ATTACK_MS = 1
const TS_FAST_RELEASE_MS = 30
const TS_SLOW_ATTACK_MS = 30
const TS_SLOW_RELEASE_MS = 300
const TS_MAX_DB = 24

/**
 * Transient shaper from the difference of a fast (1 ms attack) and a slow (30 ms attack)
 * peak follower. The normalized difference d = (fast - slow) / max(fast, slow) is positive
 * on onsets (scaled by attackDb) and negative on decaying tails, where the slow follower
 * releases later (scaled by sustainDb).
 */
export class TransientShaper {
  private readonly fA: number
  private readonly fR: number
  private readonly sA: number
  private readonly sR: number
  private fast = 0
  private slow = 0
  private attackDb = 0
  private sustainDb = 0

  constructor(sr: number) {
    const rate = safeSr(sr)
    this.fA = coefMs(TS_FAST_ATTACK_MS, rate)
    this.fR = coefMs(TS_FAST_RELEASE_MS, rate)
    this.sA = coefMs(TS_SLOW_ATTACK_MS, rate)
    this.sR = coefMs(TS_SLOW_RELEASE_MS, rate)
  }

  set(attackDb: number, sustainDb: number): void {
    this.attackDb = clamp(finiteOr(attackDb, 0), -TS_MAX_DB, TS_MAX_DB)
    this.sustainDb = clamp(finiteOr(sustainDb, 0), -TS_MAX_DB, TS_MAX_DB)
  }

  process(x: number): number {
    if (!(x - x === 0)) return 0
    const a = x < 0 ? -x : x
    let f = this.fast
    let s = this.slow
    f = a + (a > f ? this.fA : this.fR) * (f - a)
    s = a + (a > s ? this.sA : this.sR) * (s - a)
    if (f < 1e-12) f = 0
    if (s < 1e-12) s = 0
    this.fast = f
    this.slow = s
    if (this.attackDb === 0 && this.sustainDb === 0) return x
    const m = f > s ? f : s
    if (m < 1e-6) return x
    const d = (f - s) / m
    const gDb = d > 0 ? this.attackDb * d : -this.sustainDb * d
    return x * Math.exp(gDb * DB_TO_LN)
  }

  reset(): void {
    this.fast = 0
    this.slow = 0
  }
}
