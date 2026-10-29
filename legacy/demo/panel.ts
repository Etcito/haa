// Shared browser demo panel and action types.
import type { AudioControls } from '../src/contract.ts'

/** A demo operation; seconds is how long the levels operation measures it. */
export interface DemoAction {
  id: string
  label: string
  seconds: number
  /** Runs the action and optionally returns a function that stops it. */
  run(audio: AudioControls): (() => void) | void
}

/** A group of demo actions and controls mounted into the page. */
export interface DemoPanel {
  id: string
  title: string
  order: number
  actions(): DemoAction[]
  mount(root: HTMLElement, audio: AudioControls): void
}
