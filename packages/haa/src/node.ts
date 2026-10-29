// Node file helpers for loading the wasm engine and rendering patches to WAV.
import { mkdirSync, writeFileSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { encodeWav, renderPatch } from './offline.ts';
import type { RenderOptions, StereoRender } from './offline.ts';

export * from './offline.ts';

/** Compile the packaged wasm engine, or a wasm file at the supplied path. */
export async function loadModule(
  path: string = fileURLToPath(new URL('../pkg/haa.wasm', import.meta.url)),
): Promise<WebAssembly.Module> {
  return WebAssembly.compile(await readFile(path));
}

/** Read a patch file and render it with the packaged wasm engine. */
export async function renderPatchFile(
  patchPath: string,
  o?: RenderOptions,
): Promise<StereoRender> {
  const bytes = await readFile(patchPath);
  const module = await loadModule();
  return renderPatch(module, bytes, o);
}

/** Write a stereo PCM WAV, creating its parent directories as needed. */
export function writeWav(path: string, r: StereoRender): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, encodeWav(r));
}
