// Spatial demo panel (browser-only): beacon sweep and fly-by actions through createSpatial,
// with a top-down canvas view of the listener and the moving source.
import type { AudioControls, Listener, Vec3 } from '../../src/contract.ts'
import { createSpatial } from '../../src/spatial/model.ts'
import type { DemoAction, DemoPanel } from '../panel.ts'

/** Half-width of the top-down view, in meters. */
const VIEW_RANGE_M = 60
/** Side of the top-down view canvas, in pixels. */
const VIEW_SIZE_PX = 320
/** Position update period of the running action, in ms. */
const UPDATE_MS = 20

const model = createSpatial()
const listeners: readonly Listener[] = [{ pos: { x: 0, y: 0, z: 0 }, quat: { x: 0, y: 0, z: 0, w: 1 } }]
const sourcePos: Vec3 = { x: 0, y: 0, z: 0 }
let sourceActive = false
let view: HTMLCanvasElement | null = null

/** Draws the top-down view: listener at the centre facing -Z (up), the source as a dot, ±60 m scale. */
function drawView(): void {
  const g = view?.getContext('2d')
  if (!g) return
  const s = VIEW_SIZE_PX, c = s / 2, k = c / VIEW_RANGE_M
  g.fillStyle = '#111827'; g.fillRect(0, 0, s, s)
  g.strokeStyle = '#374151'; g.lineWidth = 1
  for (const m of [20, 40, 60]) { g.beginPath(); g.arc(c, c, m * k, 0, Math.PI * 2); g.stroke() }
  g.beginPath(); g.moveTo(0, c); g.lineTo(s, c); g.moveTo(c, 0); g.lineTo(c, s); g.stroke()
  g.fillStyle = '#9ca3af'; g.font = '11px sans-serif'
  g.fillText(`-${VIEW_RANGE_M} m`, 4, c - 4); g.fillText(`+${VIEW_RANGE_M} m`, s - 40, c - 4)
  g.fillText('-Z (forward)', c + 4, 12)
  g.fillStyle = '#e5e7eb'; g.beginPath(); g.arc(c, c, 5, 0, Math.PI * 2); g.fill()
  g.strokeStyle = '#e5e7eb'; g.lineWidth = 2; g.beginPath(); g.moveTo(c, c); g.lineTo(c, c - 14); g.stroke()
  if (sourceActive) {
    g.fillStyle = '#f59e0b'; g.beginPath()
    g.arc(c + sourcePos.x * k, c + sourcePos.z * k, 6, 0, Math.PI * 2); g.fill()
  }
}

/** Moves a beacon from `from` to `to` over `seconds`, updating pos every UPDATE_MS; vel is constant. */
function moveBeacon(audio: AudioControls, from: Vec3, to: Vec3, vel: Vec3, seconds: number): (() => void) | void {
  audio.setSpatial(model)
  audio.setListeners(listeners)
  const pos: Vec3 = { x: from.x, y: from.y, z: from.z }
  const bed = audio.startBed('beacon', { pos, vel })
  if (!bed) return
  sourcePos.x = pos.x; sourcePos.y = pos.y; sourcePos.z = pos.z
  sourceActive = true
  drawView()
  const started = performance.now()
  const interval = window.setInterval(() => {
    const f = Math.min(1, (performance.now() - started) / (seconds * 1000))
    pos.x = from.x + (to.x - from.x) * f
    pos.y = from.y + (to.y - from.y) * f
    pos.z = from.z + (to.z - from.z) * f
    bed.setPos(pos, vel)
    sourcePos.x = pos.x; sourcePos.y = pos.y; sourcePos.z = pos.z
    drawView()
  }, UPDATE_MS)
  return () => { window.clearInterval(interval); bed.stop(); sourceActive = false; drawView() }
}

/** Spatial panel: a beacon swept left to right in front of the listener, and a Doppler fly-by. */
export const PANEL: DemoPanel = {
  id: 'spatial',
  title: 'Spatial',
  order: 10,
  actions(): DemoAction[] {
    return [
      {
        id: 'spatial:sweep', label: 'Sweep L→R', seconds: 4,
        run: (audio: AudioControls) =>
          moveBeacon(audio, { x: -20, y: 0, z: -5 }, { x: 20, y: 0, z: -5 }, { x: 10, y: 0, z: 0 }, 4),
      },
      {
        id: 'spatial:flyby', label: 'Fly-by (Doppler)', seconds: 4,
        run: (audio: AudioControls) =>
          moveBeacon(audio, { x: 3, y: 0, z: -60 }, { x: 3, y: 0, z: 60 }, { x: 0, y: 0, z: 30 }, 4),
      },
    ]
  },
  mount(root, audio): void {
    audio.setSpatial(model)
    for (const action of this.actions()) {
      const button = document.createElement('button')
      button.type = 'button'
      button.textContent = action.label
      button.addEventListener('click', () => { action.run(audio) })
      root.append(button)
    }
    const canvas = document.createElement('canvas')
    canvas.width = VIEW_SIZE_PX; canvas.height = VIEW_SIZE_PX
    canvas.style.display = 'block'
    root.append(canvas)
    view = canvas
    drawView()
  },
}
