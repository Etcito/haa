#!/usr/bin/env node
// Packs @etcito/haa (packages/haa) into ./release/etcito-haa-<version>.tgz for a GitHub Release.
// Local only: there is no GitHub workflow. It builds the wasm and the JS (npm run build), packs the
// workspace package, checks the required tar entries, then prints the `gh release create` command
// for the owner to run. It never runs that command itself.
// Usage: node scripts/release-pack.mjs [--tag v<version>]
// With --tag, the package version must equal the tag without its leading "v" (checked before any build).
import { execSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gunzipSync } from 'node:zlib';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = join(ROOT, 'release');

// Asset file name is fixed: <asset>-<version>.tgz (npm's own name for the scoped package).
const PACKAGE = {
  npmName: '@etcito/haa',
  asset: 'etcito-haa',
  manifest: join(ROOT, 'packages', 'haa', 'package.json'),
  required: [
    'package/package.json',
    'package/dist/index.js',
    'package/dist/index.d.ts',
    'package/dist/offline.js',
    'package/dist/offline.d.ts',
    'package/dist/node.js',
    'package/dist/node.d.ts',
    'package/dist/worklet.js',
    'package/dist/cli.js',
    'package/pkg/haa.wasm',
    'package/README.md',
    'package/LICENSE',
  ],
};

function fail(message) {
  console.error(`[release-pack] ${message}`);
  process.exit(1);
}

// Entry names of a .tgz (ustar): 512-byte headers, name in bytes 0-99, prefix in 345-499, size in octal at 124.
// Pure Node, so it does not depend on which tar binary is first in PATH.
function tarEntries(path) {
  const data = gunzipSync(readFileSync(path));
  const names = [];
  let offset = 0;
  while (offset + 512 <= data.length) {
    const header = data.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    const field = (start, end) => header.subarray(start, end).toString('latin1').split('\0')[0];
    const prefix = field(345, 500);
    const name = field(0, 100);
    names.push(prefix ? `${prefix}/${name}` : name);
    const size = parseInt(field(124, 136).trim() || '0', 8);
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  return names;
}

function run(command) {
  try {
    execSync(command, { cwd: ROOT, stdio: ['ignore', 'inherit', 'inherit'] });
  } catch {
    fail(`${command} failed`);
  }
}

const tagIndex = process.argv.indexOf('--tag');
const tag = tagIndex === -1 ? null : process.argv[tagIndex + 1];
if (tagIndex !== -1 && !tag) fail('--tag needs a value, e.g. --tag v0.2.0');

// Read and check the manifest before building anything.
let manifest;
try {
  manifest = JSON.parse(readFileSync(PACKAGE.manifest, 'utf8'));
} catch (error) {
  fail(`cannot read packages/haa/package.json: ${error.message}`);
}
if (manifest.name !== PACKAGE.npmName) fail(`packages/haa/package.json name is ${manifest.name}, expected ${PACKAGE.npmName}`);
if (tag !== null && tag !== `v${manifest.version}`) {
  fail(`tag ${tag} does not match ${manifest.name} ${manifest.version} (the tag must be v${manifest.version})`);
}
const file = `${PACKAGE.asset}-${manifest.version}.tgz`;

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

run('npm run build');
run(`npm pack -w ${PACKAGE.npmName} --pack-destination "${OUT}"`);
const tarball = join(OUT, file);
if (!existsSync(tarball)) fail(`npm pack did not produce ${file}`);
const entries = new Set(tarEntries(tarball));
for (const entry of PACKAGE.required) {
  if (!entries.has(entry)) fail(`${file} is missing ${entry}`);
}
console.log(`release-pack: ${file} ok`);
console.log(`gh release create v${manifest.version} release/${file}`);
