// Milestone 2 proof (module proof-m2, node): the explicit registries and the deterministic MP3
// bake. Cases `bake run1` and `bake run2` each bake the impact., ui., cue. and sting. families
// into out/m2/<run> in their own process; the parent then runs the 4 CHECKs (registry, manifest,
// byte determinism across the two processes, pcmHash against a fresh render) and writes
// out/m2/report.json and out/m2/report.md.
//
//   node scripts/proof/m2.ts [--only <pattern>]
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AUDIO_RATE, GAP_SAMPLES } from '../../src/contract.ts';
import type { BankManifest } from '../../src/contract.ts';
import { packStrip, pcmHash } from '../../src/bank/pack.ts';
import { renderFamilyVariants } from '../../src/bank/render.ts';
import { bake } from '../../src/bake/bake.ts';
import { BUILTIN_BEDS, BUILTIN_FAMILIES, BUILTIN_MIXES, BUILTIN_STYLES } from '../../src/content/index.ts';
import { registerBuiltins } from '../../src/registry/builtins.ts';
import {
  activeMix, getFamily, listBeds, listFamilies, listMixes, listStyles, registerFamily,
} from '../../src/registry/registry.ts';
import { sha256File } from './lib/out.ts';
import { check, checkBool, checks, proofMain, runCases, selected } from './lib/proof.ts';
import type { ProofCase } from './lib/proof.ts';

const SCRIPT = fileURLToPath(import.meta.url);
const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..', '..');
const OUT = join(ROOT, 'out', 'm2');
const SRC = join(ROOT, 'src');

/** Family id prefixes baked by the proof. */
const PREFIXES = ['impact.', 'ui.', 'cue.', 'sting.'] as const;
/** Expected family counts: 12 impact, 10 ui/cue, 3 sting. */
const EXPECTED = { impact: 12, uiCue: 10, sting: 3 } as const;
const EXPECTED_TOTAL = EXPECTED.impact + EXPECTED.uiCue + EXPECTED.sting;
const RUNS = ['run1', 'run2'] as const;
type RunId = (typeof RUNS)[number];

const LABELS = {
  // Built from parts so this file never holds the plain dotted token (test/repo/hygiene.test.ts).
  registry: `registry: no import.meta${'.'}glob and builtins registered`,
  manifest: 'bake: manifest valid',
  bytes: 'bake: deterministic bytes',
  pcm: 'bake: pcm hash matches a fresh render',
} as const;

/** The bundler glob token, written so this file never contains it in plain text. */
const GLOB_TOKEN = /import\.meta\.glob/;

/** One baked file of a run. */
interface FileInfo {
  file: string;
  bytes: number;
  sha256: string;
}

/** The @@RESULT of one bake case: its files (each .mp3 plus manifest.json, sorted). */
interface CaseResult {
  run: RunId;
  files: FileInfo[];
}

function byString(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Case `bake <run>`: a clean bake of the selected builtins into out/m2/<run>. */
function bakeCase(run: RunId): CaseResult {
  const dir = join(OUT, run);
  rmSync(dir, { recursive: true, force: true });
  registerBuiltins();
  const defs = listFamilies().filter((d) => PREFIXES.some((p) => d.id.startsWith(p)));
  bake(defs, { outDir: dir, log: (line) => console.log(`  [${run}] ${line}`) });
  const names = readdirSync(dir).filter((f) => f.endsWith('.mp3') || f === 'manifest.json').sort(byString);
  const files = names.map((file) => ({ file, bytes: statSync(join(dir, file)).size, sha256: sha256File(join(dir, file)) }));
  return { run, files };
}

const CASES: ProofCase<CaseResult>[] = RUNS.map((run) => ({ id: run, label: `bake ${run}`, run: () => bakeCase(run) }));

/** Every .ts file under dir, recursively. */
function tsFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const path = join(dir, e.name);
    return e.isDirectory() ? tsFiles(path) : e.name.endsWith('.ts') ? [path] : [];
  });
}

/** The header fields of the first MPEG frame of an MP3 file, or a reason it has none. */
function firstFrame(bytes: Uint8Array): { ok: boolean; detail: string } {
  for (let i = 0; i + 3 < bytes.length; i++) {
    if (bytes[i] !== 0xff || (bytes[i + 1] & 0xe0) !== 0xe0) continue;
    const version = (bytes[i + 1] >> 3) & 3; // 3 = MPEG-1
    const layer = (bytes[i + 1] >> 1) & 3; // 1 = Layer III
    const bitrate = bytes[i + 2] >> 4; // 9 = 128 kbps (MPEG-1 Layer III)
    const rate = (bytes[i + 2] >> 2) & 3; // 1 = 48 kHz (MPEG-1)
    const mode = bytes[i + 3] >> 6; // 3 = mono
    const ok = version === 3 && layer === 1 && bitrate === 9 && rate === 1 && mode === 3;
    return { ok, detail: `version ${version} layer ${layer} bitrate ${bitrate} rate ${rate} mode ${mode}` };
  }
  return { ok: false, detail: 'no frame sync' };
}

