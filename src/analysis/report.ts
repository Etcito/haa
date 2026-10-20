// Report builders of the proofs (shared 'Measurement and reports'): the ReportEntry, ReportCheck
// and Report types, reportMarkdown (report.md) and reportHtml (index.html).
// DSP-safe, plain string building. Numbers use a fixed precision (2 decimals for dB/LUFS/seconds,
// 5 decimals for DC), so the same report always gives the same text.

import { OCTAVE_BANDS } from './measure.ts'
import type { Measure } from './measure.ts'

/** One measured file of a report. file is relative to the report folder. */
export interface ReportEntry {
  group: string
  id: string
  file: string
  title: string
  measure: Measure
  targetLufs?: number
  note?: string
}

/** One CHECK line of a proof. */
export interface ReportCheck {
  label: string
  measured: string
  threshold: string
  pass: boolean
}

/** The report of one proof run. */
export interface Report {
  proof: string
  entries: ReportEntry[]
  checks: ReportCheck[]
}

/** A number at a fixed precision; non-finite values print as 'inf', '-inf' or 'nan'. */
function fixed(v: number, digits: number): string {
  if (Number.isNaN(v)) return 'nan'
  if (!Number.isFinite(v)) return v > 0 ? 'inf' : '-inf'
  const s = v.toFixed(digits)
  // Avoid '-0.00': a value that rounds to zero prints unsigned.
  return Number(s) === 0 ? (0).toFixed(digits) : s
}

/** dB, LUFS and seconds: 2 decimals. */
const db = (v: number): string => fixed(v, 2)
/** DC: 1e-5 resolution. */
const dc = (v: number): string => fixed(v, 5)

/** A band center label (31.5, 63, ..., 1k, 16k). */
function bandLabel(hz: number): string {
  return hz >= 1000 ? `${hz / 1000}k` : `${hz}`
}

/** Entries grouped by group, groups in their first-appearance order, entries in report order. */
function groups(entries: readonly ReportEntry[]): [string, ReportEntry[]][] {
  const map = new Map<string, ReportEntry[]>()
  for (const e of entries) {
    const list = map.get(e.group)
    if (list) list.push(e)
    else map.set(e.group, [e])
  }
  return [...map.entries()]
}

/** Column headers of the measure tables (id, then the measures). */
const MEASURE_HEADERS: readonly string[] = [
  'seconds', 'LUFS', 'target', 'sample peak dBFS', 'true peak dBTP', 'DC', 'min window RMS dBFS',
  ...OCTAVE_BANDS.map((hz) => `${bandLabel(hz)} dB`),
]

/** The formatted measure cells of an entry, in MEASURE_HEADERS order. */
function measureCells(e: ReportEntry): string[] {
  const m = e.measure
  const bands = OCTAVE_BANDS.map((_, i) => db(m.bandsDb[i] ?? Number.NaN))
  return [
    db(m.seconds), db(m.lufs), e.targetLufs === undefined ? '-' : db(e.targetLufs),
    db(m.samplePeakDb), db(m.truePeakDb), dc(m.dc), db(m.minWindowRmsDb), ...bands,
  ]
}

/** A markdown table cell: pipes escaped, line breaks flattened. */
function mdCell(s: string): string {
  return s.replace(/\\/g, '\\\\').replace(/\|/g, '\\|').replace(/\r?\n/g, ' ')
}

/** A markdown table row. */
function mdRow(cells: readonly string[]): string {
  return `| ${cells.map(mdCell).join(' | ')} |`
}

/** The markdown separator row of n columns. */
function mdRule(n: number): string {
  return `|${' --- |'.repeat(n)}`
}

/** The CHECK rows (label, measured, threshold, result). */
function checkCells(c: ReportCheck): string[] {
  return [c.label, c.measured, c.threshold, c.pass ? 'PASS' : 'FAIL']
}

const CHECK_HEADERS: readonly string[] = ['check', 'measured', 'threshold', 'result']

/**
 * report.md: one table per entry group (id, seconds, LUFS, target, sample peak, true peak, DC,
 * min window RMS, the 10 band levels), the entry notes under their table, then the CHECK table.
 */
