// Shared milestone proof helpers (module proof-m1, node): argument parsing and --only selection,
// independent cases run in child processes (--case <id>, one '@@RESULT <json>' line each, at most
// NMC_CPUS at once), CHECK lines recorded for the report, and the top-level proofMain runner.
import { spawn } from 'node:child_process';
import type { ChildProcess } from 'node:child_process';
import type { ReportCheck } from '../../../src/analysis/report.ts';

/** Parsed proof command line. */
export interface ProofArgs {
  /** --only <pattern>: case-insensitive; '/re/' is a RegExp, anything else an escaped substring. */
  only: RegExp | null;
  /** --full: run the long renders. */
  full: boolean;
  /** --case <id>: this process is a child running that one case. */
  caseId: string | null;
}

/** One independent unit of a proof, run in its own child process. */
export interface ProofCase<R> {
  id: string;
  /** What --only filters ('family <id>', 'bed <id>', 'render <name>'). */
  label: string;
  run(a: ProofArgs): R | Promise<R>;
}

/** A failed CHECK or case. proofMain prints its message and exits 1. */
export class ProofFail extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ProofFail';
  }
}

/** Every CHECK printed so far, in order (feeds Report.checks). */
export const checks: ReportCheck[] = [];

/** Prefix of the single result line a child prints on stdout. */
const RESULT_PREFIX = '@@RESULT ';

/** The --only pattern as a case-insensitive RegExp. */
function onlyPattern(p: string): RegExp {
  const re = /^\/(.+)\/([a-z]*)$/.exec(p);
  if (re) return new RegExp(re[1], re[2].includes('i') ? re[2] : `${re[2]}i`);
  return new RegExp(p.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'i');
}

/** Parse the proof arguments (`process.argv.slice(2)`). Unknown arguments throw. */
export function parseArgs(argv: string[]): ProofArgs {
  const a: ProofArgs = { only: null, full: false, caseId: null };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const eq = arg.indexOf('=');
    const name = arg.startsWith('--') && eq > 0 ? arg.slice(0, eq) : arg;
    const inline = name === arg ? null : arg.slice(eq + 1);
    const value = (): string => {
      if (inline !== null) return inline;
      const v = argv[++i];
      if (v === undefined) throw new Error(`missing value for ${name}`);
      return v;
    };
    if (name === '--only') a.only = onlyPattern(value());
    else if (name === '--case') a.caseId = value();
    else if (name === '--full' && inline === null) a.full = true;
    else throw new Error(`unknown argument ${arg}`);
  }
  return a;
}

/** True when `label` is selected by --only (always true without it). */
export function selected(label: string, a: ProofArgs): boolean {
  if (!a.only) return true;
  a.only.lastIndex = 0;
  return a.only.test(label);
}

/** The number of child processes allowed at once: NMC_CPUS, 1 when unset, at least 1. */
function maxProcesses(): number {
  return Math.max(1, Math.floor(Number(process.env.NMC_CPUS) || 1));
}

/** Run the case of a child process, print its result line and exit. Never returns. */
async function runChild<R>(cases: readonly ProofCase<R>[], a: ProofArgs): Promise<never> {
  const c = cases.find((x) => x.id === a.caseId);
  let code = 0;
  try {
    if (!c) throw new Error(`unknown case ${a.caseId}`);
    const result = await c.run(a);
    await new Promise<void>((done, fail) =>
      process.stdout.write(`${RESULT_PREFIX}${JSON.stringify(result)}\n`, (e) => (e ? fail(e) : done())));
  } catch (e) {
    console.error(`[case ${a.caseId}] ${e instanceof Error ? e.message : String(e)}`);
    code = 1;
  }
  process.exit(code);
}

/** Spawn one child for case `id` and resolve with its parsed result; reject naming the case. */
function runOne<R>(script: string, id: string, a: ProofArgs, live: Set<ChildProcess>): Promise<R> {
  return new Promise<R>((resolve, reject) => {
    const args = [script, '--case', id, ...(a.full ? ['--full'] : [])];
    const child = spawn(process.execPath, args, { stdio: ['ignore', 'pipe', 'inherit'] });
    live.add(child);
    const results: string[] = [];
    let pending = '';
    const line = (text: string): void => {
      if (text.startsWith(RESULT_PREFIX)) results.push(text.slice(RESULT_PREFIX.length));
      else if (text.length > 0) console.log(text);
    };
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', (chunk: string) => {
      const lines = (pending + chunk).split('\n');
      pending = lines.pop() ?? '';
      for (const text of lines) line(text);
    });
    child.on('error', (e) => {
      live.delete(child);
      reject(new ProofFail(`case ${id}: ${e.message}`));
    });
    child.on('close', (code, signal) => {
      live.delete(child);
      line(pending);
      if (code !== 0) return reject(new ProofFail(`case ${id}: exit ${code ?? signal}`));
      if (results.length !== 1) {
        return reject(new ProofFail(`case ${id}: ${results.length} @@RESULT lines (expected 1)`));
      }
      try {
        resolve(JSON.parse(results[0]) as R);
      } catch {
        reject(new ProofFail(`case ${id}: unreadable @@RESULT line`));
      }
    });
  });
}

