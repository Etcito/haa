// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe seamless loop tables. Runs unchanged in node, on the main thread and in
// AudioWorkletGlobalScope. LoopTable.read never allocates; makeSeamless and seamScore
// run at proof / table-build time only.
/** Cyclic table with linear interpolation. */
export class LoopTable {
    length;
    data;
    constructor(data) {
        this.data = data;
        this.length = data.length;
    }
    /** Read at a fractional position; pos wraps cyclically. */
    read(pos) {
        const n = this.length;
        if (n === 0)
            return 0;
        let p = pos % n;
        if (p < 0)
            p += n;
        const i0 = Math.floor(p);
        const frac = p - i0;
        const i1 = i0 + 1 >= n ? 0 : i0 + 1;
        const a = this.data[i0];
        const b = this.data[i1];
        return a + (b - a) * frac;
    }
}
/**
 * Build a seamless table of length buf.length - crossfade.
 * The tail (the last `crossfade` samples) is faded into the head with equal-power gains:
 * the head uses cos(t * pi/2), the tail sin(t * pi/2), t going 0..1 across the region.
 */
export function makeSeamless(buf, crossfade) {
    const len = buf.length;
    const c = Math.max(0, Math.min(Math.floor(crossfade), len));
    const n = len - c;
    const out = new Float32Array(n);
    for (let i = 0; i < n; i++)
        out[i] = buf[i];
    if (c > 0) {
        const step = Math.PI / 2 / c;
        const end = Math.min(c, n);
        for (let i = 0; i < end; i++) {
            const t = i * step;
            out[i] = buf[i] * Math.cos(t) + buf[n + i] * Math.sin(t);
        }
    }
    return out;
}
/**
 * Seam quality of a loop table: the wrap jump |t[0] - t[N-1]| and the 99th percentile of
 * the per-sample step |t[i+1] - t[i]|. Allocates (it sorts a copy): proof time only.
 */
export function seamScore(t) {
    const n = t.length;
    if (n < 2)
        return { jump: 0, p99: 0 };
    const jump = Math.abs(t[0] - t[n - 1]);
    const steps = new Array(n - 1);
    for (let i = 0; i < n - 1; i++)
        steps[i] = Math.abs(t[i + 1] - t[i]);
    steps.sort((a, b) => a - b);
    const idx = Math.min(n - 2, Math.ceil(0.99 * (n - 1)) - 1);
    return { jump, p99: steps[idx < 0 ? 0 : idx] };
}
