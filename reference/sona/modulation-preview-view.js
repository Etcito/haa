import { tr, locale } from './i18n.js';

const escape = value => String(value ?? '').replace(/[&<>"']/g, character => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[character]));
const clamp = (value, low, high) => Math.min(high, Math.max(low, value));
const bipolar = new Set(['lfo', 'random']);
const modes = {
  preview: ['Output preview', 'First pass from rest. Incoming modulation is included.'],
  live: ['Live output', 'Actual playback output. The band preserves fast changes.'],
  pending: ['Calculating…', 'Updating to match the current graph.'],
  waiting: ['Waiting for signal…', 'Waiting for output from the playing graph.'],
  unavailable: ['Preview unavailable', 'No calculated output is available.'],
};

// Formatting and geometry helpers are called by the preview render loop.
function numberLabel(value) {
  if (!Number.isFinite(value)) return '—';
  if (Object.is(value, -0) || value === 0) return '0';
  const magnitude = Math.abs(value);
  return value.toLocaleString(locale(), magnitude >= 10000 || magnitude < .001
    ? { notation: 'scientific', maximumFractionDigits: 2, useGrouping: false }
    : { maximumSignificantDigits: 3, useGrouping: false });
}

function niceBound(value) {
  if (!Number.isFinite(value) || value === 0) return 0;
  const magnitude = Math.abs(value), power = 10 ** Math.floor(Math.log10(magnitude));
  const fraction = magnitude / power;
  const step = fraction <= 1 ? 1 : fraction <= 2 ? 2 : fraction <= 5 ? 5 : 10;
  return Math.sign(value) * step * power;
}

function valueDomain(node, low, high) {
  if (bipolar.has(node.type)) return [Math.min(-1, niceBound(low)), Math.max(1, niceBound(high))];
  if (node.type !== 'math') return [Math.min(0, niceBound(low)), Math.max(1, niceBound(high))];
  let minimum = niceBound(Math.min(0, low)), maximum = niceBound(Math.max(0, high));
  if (maximum - minimum < 1) {
    if (maximum === 0 && minimum < 0) minimum = -1;
    else if (minimum < 0) { minimum = -1; maximum = 1; }
    else maximum = 1;
  }
  return [minimum, maximum];
}

/** Render measured output only. Pending states deliberately contain no invented trace. */
export function renderModulationPreview(node, frame, { name = '', expanded = false } = {}) {
  const mode = modes[frame?.mode] ? frame.mode : 'pending';
  const [label, description] = modes[mode];
  const start = Number.isFinite(frame?.start) ? frame.start : 0;
  const end = Number.isFinite(frame?.end) && frame.end > start ? frame.end : start + 1;
  const hasOutput = mode === 'preview' || mode === 'live';
  const points = hasOutput && Array.isArray(frame?.points)
    ? frame.points.filter(point => Array.isArray(point) && point.length >= 4 && point.every(Number.isFinite) && point[0] >= start && point[0] <= end)
    : [];
  let low = 0, high = 0;
  if (points.length) {
    low = Infinity; high = -Infinity;
    for (const point of points) { low = Math.min(low, point[1], point[2], point[3]); high = Math.max(high, point[1], point[2], point[3]); }
  }
  const current = mode === 'live' && points.length
    ? (Number.isFinite(frame?.current) ? frame.current : points.at(-1)[3]) : null;
  if (current !== null) { low = Math.min(low, current); high = Math.max(high, current); }
  const [minimum, maximum] = valueDomain(node, low, high);
  const width = 192, height = expanded ? 78 : 40;
  const left = 29, right = 190, top = 4, bottom = height - 11;
  const x = time => left + clamp((time - start) / (end - start), 0, 1) * (right - left);
  const y = value => bottom - clamp((value - minimum) / (maximum - minimum), 0, 1) * (bottom - top);
  const coordinate = value => Number(value.toFixed(3));
  const pointAt = (time, value) => `${coordinate(x(time))},${coordinate(y(value))}`;
  let geometry = '';
  if (points.length) {
    const upper = points.map(point => pointAt(point[0], Math.max(point[1], point[2])));
    const lower = points.map(point => pointAt(point[0], Math.min(point[1], point[2]))).reverse();
    const line = points.map(point => pointAt(point[0], point[3]));
    const bars = points.map(point => `M${pointAt(point[0], point[1])}L${pointAt(point[0], point[2])}`).join('');
    const last = points.at(-1);
    geometry = `<path class="modulation-band" d="M${upper.join('L')}L${lower.join('L')}Z"/><path class="modulation-extents" d="${bars}"/><polyline class="modulation-line" points="${line.join(' ')}"/>${mode === 'live' ? `<circle class="modulation-current" cx="${coordinate(x(last[0]))}" cy="${coordinate(y(current))}" r="1.8"/>` : ''}`;
  }
  const range = `${numberLabel(minimum)} … ${numberLabel(maximum)}`;
  const timeRange = `${numberLabel(start)}–${numberLabel(end)} s`;
  const observedLow = numberLabel(low), observedHigh = numberLabel(high);
  const valueLabel = current !== null ? numberLabel(current) : mode === 'preview' && points.length ? observedLow === observedHigh ? observedLow : `${observedLow} … ${observedHigh}` : '';
  const valueTitle = current !== null ? 'Current value' : observedLow === observedHigh ? 'Output value' : 'Output minimum and maximum';
  const accessible = `${name || tr(node.label || node.type)} · ${tr(label)} · ${tr('Value range')}: ${range} · ${tr('Time')}: ${timeRange}${valueLabel ? ` · ${tr(valueTitle)}: ${valueLabel}` : ''}`;
  const tooltip = `${tr(description)} ${tr('The band shows the minimum and maximum within each time interval.')}`;
  const empty = points.length ? '' : `<text class="modulation-empty" x="${(left + right) / 2}" y="${(top + bottom) / 2 + 2}" text-anchor="middle">${escape(tr(hasOutput ? 'Waiting for signal…' : label))}</text>`;
  return `<div class="modulation-preview${expanded ? ' modulation-preview-expanded' : ''}" data-mode="${mode}" data-range-min="${minimum}" data-range-max="${maximum}" role="img" aria-label="${escape(accessible)}" title="${escape(tooltip)}"><div class="modulation-preview-heading" aria-hidden="true"><span class="modulation-preview-label"><i></i>${escape(tr(label))}</span>${valueLabel ? `<span class="modulation-value" title="${escape(tr(valueTitle))}">${escape(valueLabel)}</span>` : ''}</div><svg viewBox="0 0 ${width} ${height}" aria-hidden="true" focusable="false"><path class="modulation-grid" d="M${left} ${top}H${right}M${left} ${bottom}H${right}"/><path class="modulation-zero" d="M${left} ${coordinate(y(0))}H${right}"/>${geometry}${empty}<text class="modulation-axis" x="25" y="${top + 3}" text-anchor="end">${escape(numberLabel(maximum))}</text><text class="modulation-axis" x="25" y="${bottom + 2}" text-anchor="end">${escape(numberLabel(minimum))}</text><text class="modulation-axis modulation-time" x="${left}" y="${height - 1}">${escape(numberLabel(start))} s</text><text class="modulation-axis modulation-time" x="${right}" y="${height - 1}" text-anchor="end">${escape(numberLabel(end))} s</text></svg>${expanded ? `<p class="modulation-preview-caption">${escape(tr(description))}</p>` : ''}</div>`;
}
