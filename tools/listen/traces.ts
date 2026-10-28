// Listen page trace panel: one small labelled canvas per plan node, ring buffers over the
// last 8 s of HAA and Sona control values, and a requestAnimationFrame loop that draws them.
// HAA is a solid line in --cyan, Sona a dashed line in --amber (ARCHITECTURE §17).

/** Seconds of trace history kept and shown. */
const WINDOW_SECONDS = 8;

/** Samples per engine and node; 25 Hz over 8 s is 200, with headroom for faster senders. */
const CAPACITY = 512;

const HAA_COLOR = '--cyan';
const SONA_COLOR = '--amber';
const HAA_FALLBACK = '#4dd0e1';
const SONA_FALLBACK = '#ffb74d';

/** A node the panel draws; `label` is used when non-empty, else `id`. */
export interface TraceNode {
  id: string;
  label: string;
}

/** The panel main.ts pushes trace packets into. */
export interface TracePanel {
  pushHaa(seconds: number, values: Float32Array): void;
  pushSona(seconds: number, values: Record<string, number>): void;
  clear(): void;
  dispose(): void;
}

/** Fixed-capacity ring of (time, value) samples. */
interface Ring {
  times: Float64Array;
  values: Float32Array;
  /** Index of the oldest sample. */
  head: number;
  count: number;
  /** Newest pushed time; a push older than this restarts the buffer. */
  newest: number;
}

/** One node's canvas and its two engine rings. */
interface Series {
  id: string;
  canvas: HTMLCanvasElement;
  haa: Ring;
  sona: Ring;
}

function createRing(): Ring {
  return { times: new Float64Array(CAPACITY), values: new Float32Array(CAPACITY), head: 0, count: 0, newest: 0 };
}

function clearRing(ring: Ring): void {
  ring.head = 0;
  ring.count = 0;
  ring.newest = 0;
}

/** Push one sample, dropping entries older than the window and restarting on a time reset. */
function pushValue(ring: Ring, time: number, value: number): void {
  if (!Number.isFinite(time) || !Number.isFinite(value)) return;
  if (ring.count > 0 && time < ring.newest) clearRing(ring);
  if (ring.count === CAPACITY) {
    ring.head = (ring.head + 1) % CAPACITY;
    ring.count -= 1;
  }
  const at = (ring.head + ring.count) % CAPACITY;
  ring.times[at] = time;
  ring.values[at] = value;
  ring.count += 1;
  ring.newest = time;
  const cutoff = time - WINDOW_SECONDS;
  while (ring.count > 0 && ring.times[ring.head] < cutoff) {
    ring.head = (ring.head + 1) % CAPACITY;
    ring.count -= 1;
  }
}

/** Resize the backing store to the CSS size, returning its device-pixel dimensions. */
function sizeCanvas(canvas: HTMLCanvasElement): { width: number; height: number } {
  const dpr = window.devicePixelRatio || 1;
  const rect = canvas.getBoundingClientRect();
  const width = Math.max(1, Math.round((rect.width || 300) * dpr));
  const height = Math.max(1, Math.round((rect.height || 60) * dpr));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  return { width, height };
}

/** Min and max of the finite values inside [left, right] across the given rings. */
function bounds(rings: Ring[], left: number, right: number): { min: number; max: number } | null {
  let min = Infinity;
  let max = -Infinity;
  for (const ring of rings) {
    for (let i = 0; i < ring.count; i++) {
      const at = (ring.head + i) % CAPACITY;
      const time = ring.times[at];
      if (time < left || time > right) continue;
      const value = ring.values[at];
      if (!Number.isFinite(value)) continue;
      if (value < min) min = value;
      if (value > max) max = value;
    }
  }
  if (min === Infinity) return null;
  if (min === max) {
    min -= 1;
    max += 1;
  }
  return { min, max };
}

