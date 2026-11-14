// Builds the Rust wasm engine and copies it into the npm package.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('../', import.meta.url));

try {
  execFileSync('cargo', ['build', '--release', '--target', 'wasm32-unknown-unknown', '-p', 'haa-wasm'], { stdio: 'inherit', cwd: ROOT });
  mkdirSync(new URL('../packages/haa/pkg/', import.meta.url), { recursive: true });
  copyFileSync(
    new URL('../target/wasm32-unknown-unknown/release/haa_wasm.wasm', import.meta.url),
    new URL('../packages/haa/pkg/haa.wasm', import.meta.url),
  );
  // The mixer of the render workers (crates/haa-mix): its own small wasm.
  execFileSync('cargo', ['build', '--release', '--target', 'wasm32-unknown-unknown', '-p', 'haa-mix'], { stdio: 'inherit', cwd: ROOT });
  copyFileSync(
    new URL('../target/wasm32-unknown-unknown/release/haa_mix.wasm', import.meta.url),
    new URL('../packages/haa/pkg/haa-mix.wasm', import.meta.url),
  );
  // The baked acoustic field (crates/haa-field, with haa-trace's geometry): its own small wasm, for a worker and the main thread.
  execFileSync('cargo', ['build', '--release', '--target', 'wasm32-unknown-unknown', '-p', 'haa-field'], { stdio: 'inherit', cwd: ROOT });
  copyFileSync(
    new URL('../target/wasm32-unknown-unknown/release/haa_field.wasm', import.meta.url),
    new URL('../packages/haa/pkg/haa-field.wasm', import.meta.url),
  );
  // The acoustic tracer (crates/haa-trace): its own small wasm, for the main thread or a worker.
  execFileSync('cargo', ['build', '--release', '--target', 'wasm32-unknown-unknown', '-p', 'haa-trace'], { stdio: 'inherit', cwd: ROOT });
  copyFileSync(
    new URL('../target/wasm32-unknown-unknown/release/haa_trace.wasm', import.meta.url),
    new URL('../packages/haa/pkg/haa-trace.wasm', import.meta.url),
  );
} catch (error) {
  console.error(`[build-wasm] ${String(error instanceof Error ? error.message : error).replace(/\s*\r?\n\s*/g, ' ')}`);
  process.exit(1);
}
