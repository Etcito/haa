// Proof output helpers (module proof-m1, node): the report triplet (report.json, report.md,
// index.html) of a proof that renders sound, and the sha256 of a file.
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { reportHtml, reportMarkdown } from '../../../src/analysis/report.ts';
import type { Measure } from '../../../src/analysis/measure.ts';
import type { Report, ReportCheck, ReportEntry } from '../../../src/analysis/report.ts';

/** A Measure with its keys in the fixed shared order. */
function orderedMeasure(m: Measure): Measure {
  return {
    frames: m.frames,
    seconds: m.seconds,
    nonFinite: m.nonFinite,
    samplePeakDb: m.samplePeakDb,
    truePeakDb: m.truePeakDb,
    dc: m.dc,
    rmsDb: m.rmsDb,
    minWindowRmsDb: m.minWindowRmsDb,
    lufs: m.lufs,
    shortTermMaxLufs: m.shortTermMaxLufs,
    bandsDb: [...m.bandsDb],
  };
}

/** A ReportEntry with its keys in the fixed shared order (optional keys only when set). */
function orderedEntry(e: ReportEntry): ReportEntry {
  const out: ReportEntry = { group: e.group, id: e.id, file: e.file, title: e.title, measure: orderedMeasure(e.measure) };
  if (e.targetLufs !== undefined) out.targetLufs = e.targetLufs;
  if (e.note !== undefined) out.note = e.note;
  return out;
}

/** A ReportCheck with its keys in the fixed shared order. */
function orderedCheck(c: ReportCheck): ReportCheck {
  return { label: c.label, measured: c.measured, threshold: c.threshold, pass: c.pass };
}

/**
 * Write the report triplet of `r` into `dir` (created when missing): report.json (2-space
 * indentation, fixed key order, trailing newline, no timestamp; non-finite numbers become null),
 * report.md (reportMarkdown) and index.html (reportHtml). Entry files are relative to `dir`.
 */
export function writeReport(dir: string, r: Report): void {
  mkdirSync(dir, { recursive: true });
  const json: Report = { proof: r.proof, entries: r.entries.map(orderedEntry), checks: r.checks.map(orderedCheck) };
  writeFileSync(join(dir, 'report.json'), `${JSON.stringify(json, null, 2)}\n`);
  writeFileSync(join(dir, 'report.md'), reportMarkdown(r));
  writeFileSync(join(dir, 'index.html'), reportHtml(r));
}

/** The lowercase hex sha256 of the file at `path`. */
export function sha256File(path: string): string {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}
