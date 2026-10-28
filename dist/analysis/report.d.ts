import type { Measure } from './measure.js';
/** One measured file of a report. file is relative to the report folder. */
export interface ReportEntry {
    group: string;
    id: string;
    file: string;
    title: string;
    measure: Measure;
    targetLufs?: number;
    note?: string;
}
/** One CHECK line of a proof. */
export interface ReportCheck {
    label: string;
    measured: string;
    threshold: string;
    pass: boolean;
}
/** The report of one proof run. */
export interface Report {
    proof: string;
    entries: ReportEntry[];
    checks: ReportCheck[];
}
/**
 * report.md: one table per entry group (id, seconds, LUFS, target, sample peak, true peak, DC,
 * min window RMS, the 10 band levels), the entry notes under their table, then the CHECK table.
 */
export declare function reportMarkdown(r: Report): string;
/**
 * index.html: a standalone page (inline CSS, no script). Each entry has its title, an
 * `<audio controls preload='none'>` of its file (relative to the report) and its measures,
 * grouped by entry group, then the CHECK table. All text is HTML-escaped.
 */
export declare function reportHtml(r: Report): string;
