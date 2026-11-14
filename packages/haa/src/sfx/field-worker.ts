// Worker entry (`@etcito/haa/field-worker`): bakes the acoustic field off the main thread, nearest
// to the listener first, in slices, and posts the baked cell records (docs/physics-sound.md,
// "Acoustic field"). The main thread ingests them into its own copy for the lookups.
import type { FieldGrid, SfxGeometry } from './field-wasm.ts';
import { FieldWasm } from './field-wasm.ts';
import type { Vec3 } from './acoustics.ts';

export type FieldWorkerIn =
  | { t: 'init'; module: WebAssembly.Module; geometry: SfxGeometry; grid: FieldGrid; focus: Vec3; slice?: number }
  | { t: 'focus'; p: Vec3 }
  | { t: 'geometry'; geometry: SfxGeometry; lo: Vec3; hi: Vec3 };

export type FieldWorkerOut =
  | { t: 'cells'; records: Float32Array; left: number; ms: number; casts: number; baked: number; bytes: number }
  | { t: 'dirty'; lo: Vec3; hi: Vec3; count: number }
  | { t: 'error'; message: string };

const scope = globalThis as unknown as { onmessage: ((e: MessageEvent<FieldWorkerIn>) => void) | null; postMessage(m: FieldWorkerOut, transfer?: Transferable[]): void };
let field: FieldWasm | null = null;
let focus: Vec3 = { x: 0, y: 0, z: 0 };
let slice = 200;
let running = false;

function step(): void {
  if (!field) return;
  const t0 = performance.now();
  const c0 = field.stats().casts;
  const left = field.bake(slice, focus);
  const records = field.take();
  const st = field.stats();
  scope.postMessage({ t: 'cells', records, left, ms: performance.now() - t0, casts: st.casts - c0, baked: st.baked, bytes: st.bytes }, [records.buffer]);
  if (left > 0) setTimeout(step, 0);
  else running = false;
}

function kick(): void {
  if (running) return;
  running = true;
  setTimeout(step, 0);
}

scope.onmessage = (e) => {
  const m = e.data;
  try {
    if (m.t === 'init') {
      field = new FieldWasm(m.module);
      field.setScene(m.geometry);
      field.setGrid(m.grid);
      focus = m.focus;
      slice = m.slice ?? slice;
      kick();
    } else if (m.t === 'focus') {
      focus = m.p;
      kick();
    } else if (m.t === 'geometry' && field) {
      field.setScene(m.geometry);
      const count = field.dirty(m.lo, m.hi);
      scope.postMessage({ t: 'dirty', lo: m.lo, hi: m.hi, count });
      kick();
    }
  } catch (error) {
    scope.postMessage({ t: 'error', message: error instanceof Error ? error.message : String(error) });
  }
};
