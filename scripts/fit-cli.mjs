// Fits a HAA patch to a WAV sample with the wasm fitter (packages/haa/src/fit.ts over haa.wasm).
//
//   node scripts/fit-cli.mjs <target.wav> <templates-dir> --out best.json
//        [--budget N] [--seed N] [--top-k N] [--mutations N] [--max-dims N] [--step N]
//        [--report report.json] [--render fit.wav]
//
// Every *.json patch in <templates-dir> is a template. The fitted patch goes to --out, the full
// result (scores per template, trials, loss history, target/fit comparison) to --report (default
// <out without .json>.report.json). --render also renders the fitted patch with the offline
// renderer at the target's rate and length, writes it as a WAV and prints the target-vs-fit check.
// The WAV is read here (PCM 16/24/32-bit or float 32, channels averaged to mono); every
// computation runs in the wasm. Needs `node scripts/build-wasm.mjs` first and Node >= 22.18.
// Errors: one `[fit-cli] <reason>` line on stderr, exit 1.

import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { createFitter } from '../packages/haa/src/fit.ts';
import { encodeWav, renderPatch } from '../packages/haa/src/offline.ts';

const WASM = fileURLToPath(new URL('../packages/haa/pkg/haa.wasm', import.meta.url));
const USAGE = 'usage: node scripts/fit-cli.mjs <target.wav> <templates-dir> --out best.json [--budget N] [--seed N] [--top-k N] [--mutations N] [--max-dims N] [--step N] [--report file] [--render file.wav]';

/** Mono samples and rate of a RIFF/WAVE file. */
function readWav(path) {
  const bytes = readFileSync(path);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = (at) => String.fromCharCode(bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]);
  if (bytes.length < 12 || tag(0) !== 'RIFF' || tag(8) !== 'WAVE') throw new Error(`${path} is not a WAV file`);
  let format = 0, channels = 0, rate = 0, bits = 0, data = null;
  for (let at = 12; at + 8 <= bytes.length;) {
    const id = tag(at);
    const len = view.getUint32(at + 4, true);
    const body = at + 8;
    if (id === 'fmt ') {
      format = view.getUint16(body, true);
      channels = view.getUint16(body + 2, true);
      rate = view.getUint32(body + 4, true);
      bits = view.getUint16(body + 14, true);
      if (format === 0xfffe && len >= 26) format = view.getUint16(body + 24, true);
    } else if (id === 'data') {
      data = { at: body, len: Math.min(len, bytes.length - body) };
    }
    at = body + len + (len & 1);
  }
  if (!data || channels < 1 || rate <= 0) throw new Error(`${path}: no fmt or data chunk`);
  const width = bits / 8;
  const frames = Math.floor(data.len / (width * channels));
  const read = format === 3 && bits === 32
    ? (o) => view.getFloat32(o, true)
    : format === 1 && bits === 16 ? (o) => view.getInt16(o, true) / 32768
    : format === 1 && bits === 24 ? (o) => ((view.getUint8(o) | (view.getUint8(o + 1) << 8) | (view.getInt8(o + 2) << 16)) / 8388608)
    : format === 1 && bits === 32 ? (o) => view.getInt32(o, true) / 2147483648
    : null;
  if (!read) throw new Error(`${path}: unsupported WAV format ${format}/${bits}-bit`);
  const pcm = new Float32Array(frames);
  for (let f = 0; f < frames; f += 1) {
    let sum = 0;
    for (let c = 0; c < channels; c += 1) sum += read(data.at + (f * channels + c) * width);
    pcm[f] = sum / channels;
  }
  return { pcm, rate };
}

