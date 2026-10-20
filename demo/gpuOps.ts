// Browser-GPU proof ops of the demo page: window.nmcGpu = { version: 1, ready, ops: { render } }.
// It is inert until the GPU client calls render; levels follow the shared contract 'Page ops' section.
import type { HaaAudio } from '../src/contract.ts'
import type { DemoAction } from './panel.ts'

/** What the page gives the ops: the audio (null before Start), the starter, the actions and the meter canvas. */
export interface GpuOpsHost {
  audio: HaaAudio | null
  start(): Promise<HaaAudio>
  actions: DemoAction[]
  canvas: HTMLCanvasElement
  /** Draws one meter frame on the canvas (main.ts); the snapshot calls it right before capture. */
  drawMeter?(): void
}

/** One render step, as listed in the contract. */
export type GpuStep =
  | { op: 'start' }
  | { op: 'levels'; actions?: string[] }
  | { op: 'trace'; action: string; everyMs: number }
  | { op: 'snapshot' }

/** PNG returned to the GPU client (the client takes no screenshot itself). */
export interface GpuFile { name: string; base64: string; type: string }

/** render result: `results` per the contract; `json` and `files` are the GPU client result envelope. */
export interface GpuRenderResult { results: unknown[]; json: { results: unknown[] }; files: GpuFile[] }

interface NmcGpu { version: 1; ready: Promise<void>; ops: { render(steps: unknown): Promise<GpuRenderResult> } }

declare global {
  interface Window {
    nmcGpu?: NmcGpu
  }
}

const SAMPLE_MS = 20
const BASELINE_MS = 200
const RMS_SECONDS = 0.05
const SILENCE_DB = -70
const SILENCE_MAX_MS = 3000
/** Bound on ctx.resume(): a context blocked by autoplay policy fails fast instead of hanging. */
const START_RESUME_MS = 2000
const DB_FLOOR = -200

let resolveReady: () => void = () => {}
const readyPromise = new Promise<void>((resolve) => { resolveReady = resolve })

/** Resolves window.nmcGpu.ready; main.ts calls it once every panel is mounted. Idempotent. */
export function markReady(): void {
  resolveReady()
}

/** Meter frame buffer for 'trace', preallocated once (the runtime analysers use fftSize 2048). */
const frame = new Float32Array(2048)
/** Measurement analyser length: ~680 ms at 48 kHz, so a stalled timer does not miss a peak. */
const MEASURE_FFT = 32768
/** Measurement buffer, preallocated once and shared by every measurement analyser. */
const long = new Float32Array(MEASURE_FFT)
/** One measurement analyser per audio, fed by audio.meters.mix (an analyser passes its input through). */
const measurers = new WeakMap<HaaAudio, AnalyserNode>()

function db(x: number): number {
  return x > 0 ? Math.max(DB_FLOOR, 20 * Math.log10(x)) : DB_FLOOR
}

const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms))

/** Reads one analyser frame and returns the 50 ms RMS dB and the frame peak dB. */
function read(an: AnalyserNode, sr: number): { rmsDb: number; peakDb: number } {
  const n = Math.min(frame.length, an.fftSize)
  const buf = n === frame.length ? frame : frame.subarray(0, n)
  an.getFloatTimeDomainData(buf)
  const w = Math.max(1, Math.min(n, Math.round(RMS_SECONDS * sr)))
  let sum = 0
  let peak = 0
  for (let i = 0; i < n; i++) {
    const a = Math.abs(buf[i]!)
    if (a > peak) peak = a
    if (i >= n - w) sum += buf[i]! * buf[i]!
  }
  return { rmsDb: db(Math.sqrt(sum / w)), peakDb: db(peak) }
}

type MeteredAudio = HaaAudio & { meters: NonNullable<HaaAudio['meters']> }