/**
 * Run the selected cases. In a child (`--case <id>`) this runs that case, prints one
 * '@@RESULT <json>' line and exits the process. In the parent it spawns
 * `process.execPath <script> --case <id> [--full]` for every case whose label is selected,
 * at most NMC_CPUS at once (1 when unset), and returns the results by case id.
 * The first failing child (non-zero exit, missing or extra result line) stops the run with a
 * ProofFail naming the case; the children still running are killed.
 */
export async function runCases<R>(
  script: string,
  cases: readonly ProofCase<R>[],
  a: ProofArgs,
): Promise<Map<string, R>> {
  if (a.caseId !== null) return runChild(cases, a);
  const todo = cases.filter((c) => selected(c.label, a));
  const out = new Map<string, R>();
  const live = new Set<ChildProcess>();
  let next = 0;
  let failure: unknown = null;
  const worker = async (): Promise<void> => {
    while (failure === null && next < todo.length) {
      const c = todo[next++];
      try {
        out.set(c.id, await runOne<R>(script, c.id, a, live));
      } catch (e) {
        if (failure === null) failure = e;
        for (const child of live) child.kill();
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(maxProcesses(), todo.length) }, worker));
  if (failure !== null) throw failure;
  // Keep the case order of the proof, whatever order the children finished in.
  return new Map(todo.map((c) => [c.id, out.get(c.id) as R]));
}

/** A measured value at 4 significant digits, without trailing zeros. */
function formatNumber(v: number): string {
  return Number.isFinite(v) ? String(Number(v.toPrecision(4))) : String(v);
}

/** Print a CHECK line, record it in `checks`, and throw ProofFail on FAIL. */
function record(label: string, measured: string, threshold: string, pass: boolean, line: string): void {
  console.log(`CHECK ${label}: ${line} ${pass ? 'PASS' : 'FAIL'}`);
  checks.push({ label, measured, threshold, pass });
  if (!pass) throw new ProofFail(`FAIL ${label}: ${measured} (${threshold})`);
}

/**
 * Numeric check: prints `CHECK <label>: <measured> <unit> (<op> <threshold>) PASS|FAIL`.
 * `worst` (the id of the worst case of an aggregated check) prints in brackets after the unit.
 */
export function check(
  label: string,
  measured: number,
  op: '<=' | '<' | '>=' | '>' | '==',
  threshold: number,
  unit: string,
  worst?: string,
): void {
  const pass =
    op === '<=' ? measured <= threshold
    : op === '<' ? measured < threshold
    : op === '>=' ? measured >= threshold
    : op === '>' ? measured > threshold
    : measured === threshold;
  const value = `${formatNumber(measured)}${unit ? ` ${unit}` : ''}${worst ? ` [${worst}]` : ''}`;
  const limit = `${op} ${formatNumber(threshold)}`;
  record(label, value, `${limit}${unit ? ` ${unit}` : ''}`, pass, `${value} (${limit})`);
}

/** Boolean check: prints `CHECK <label>: <detail> PASS|FAIL`. */
export function checkBool(label: string, ok: boolean, detail: string): void {
  record(label, detail, 'ok', ok, detail);
}

/**
 * Top-level proof runner: parses `process.argv`, runs `body`, then prints `ALL PASS <name>` and
 * `wall <s> s`. A ProofFail or any error prints `[<name>] <reason>` and exits 1.
 * In a child process, runCases exits on its own before body returns.
 */
export function proofMain(name: string, body: (a: ProofArgs) => Promise<void>): void {
  const start = performance.now();
  const fail = (e: unknown): never => {
    console.error(`[${name}] ${e instanceof Error ? e.message : String(e)}`);
    process.exit(1);
  };
  let a: ProofArgs;
  try {
    a = parseArgs(process.argv.slice(2));
  } catch (e) {
    fail(e);
    return;
  }
  body(a).then(() => {
    console.log(`ALL PASS ${name}`);
    console.log(`wall ${((performance.now() - start) / 1000).toFixed(1)} s`);
  }, fail);
}