function parseArgs(argv) {
  const positional = [];
  const flags = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg.startsWith('--')) {
      const eq = arg.indexOf('=');
      const key = eq < 0 ? arg.slice(2) : arg.slice(2, eq);
      const value = eq < 0 ? argv[++i] : arg.slice(eq + 1);
      if (value === undefined) throw new Error(`--${key} needs a value`);
      flags[key] = value;
    } else positional.push(arg);
  }
  if (positional.length !== 2 || !flags.out) throw new Error(USAGE);
  const int = (key) => {
    if (flags[key] === undefined) return undefined;
    const n = Number(flags[key]);
    if (!Number.isInteger(n) || n < 0) throw new Error(`bad --${key}: ${flags[key]}`);
    return n;
  };
  return {
    target: positional[0],
    templatesDir: positional[1],
    out: flags.out,
    report: flags.report ?? `${flags.out.replace(/\.json$/i, '')}.report.json`,
    render: flags.render,
    step: int('step') ?? 25,
    options: Object.fromEntries(Object.entries({
      budget: int('budget'), seed: int('seed'), topK: int('top-k'), mutations: int('mutations'), maxDims: int('max-dims'),
    }).filter(([, v]) => v !== undefined)),
  };
}

function writeText(path, text) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text);
}

const round = (x, d = 3) => Math.round(x * 10 ** d) / 10 ** d;

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const { pcm, rate } = readWav(args.target);
  const names = readdirSync(args.templatesDir).filter((n) => n.endsWith('.json')).sort();
  if (names.length === 0) throw new Error(`no *.json template in ${args.templatesDir}`);
  const templates = names.map((n) => readFileSync(join(args.templatesDir, n), 'utf8'));
  const wasm = readFileSync(WASM);
  const fitter = await createFitter(wasm);

  const started = Date.now();
  let progress = fitter.start(pcm, rate, templates, args.options);
  console.error(`[fit-cli] ${names.length} templates, target ${round(pcm.length / rate)} s at ${rate} Hz, budget ${progress.budget}`);
  let lastStage = '';
  while (!progress.done) {
    progress = fitter.step(args.step);
    const line = `[fit-cli] ${progress.evals}/${progress.budget} ${progress.stage} best ${progress.best ?? '-'} (${progress.template ?? '-'}${progress.mutations?.length ? ' + ' + progress.mutations.join(', ') : ''})`;
    if (progress.stage !== lastStage || progress.evals % (args.step * 8) === 0 || progress.done) console.error(line);
    lastStage = progress.stage;
  }
  const result = fitter.result();
  if (!result?.best) throw new Error('no template rendered');
  writeText(args.out, fitter.patch());
  writeText(args.report, `${JSON.stringify(result, null, 2)}\n`);
  const seconds = (Date.now() - started) / 1000;
  console.error(`[fit-cli] done in ${round(seconds, 1)} s: ${result.best.templateId}${result.best.mutations.length ? ' + ' + result.best.mutations.join(', ') : ''}, loss ${result.best.score}, offset ${result.offset} s`);
  console.error(`[fit-cli] wrote ${args.out} and ${args.report}`);

  if (args.render) {
    // Independent check: render the written patch with the offline renderer and compare in wasm.
    const module = await WebAssembly.compile(wasm);
    const render = renderPatch(module, readFileSync(args.out), { sampleRate: rate, seconds: result.seconds });
    writeText(args.render, encodeWav(render));
    const mono = new Float32Array(render.left.length);
    for (let i = 0; i < mono.length; i += 1) mono[i] = 0.5 * (render.left[i] + render.right[i]);
    const target = pcm.subarray(Math.round(result.offset * rate));
    const check = fitter.compare(target, mono, rate);
    console.error(`[fit-cli] wrote ${args.render}`);
    console.error(`[fit-cli] check loss ${check.loss.total} | centroid target ${check.target.centroidHz} Hz fit ${check.candidate.centroidHz} Hz | rms target ${check.target.rmsDb} dB fit ${check.candidate.rmsDb} dB | envelope gap ${check.rmsEnvelopeGapDb} dB, centroid gap ${check.centroidEnvelopeGapHz} Hz`);
    console.error(`[fit-cli] rms envelope target ${check.target.rmsEnvelopeDb.join(' ')}`);
    console.error(`[fit-cli] rms envelope fit    ${check.candidate.rmsEnvelopeDb.join(' ')}`);
  }
  fitter.free();
}

main().catch((error) => {
  console.error(`[fit-cli] ${String(error instanceof Error ? error.message : error).replace(/\s*\r?\n\s*/g, ' ')}`);
  process.exit(1);
});