/** Stroke one ring across the window; a single visible sample becomes a dot. */
function stroke(
  g: CanvasRenderingContext2D,
  ring: Ring,
  left: number,
  right: number,
  min: number,
  max: number,
  width: number,
  height: number,
  color: string,
  dashed: boolean,
): void {
  const span = max - min;
  const visible: Array<{ x: number; y: number }> = [];
  for (let i = 0; i < ring.count; i++) {
    const at = (ring.head + i) % CAPACITY;
    const time = ring.times[at];
    if (time < left || time > right) continue;
    const value = ring.values[at];
    if (!Number.isFinite(value)) continue;
    visible.push({
      x: ((time - left) / WINDOW_SECONDS) * width,
      y: height - ((value - min) / span) * (height - 2) - 1,
    });
  }
  if (visible.length === 0) return;
  g.strokeStyle = color;
  g.fillStyle = color;
  g.lineWidth = Math.max(1, Math.round((window.devicePixelRatio || 1) * 1.25));
  g.lineJoin = 'round';
  g.lineCap = 'round';
  g.setLineDash(dashed ? [4, 3] : []);
  if (visible.length === 1) {
    const dot = visible[0];
    g.beginPath();
    g.arc(dot.x, dot.y, g.lineWidth, 0, Math.PI * 2);
    g.fill();
    return;
  }
  g.beginPath();
  g.moveTo(visible[0].x, visible[0].y);
  for (let i = 1; i < visible.length; i++) g.lineTo(visible[i].x, visible[i].y);
  g.stroke();
}

/** Create the trace panel inside `root`, one canvas per node in `nodes`. */
export function createTracePanel(root: HTMLElement, nodes: TraceNode[]): TracePanel {
  const styles = getComputedStyle(document.documentElement);
  const haaColor = styles.getPropertyValue(HAA_COLOR).trim() || HAA_FALLBACK;
  const sonaColor = styles.getPropertyValue(SONA_COLOR).trim() || SONA_FALLBACK;
  const byId = new Map<string, Series>();
  const series: Series[] = nodes.map((node) => {
    const figure = document.createElement('figure');
    figure.className = 'trace';
    const label = document.createElement('span');
    label.className = 'trace-label';
    label.textContent = node.label || node.id;
    const canvas = document.createElement('canvas');
    figure.append(label, canvas);
    root.append(figure);
    const entry: Series = { id: node.id, canvas, haa: createRing(), sona: createRing() };
    byId.set(node.id, entry);
    return entry;
  });

  function latest(): number | null {
    let newest: number | null = null;
    for (const s of series) {
      if (s.haa.count > 0 && (newest === null || s.haa.newest > newest)) newest = s.haa.newest;
      if (s.sona.count > 0 && (newest === null || s.sona.newest > newest)) newest = s.sona.newest;
    }
    return newest;
  }

  function draw(): void {
    if (disposed) return;
    frame = requestAnimationFrame(draw);
    const right = latest();
    for (const s of series) {
      const { width, height } = sizeCanvas(s.canvas);
      const g = s.canvas.getContext('2d');
      if (!g) continue;
      g.setLineDash([]);
      g.clearRect(0, 0, width, height);
      if (right === null) continue;
      const left = right - WINDOW_SECONDS;
      const range = bounds([s.haa, s.sona], left, right);
      if (!range) continue;
      stroke(g, s.sona, left, right, range.min, range.max, width, height, sonaColor, true);
      stroke(g, s.haa, left, right, range.min, range.max, width, height, haaColor, false);
    }
  }

  let frame = 0;
  let disposed = false;
  frame = requestAnimationFrame(draw);

  return {
    pushHaa(seconds: number, values: Float32Array): void {
      const count = Math.min(values.length, series.length);
      for (let i = 0; i < count; i++) pushValue(series[i].haa, seconds, values[i]);
    },
    pushSona(seconds: number, values: Record<string, number>): void {
      for (const [id, value] of Object.entries(values)) {
        const target = byId.get(id);
        if (target) pushValue(target.sona, seconds, value);
      }
    },
    clear(): void {
      for (const s of series) {
        clearRing(s.haa);
        clearRing(s.sona);
      }
    },
    dispose(): void {
      if (disposed) return;
      disposed = true;
      cancelAnimationFrame(frame);
      for (const s of series) {
        clearRing(s.haa);
        clearRing(s.sona);
      }
      root.replaceChildren();
    },
  };
}
