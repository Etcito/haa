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
} catch (error) {
  console.error(`[build-wasm] ${String(error instanceof Error ? error.message : error).replace(/\s*\r?\n\s*/g, ' ')}`);
  process.exit(1);
}