function readManifest(run: RunId): BankManifest {
  return JSON.parse(readFileSync(join(OUT, run, 'manifest.json'), 'utf8')) as BankManifest;
}

/** Check 1: the src scan, the registered counts and the duplicate rejection. */
function registryCheck(): void {
  const globFiles = tsFiles(SRC).filter((f) => GLOB_TOKEN.test(readFileSync(f, 'utf8'))).map((f) => relative(ROOT, f));
  registerBuiltins();
  const counts: [string, number, number][] = [
    ['families', listFamilies().length, BUILTIN_FAMILIES.length],
    ['beds', listBeds().length, BUILTIN_BEDS.length],
    ['mixes', listMixes().length, BUILTIN_MIXES.length],
    ['styles', listStyles().length, BUILTIN_STYLES.length],
  ];
  const countsOk = counts.every(([, n, want]) => n === want);
  let duplicate = 'no throw';
  try {
    registerFamily(BUILTIN_FAMILIES[0]);
  } catch (e) {
    duplicate = e instanceof Error ? e.message : String(e);
  }
  const dupOk = duplicate.startsWith('[haa] duplicate family');
  checkBool(LABELS.registry, globFiles.length === 0 && countsOk && dupOk,
    `${globFiles.length} src files with the glob token${globFiles.length ? ` [${globFiles[0]}]` : ''}; `
    + `${counts.map(([k, n, want]) => `${k} ${n}/${want}`).join(', ')}; duplicate: ${duplicate}`);
}

/** Check 2: the run1 manifest schema, files, frame headers, variant counts and family set. */
function manifestCheck(m: BankManifest): void {
  const problems: string[] = [];
  const keys = Object.keys(m).join(',');
  if (keys !== 'version,sampleRate,kbps,encoder,preset,families') problems.push(`keys ${keys}`);
  const want = { version: 1, sampleRate: 48000, kbps: 128, encoder: 'lamejs 1.2.1', preset: activeMix().id };
  for (const [k, v] of Object.entries(want)) {
    const got = (m as unknown as Record<string, unknown>)[k];
    if (got !== v) problems.push(`${k} ${String(got)} != ${String(v)}`);
  }
  const ids = m.families.map((f) => f.id);
  if (ids.join() !== [...ids].sort(byString).join()) problems.push('families not sorted');
  for (const f of m.families) {
    const path = join(OUT, 'run1', f.file);
    if (!existsSync(path)) {
      problems.push(`${f.id}: missing ${f.file}`);
      continue;
    }
    const frame = firstFrame(readFileSync(path));
    if (!frame.ok) problems.push(`${f.id}: frame ${frame.detail}`);
    const def = getFamily(f.id);
    if (!def) problems.push(`${f.id}: not registered`);
    else if (f.variants.length !== def.variants) problems.push(`${f.id}: ${f.variants.length} variants != ${def.variants}`);
  }
  const impact = ids.filter((id) => id.startsWith('impact.')).length;
  const uiCue = ids.filter((id) => id.startsWith('ui.') || id.startsWith('cue.')).length;
  const sting = ids.filter((id) => id.startsWith('sting.')).length;
  if (impact !== EXPECTED.impact || uiCue !== EXPECTED.uiCue || sting !== EXPECTED.sting) {
    problems.push(`${impact} impact, ${uiCue} ui/cue, ${sting} sting`);
  }
  if (problems.length > 0) {
    checkBool(LABELS.manifest, false, `${problems.length} problems [${problems.slice(0, 3).join('; ')}]`);
  } else {
    check(LABELS.manifest, ids.length, '==', EXPECTED_TOTAL, 'families',
      `${impact} impact, ${uiCue} ui/cue, ${sting} sting; MPEG-1 Layer III 48 kHz 128 kbps mono`);
  }
}

/** Check 3: the files whose sha256 (or presence) differs between the two runs. */
function bytesCheck(r1: CaseResult, r2: CaseResult): void {
  const a = new Map(r1.files.map((f) => [f.file, f.sha256]));
  const b = new Map(r2.files.map((f) => [f.file, f.sha256]));
  const all = [...new Set([...a.keys(), ...b.keys()])].sort(byString);
  const differing = all.filter((f) => a.get(f) === undefined || a.get(f) !== b.get(f));
  check(LABELS.bytes, differing.length, '==', 0, 'files', differing[0] ?? `${all.length} compared`);
}

