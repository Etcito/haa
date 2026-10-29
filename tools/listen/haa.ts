// Listen page HAA side: the B side of the A/B switch. Loads the compiled wasm module once,
// creates the player over the page's AudioContext and exposes the controls main.ts drives.

import { createPlayer, loadHaaModule } from '../../packages/haa/src/index.ts';
import type { PatchIssue, TracePacket, VoiceInfo } from '../../packages/haa/src/index.ts';

/** One compiled module per page; a failed load is retried on the next attempt. */
let modulePromise: Promise<WebAssembly.Module> | null = null;

/** Fetch and compile the wasm module, sharing it between every HAA side of the page. */
function haaModule(): Promise<WebAssembly.Module> {
  if (!modulePromise) {
    modulePromise = loadHaaModule('/packages/haa/pkg/haa.wasm');
    modulePromise.catch(() => {
      modulePromise = null;
    });
  }
  return modulePromise;
}

/** The HAA side of the listening page, as main.ts sees it. */
export interface HaaSide {
  readonly info: VoiceInfo;
  readonly issues: PatchIssue[];
  readonly playable: boolean;
  set(name: string, value: number | string): void;
  trigger(name: string): void;
  restart(): void;
  onTrace(cb: (t: TracePacket) => void): () => void;
  dispose(): void;
}

/** Create the wasm player on `ctx`, routed through `out`, and delegate its lifecycle API. */
export async function createHaaSide(
  ctx: AudioContext,
  patchBytes: Uint8Array,
  out: GainNode,
): Promise<HaaSide> {
  const module = await haaModule();
  const player = await createPlayer(ctx, {
    module,
    processorUrl: '/packages/haa/dist/worklet.js',
    patch: patchBytes,
    trace: true,
    output: out,
  });
  return {
    get info() {
      return player.info;
    },
    get issues() {
      return player.issues;
    },
    get playable() {
      return player.playable;
    },
    set(name: string, value: number | string): void {
      player.set(name, value);
    },
    trigger(name: string): void {
      player.trigger(name);
    },
    restart(): void {
      player.restart();
    },
    onTrace(cb: (t: TracePacket) => void): () => void {
      return player.onTrace(cb);
    },
    dispose(): void {
      player.dispose();
    },
  };
}