function needAudio(host: GpuOpsHost): MeteredAudio {
  const audio = host.audio
  if (!audio) throw new Error('[gpu] audio not started')
  if (!audio.meters) throw new Error('[gpu] no meters')
  return audio as MeteredAudio
}

function measurer(audio: MeteredAudio): AnalyserNode {
  let an = measurers.get(audio)
  if (!an) {
    an = audio.ctx.createAnalyser()
    an.fftSize = MEASURE_FFT
    audio.meters.mix.connect(an)
    measurers.set(audio, an)
  }
  return an
}

/** Reads the measurement buffer: max 50 ms RMS (10 ms hop) and peak over the last `count` samples, and the last-50 ms RMS. */
function readLong(an: AnalyserNode, sr: number, count: number): { maxRmsDb: number; peakDb: number; lastDb: number } {
  an.getFloatTimeDomainData(long)
  const n = long.length
  const w = Math.max(1, Math.min(n, Math.round(RMS_SECONDS * sr)))
  const hop = Math.max(1, Math.round(0.01 * sr))
  const from = n - Math.max(0, Math.min(n, count))
  let peak = 0
  for (let i = from; i < n; i++) peak = Math.max(peak, Math.abs(long[i]!))
  const rms = (s: number, len: number): number => {
    let sum = 0
    for (let i = s; i < s + len; i++) sum += long[i]! * long[i]!
    return len > 0 ? Math.sqrt(sum / len) : 0
  }
  let maxRms = n - from < w ? rms(from, n - from) : 0
  for (let e = n; e - w >= from; e -= hop) maxRms = Math.max(maxRms, rms(e - w, w))
  return { maxRmsDb: db(maxRms), peakDb: db(peak), lastDb: db(rms(n - w, w)) }
}

/** Polls every 20 ms for `ms`; looks only at the samples produced since it began (ctx clock, capped at the buffer). */
async function measure(audio: MeteredAudio, ms: number): Promise<{ maxRmsDb: number; peakDb: number }> {
  const an = measurer(audio)
  const ctx = audio.ctx
  const sr = ctx.sampleRate
  let maxRmsDb = DB_FLOOR
  let peakDb = DB_FLOOR
  const t0 = performance.now()
  const t0Ctx = ctx.currentTime
  for (;;) {
    const done = performance.now() - t0 >= ms
    const m = readLong(an, sr, Math.round((ctx.currentTime - t0Ctx) * sr))
    if (m.maxRmsDb > maxRmsDb) maxRmsDb = m.maxRmsDb
    if (m.peakDb > peakDb) peakDb = m.peakDb
    if (done) return { maxRmsDb, peakDb }
    await sleep(SAMPLE_MS)
  }
}

/** Waits until the last-50 ms mix RMS stayed below SILENCE_DB for BASELINE_MS in a row (at most SILENCE_MAX_MS). */
async function settle(audio: MeteredAudio): Promise<void> {
  const an = measurer(audio)
  const sr = audio.ctx.sampleRate
  const t0 = performance.now()
  let quietSince = -1
  for (;;) {
    const now = performance.now()
    if (readLong(an, sr, 0).lastDb >= SILENCE_DB) quietSince = -1
    else if (quietSince < 0) quietSince = now
    if ((quietSince >= 0 && now - quietSince >= BASELINE_MS) || now - t0 >= SILENCE_MAX_MS) return
    await sleep(SAMPLE_MS)
  }
}

async function stepStart(host: GpuOpsHost): Promise<unknown> {
  const audio = await host.start()
  const ctx = audio.ctx as AudioContext
  if (typeof ctx.resume === 'function') await Promise.race([ctx.resume().catch(() => {}), sleep(START_RESUME_MS)])
  const userAgent = navigator.userAgent
  if (ctx.state !== 'running') return { ctxState: ctx.state, sampleRate: ctx.sampleRate, userAgent }
  await audio.ready()
  return { ctxState: 'running', sampleRate: ctx.sampleRate, userAgent }
}

