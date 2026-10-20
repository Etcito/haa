// Core built-in families, wind bed and music demo controls.
import type { AudioControls } from '../../src/contract.ts'
import { listFamilies, listStyles } from '../../src/registry/registry.ts'
import { registerBuiltins } from '../../src/registry/builtins.ts'
import type { DemoAction, DemoPanel } from '../panel.ts'

/** Main panel for trying built-in sounds and live controls. */
export const PANEL: DemoPanel = {
  id: 'core',
  title: 'Core',
  order: 0,
  actions(): DemoAction[] {
    registerBuiltins()
    const families = listFamilies()
    return [
      ...families.map((def) => ({
        id: `family:${def.id}`,
        label: def.id,
        seconds: def.maxSeconds + 0.3,
        run: (audio: AudioControls) => { audio.play(def.id) },
      })),
      {
        id: 'bed:wind', label: 'Wind sweep', seconds: 3,
        run: (audio: AudioControls) => {
          const bed = audio.startBed('wind')
          if (!bed) return
          const started = performance.now()
          const interval = window.setInterval(() => {
            const s = Math.min(40, ((performance.now() - started) / 3000) * 40)
            bed.setParam('speed', s)
            bed.setParam('speed01', s / 50)
          }, 50)
          return () => { window.clearInterval(interval); bed.stop() }
        },
      },
      ...(['upbeat', 'calm'] as const).map((style) => ({
        id: `music:${style}`,
        label: `Music ${style}`,
        seconds: 4,
        run: (audio: AudioControls) => {
          audio.music.play(style, { intensity: 0.7 })
          return () => audio.music.stop(0.2)
        },
      })),
    ]
  },
  mount(root, audio): void {
    const actions = this.actions()
    for (const action of actions) {
      const button = document.createElement('button')
      button.type = 'button'
      button.textContent = action.label
      button.addEventListener('click', () => { action.run(audio) })
      root.append(button)
    }

    const windLabel = document.createElement('label')
    windLabel.textContent = 'Wind speed '
    const wind = document.createElement('input')
    wind.type = 'range'; wind.min = '0'; wind.max = '50'; wind.step = '0.1'; wind.value = '0'
    let windBed: ReturnType<AudioControls['startBed']> = null
    wind.addEventListener('input', () => {
      windBed ??= audio.startBed('wind')
      const speed = Number(wind.value)
      windBed?.setParam('speed', speed)
      windBed?.setParam('speed01', speed / 50)
    })
    windLabel.append(wind); root.append(windLabel)

    const styleLabel = document.createElement('label')
    styleLabel.textContent = 'Style '
    const style = document.createElement('select')
    for (const item of listStyles()) {
      const option = document.createElement('option')
      option.value = item.id; option.textContent = item.id; style.append(option)
    }
    styleLabel.append(style); root.append(styleLabel)
    const crossfade = document.createElement('button')
    crossfade.type = 'button'; crossfade.textContent = 'Crossfade'
    crossfade.addEventListener('click', () => {
      if (audio.music.style) audio.music.crossfade(style.value, 2)
      else audio.music.play(style.value, { intensity: 0.7 })
    })
    root.append(crossfade)

    const volumeLabel = document.createElement('label')
    volumeLabel.textContent = 'Master volume '
    const volume = document.createElement('input')
    volume.type = 'range'; volume.min = '0'; volume.max = '1'; volume.step = '0.01'; volume.value = '0.8'
    volume.addEventListener('input', () => audio.setVolumes({ master: Number(volume.value) }))
    volumeLabel.append(volume); root.append(volumeLabel)
  },
}