/** Check 4: the run1 manifest pcmHash against a fresh render in this process. */
function pcmCheck(m: BankManifest): void {
  const preset = activeMix();
  const bad: string[] = [];
  for (const f of m.families) {
    const def = getFamily(f.id);
    const fresh = def ? pcmHash(packStrip(renderFamilyVariants(def, AUDIO_RATE, preset)).strip) : 'unregistered';
    if (fresh !== f.pcmHash) bad.push(`${f.id} ${f.pcmHash}/${fresh}`);
  }
  check(LABELS.pcm, bad.length, '==', 0, 'mismatches', bad[0] ?? `${m.families.length} families`);
}

/** One report row: a baked file of both runs, with its manifest data (null for manifest.json). */
interface ReportFile {
  file: string;
  run1: { bytes: number; sha256: string } | null;
  run2: { bytes: number; sha256: string } | null;
  pcmHash: string | null;
  variants: number | null;
  seconds: number | null;
}

/** The report rows from the case results and the first manifest available. */
function reportFiles(results: Map<string, CaseResult>): ReportFile[] {
  const byRun = new Map(RUNS.map((run) => [run, new Map((results.get(run)?.files ?? []).map((f) => [f.file, f]))]));
  const run = RUNS.find((r) => results.has(r));
  const entries = new Map((run ? readManifest(run).families : []).map((f) => [f.file, f]));
  const names = [...new Set(RUNS.flatMap((r) => [...(byRun.get(r) as Map<string, FileInfo>).keys()]))].sort(byString);
  return names.map((file) => {
    const side = (r: RunId): { bytes: number; sha256: string } | null => {
      const f = byRun.get(r)?.get(file);
      return f ? { bytes: f.bytes, sha256: f.sha256 } : null;
    };
    const e = entries.get(file);
    const samples = e ? e.variants.reduce((s, v) => s + v.length + GAP_SAMPLES, 0) : 0;
    return {
      file,
      run1: side('run1'),
      run2: side('run2'),
      pcmHash: e ? e.pcmHash : null,
      variants: e ? e.variants.length : null,
      seconds: e ? Number((samples / AUDIO_RATE).toFixed(3)) : null,
    };
  });
}

/** Write out/m2/report.json and out/m2/report.md. */
function writeM2Report(files: readonly ReportFile[]): void {
  mkdirSync(OUT, { recursive: true });
  const json = {
    proof: 'm2',
    files,
    checks: checks.map((c) => ({ label: c.label, measured: c.measured, threshold: c.threshold, pass: c.pass })),
  };
  writeFileSync(join(OUT, 'report.json'), `${JSON.stringify(json, null, 2)}\n`);
  const cell = (v: string | number | null | undefined): string => (v === null || v === undefined ? '-' : String(v));
  const lines = [
    '# m2: deterministic MP3 bake',
    '',
    '| file | bytes run1 | sha256 run1 | bytes run2 | sha256 run2 | pcmHash | variants | seconds |',
    '| --- | ---: | --- | ---: | --- | --- | ---: | ---: |',
    ...files.map((f) => `| ${f.file} | ${cell(f.run1?.bytes)} | ${cell(f.run1?.sha256)} | ${cell(f.run2?.bytes)} | `
      + `${cell(f.run2?.sha256)} | ${cell(f.pcmHash)} | ${cell(f.variants)} | ${cell(f.seconds)} |`),
    '',
    ...checks.map((c) => `- CHECK ${c.label}: ${c.measured} (${c.threshold}) ${c.pass ? 'PASS' : 'FAIL'}`),
    '',
  ];
  writeFileSync(join(OUT, 'report.md'), lines.join('\n'));
}

proofMain('m2', async (a) => {
  const results = await runCases(SCRIPT, CASES, a);
  const r1 = results.get('run1');
  const r2 = results.get('run2');
  try {
    if (selected(LABELS.registry, a)) registryCheck();
    else registerBuiltins();
    const m1 = r1 ? readManifest('run1') : null;
    if (m1 && selected(LABELS.manifest, a)) manifestCheck(m1);
    if (r1 && r2 && selected(LABELS.bytes, a)) bytesCheck(r1, r2);
    if (m1 && selected(LABELS.pcm, a)) pcmCheck(m1);
  } finally {
    writeM2Report(reportFiles(results));
    console.log(`report ${relative(ROOT, join(OUT, 'report.md'))}`);
  }
});
