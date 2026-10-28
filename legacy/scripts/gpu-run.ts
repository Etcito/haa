// GPU run (node-only): publishes out/demo/ through the client GPU Chrome, runs the page's 'render' op
// and writes the committed LevelsResult or TraceResult. Never substitutes an offline or partial result.
//
// Usage: node scripts/gpu-run.ts --input <proofs/mN/input.json> --out <proofs/mN/result.json> --captures <out/proofs/mN>
//
// Client protocol (read from the client script $NMC_GPU_CLIENT; docs/gpu-client.md is not readable here):
// - `publish <dir>` prints one JSON line on stdout: `{ "page_ref": "<REF>", ... }`.
// - `<op> <input.json> --page-ref <REF> -o <dir>` posts `{ op, input, page_ref }`, writes every returned file
//   `{ name, base64 }` into <dir>, then `<dir>/result.json` = `{ "job_id", "client_id", "result": <op json> }`,
//   and prints `{ "job_id", "client_id", "output" }` on stdout.
// - The demo op returns json `{ results: unknown[] }`, one entry per step of input `{ steps: [...] }`.
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { parseArgs } from 'node:util';

const DEMO_DIR = 'out/demo/';
const BUILD_JSON = 'out/demo/build.json';
/** Wall limit of one client call (STEP DISCIPLINE: 60 s unless NMC_GPU_TIMEOUT_S raises it). */
const CLIENT_TIMEOUT_MS = (Number(process.env.NMC_GPU_TIMEOUT_S) || 60) * 1000;

type Obj = Record<string, unknown>;
type Step = { op: string; action?: string };

function fail(code: number, msg: string): never {
  console.error(`[gpu] ${msg}`);
  process.exit(code);
}

/** Removes every NMC_GPU_* value (token, url) from a client diagnostic line. */
function redact(s: string): string {
  let out = s;
  for (const [k, v] of Object.entries(process.env)) if (k.startsWith('NMC_GPU_') && v && v.length >= 4) out = out.split(v).join('***');
  return out;
}

/** Runs the client without a shell; resolves its parsed last stdout JSON line, never echoes stdout. */
function client(path: string, args: string[]): Promise<Obj> {
  return new Promise((resolve, reject) => {
    execFile(process.execPath, [path, ...args], { timeout: CLIENT_TIMEOUT_MS, maxBuffer: 16 << 20 }, (err, stdout, stderr) => {
      if (err) {
        const last = String(stderr).trim().split('\n').pop() ?? '';
        const why = err.killed ? `timed out after ${CLIENT_TIMEOUT_MS / 1000} s` : `exit ${err.code ?? '?'}`;
        reject(new Error(`client ${args[0]} failed (${why}): ${redact(last).slice(0, 300)}`));
        return;
      }
      const line = String(stdout).trim().split('\n').reverse().find((l) => l.trim().startsWith('{'));
      try {
        resolve(JSON.parse(line ?? '') as Obj);
      } catch {
        reject(new Error(`client ${args[0]} printed no JSON result`));
      }
    });
  });
}

const sha256 = (file: string): string => createHash('sha256').update(readFileSync(file)).digest('hex');
const readJson = (file: string): Obj => JSON.parse(readFileSync(file, 'utf8')) as Obj;
const num = (v: unknown): number => (typeof v === 'number' ? v : Number.NaN);
const str = (v: unknown): string => (typeof v === 'string' ? v : '');

async function main(): Promise<void> {
  const clientPath = process.env.NMC_GPU_CLIENT;
  if (!clientPath || !existsSync(clientPath)) {
    console.log('[gpu] no client: waiting');
    process.exit(2);
  }
  const { values } = parseArgs({ options: { input: { type: 'string' }, out: { type: 'string' }, captures: { type: 'string' } } });
  const { input, out, captures } = values;
  if (!input || !out || !captures) fail(1, 'usage: --input <input.json> --out <result.json> --captures <dir>');
  if (!existsSync(BUILD_JSON)) fail(1, `missing ${BUILD_JSON}: run node scripts/build-demo.ts first`);
  const steps = (readJson(input).steps ?? []) as Step[];
  if (!Array.isArray(steps) || steps.length === 0) fail(1, `${input} has no steps`);
  const kind = steps.find((s) => s.op === 'levels' || s.op === 'trace')?.op;
  if (!kind) fail(1, `${input} has no levels or trace step`);

  const pub = await client(clientPath, ['publish', DEMO_DIR]);
  const ref = str(pub.page_ref);
  if (!ref) fail(1, 'publish returned no page_ref');
  console.log(`[gpu] published page_ref ${ref}`);

  mkdirSync(captures, { recursive: true });
  for (const f of readdirSync(captures)) if (f.endsWith('.png') || f === 'result.json') rmSync(join(captures, f));
  await client(clientPath, ['render', input, '--page-ref', ref, '-o', captures]);
  const op = readJson(join(captures, 'result.json')).result as Obj | undefined;
  const results = (op?.results ?? []) as Obj[];
  if (!Array.isArray(results) || results.length !== steps.length) fail(1, `op returned ${Array.isArray(results) ? results.length : 0} results for ${steps.length} steps`);

  const start = results[steps.findIndex((s) => s.op === 'start')] ?? {};
  const ctxState = str(start.ctxState);
  if (ctxState !== 'running') fail(3, `AudioContext not running: ${ctxState || 'unknown'}`);
  const res = results[steps.findIndex((s) => s.op === kind)] ?? {};

  const b = readJson(BUILD_JSON);
  const build = { mainSha256: str(b.mainSha256), workletSha256: str(b.workletSha256), manifestSha256: str(b.manifestSha256) };
  const shots = readdirSync(captures).filter((f) => f.endsWith('.png')).sort();
  const capturesOut = shots.map((f) => ({ file: basename(f), sha256: sha256(join(captures, f)) }));
  const head = { version: 1 as const, build, ctxState, sampleRate: num(start.sampleRate) };
  const result =
    kind === 'levels'
      ? {
          ...head,
          userAgent: str(start.userAgent),
          actions: ((res.actions ?? []) as Obj[]).map((a) => ({ id: str(a.id), baselineDb: num(a.baselineDb), maxRmsDb: num(a.maxRmsDb), peakDb: num(a.peakDb) })),
          captures: capturesOut,
        }
      : {
          ...head,
          action: str(res.action) || str(steps.find((s) => s.op === 'trace')?.action),
          points: ((res.points ?? []) as Obj[]).map((p) => ({ t: num(p.t), x: num(p.x), leftDb: num(p.leftDb), rightDb: num(p.rightDb) })),
          captures: capturesOut,
        };
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, `${JSON.stringify(result, null, 2)}\n`);
  console.log(`[gpu] wrote ${out} (${kind}, captures: ${shots.join(', ') || 'none'})`);
}

main().catch((e: unknown) => fail(1, e instanceof Error ? e.message : String(e)));