export function reportMarkdown(r: Report): string {
  const out: string[] = [`# ${mdCell(r.proof)} report`, '']
  for (const [group, list] of groups(r.entries)) {
    out.push(`## ${mdCell(group)}`, '')
    const headers = ['id', ...MEASURE_HEADERS]
    out.push(mdRow(headers), mdRule(headers.length))
    for (const e of list) out.push(mdRow([e.id, ...measureCells(e)]))
    const notes = list.filter((e) => e.note !== undefined && e.note !== '')
    if (notes.length > 0) {
      out.push('')
      for (const e of notes) out.push(`- ${mdCell(e.id)}: ${mdCell(e.note ?? '')}`)
    }
    out.push('')
  }
  out.push('## CHECK', '')
  out.push(mdRow(CHECK_HEADERS), mdRule(CHECK_HEADERS.length))
  for (const c of r.checks) out.push(mdRow(checkCells(c)))
  out.push('')
  return out.join('\n')
}

/** Text and attribute values HTML-escaped (&, <, >, " and '). */
function esc(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;').replace(/'/g, '&#39;')
}

/** An HTML table row of escaped cells; th for the header row. */
function htmlRow(cells: readonly string[], tag: 'td' | 'th', cls = ''): string {
  const attr = cls === '' ? '' : ` class='${cls}'`
  return `<tr${attr}>${cells.map((c) => `<${tag}>${esc(c)}</${tag}>`).join('')}</tr>`
}

/** Inline style of index.html. */
const HTML_STYLE = [
  'body{font-family:system-ui,sans-serif;margin:2rem;color:#1d2330;background:#f7f8fa}',
  'h1{font-size:1.5rem}h2{font-size:1.2rem;margin-top:2rem}',
  'section.entry{background:#fff;border:1px solid #dde1e8;border-radius:6px;padding:0.75rem 1rem;margin:0.75rem 0}',
  'section.entry h3{font-size:1rem;margin:0 0 0.5rem}',
  'audio{width:100%;max-width:32rem;display:block;margin:0.25rem 0 0.5rem}',
  'table{border-collapse:collapse;font-size:0.8rem;font-variant-numeric:tabular-nums}',
  'th,td{border:1px solid #dde1e8;padding:0.2rem 0.45rem;text-align:right}',
  'th{background:#eef1f5}td:first-child,th:first-child{text-align:left}',
  'p.note{font-size:0.85rem;color:#4a5263;margin:0.4rem 0 0}',
  'tr.pass td:last-child{color:#17803d;font-weight:600}tr.fail td:last-child{color:#c0262d;font-weight:600}',
].join('\n')

/** One entry: title, audio player, its measures table and its note. */
function htmlEntry(e: ReportEntry): string {
  return [
    `<section class='entry'>`,
    `<h3>${esc(e.title)} <small>(${esc(e.id)})</small></h3>`,
    `<audio controls preload='none' src='${esc(e.file)}'></audio>`,
    `<table>`,
    htmlRow(MEASURE_HEADERS, 'th'),
    htmlRow(measureCells(e), 'td'),
    `</table>`,
    ...(e.note !== undefined && e.note !== '' ? [`<p class='note'>${esc(e.note)}</p>`] : []),
    `</section>`,
  ].join('\n')
}

/**
 * index.html: a standalone page (inline CSS, no script). Each entry has its title, an
 * `<audio controls preload='none'>` of its file (relative to the report) and its measures,
 * grouped by entry group, then the CHECK table. All text is HTML-escaped.
 */
export function reportHtml(r: Report): string {
  const title = `${r.proof} report`
  const out: string[] = [
    '<!DOCTYPE html>',
    `<html lang='en'>`,
    '<head>',
    `<meta charset='utf-8'>`,
    `<meta name='viewport' content='width=device-width, initial-scale=1'>`,
    `<title>${esc(title)}</title>`,
    `<style>\n${HTML_STYLE}\n</style>`,
    '</head>',
    '<body>',
    `<h1>${esc(title)}</h1>`,
  ]
  for (const [group, list] of groups(r.entries)) {
    out.push(`<h2>${esc(group)}</h2>`)
    for (const e of list) out.push(htmlEntry(e))
  }
  out.push('<h2>CHECK</h2>', '<table>', htmlRow(CHECK_HEADERS, 'th'))
  for (const c of r.checks) out.push(htmlRow(checkCells(c), 'td', c.pass ? 'pass' : 'fail'))
  out.push('</table>', '</body>', '</html>', '')
  return out.join('\n')
}