function pickActions(host: GpuOpsHost, ids: string[] | undefined): DemoAction[] {
  if (!ids) return host.actions
  return ids.map((id) => {
    const a = host.actions.find((x) => x.id === id)
    if (!a) throw new Error(`[gpu] unknown action ${id}`)
    return a
  })
}

async function stepLevels(host: GpuOpsHost, ids: string[] | undefined): Promise<unknown> {
  const audio = needAudio(host)
  const sr = audio.ctx.sampleRate
  const actions: { id: string; baselineDb: number; maxRmsDb: number; peakDb: number }[] = []
  for (const action of pickActions(host, ids)) {
    await settle(audio)
    const baselineDb = (await measure(audio, BASELINE_MS)).maxRmsDb
    const stop = action.run(audio)
    const m = await measure(audio, action.seconds * 1000)
    if (stop) stop()
    actions.push({ id: action.id, baselineDb, maxRmsDb: m.maxRmsDb, peakDb: m.peakDb })
  }
  return { ctxState: audio.ctx.state, sampleRate: sr, userAgent: navigator.userAgent, actions }
}

async function stepTrace(host: GpuOpsHost, id: string, everyMs: number): Promise<unknown> {
  const audio = needAudio(host)
  const sr = audio.ctx.sampleRate
  const [action] = pickActions(host, [id])
  const every = Math.max(1, everyMs)
  const points: { t: number; x: number; leftDb: number; rightDb: number }[] = []
  const stop = action!.run(audio)
  const t0 = performance.now()
  for (;;) {
    const t = (performance.now() - t0) / 1000
    if (t > action!.seconds) break
    points.push({ t, x: t / action!.seconds, leftDb: read(audio.meters.left, sr).rmsDb, rightDb: read(audio.meters.right, sr).rmsDb })
    await sleep(every)
  }
  if (stop) stop()
  return { ctxState: audio.ctx.state, sampleRate: sr, action: action!.id, points }
}

/** Draws one meter frame synchronously, then captures the canvas. */
async function stepSnapshot(host: GpuOpsHost, files: GpuFile[]): Promise<unknown> {
  const { canvas } = host
  host.drawMeter?.()
  const name = `snapshot-${files.length}.png`
  files.push({ name, base64: canvas.toDataURL('image/png').split(',')[1] ?? '', type: 'image/png' })
  return { canvas: { width: canvas.width, height: canvas.height }, file: name }
}

async function runStep(host: GpuOpsHost, step: GpuStep, files: GpuFile[]): Promise<unknown> {
  switch (step.op) {
    case 'start': return stepStart(host)
    case 'levels': return stepLevels(host, step.actions)
    case 'trace': return stepTrace(host, step.action, step.everyMs)
    case 'snapshot': return stepSnapshot(host, files)
    default: throw new Error(`[gpu] unknown op ${String((step as { op?: unknown }).op)}`)
  }
}

/**
 * Installs window.nmcGpu. render(steps) accepts the step array or `{ steps }` (the client input file),
 * runs the steps in order and returns `{ results }` (plus the client envelope `json`/`files`); a failing
 * step yields `{ error }` in results and never throws. `ready` resolves when main.ts calls markReady().
 */
export function installGpuOps(get: () => GpuOpsHost): void {
  const render = async (input: unknown): Promise<GpuRenderResult> => {
    const raw = Array.isArray(input) ? input : (input as { steps?: unknown } | null)?.steps
    const steps = Array.isArray(raw) ? (raw as GpuStep[]) : []
    const results: unknown[] = []
    const files: GpuFile[] = []
    for (const step of steps) {
      try {
        results.push(await runStep(get(), step, files))
      } catch (e) {
        results.push({ error: e instanceof Error ? e.message : String(e) })
      }
    }
    return { results, json: { results }, files }
  }
  window.nmcGpu = { version: 1, ready: readyPromise, ops: { render } }
}
