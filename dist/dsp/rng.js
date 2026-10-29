// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe randomness and seed hashing. Runs unchanged in node, on the main thread and in
// AudioWorkletGlobalScope. No random-number globals, clocks, timers, DOM or Web Audio.
// The per-sample methods never allocate.
import { TAU } from "./math.js";
/** FNV-1a 32 over the parts; returns an unsigned 32-bit integer. */
export function hashSeed(...parts) {
    let h = 0x811c9dc5;
    const feed = (byte) => {
        h ^= byte & 0xff;
        h = Math.imul(h, 0x01000193);
    };
    for (let i = 0; i < parts.length; i++) {
        if (i > 0)
            feed(0xff);
        const p = parts[i];
        if (typeof p === 'number') {
            const x = p >>> 0;
            feed(x & 0xff);
            feed((x >>> 8) & 0xff);
            feed((x >>> 16) & 0xff);
            feed((x >>> 24) & 0xff);
        }
        else {
            for (let j = 0; j < p.length; j++) {
                const c = p.charCodeAt(j);
                feed(c & 0xff);
                feed((c >>> 8) & 0xff);
            }
        }
    }
    return h >>> 0;
}
/**
 * mulberry32 PRNG. Audio's own stream: it never reads or writes a simulation Rng.
 */
export class AudioRng {
    s;
    gaussCache = null;
    constructor(seed) {
        this.s = seed >>> 0;
    }
    get state() {
        return this.s;
    }
    set state(v) {
        this.s = v >>> 0;
    }
    /** Uniform in [0, 1). */
    next() {
        this.s = (this.s + 0x6d2b79f5) >>> 0;
        let t = this.s;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    }
    /** Uniform in [a, b). */
    range(a, b) {
        return a + (b - a) * this.next();
    }
    /** Uniform integer in [a, b], inclusive on both ends. */
    int(a, b) {
        const lo = Math.ceil(Math.min(a, b));
        const hi = Math.floor(Math.max(a, b));
        return lo + Math.floor(this.next() * (hi - lo + 1));
    }
    /** Standard normal via Box-Muller; the second value of each pair is cached. */
    gauss() {
        if (this.gaussCache !== null) {
            const cached = this.gaussCache;
            this.gaussCache = null;
            return cached;
        }
        const u = this.next();
        const v = this.next();
        const r = Math.sqrt(-2 * Math.log(u > 0 ? u : 1e-300));
        const theta = TAU * v;
        this.gaussCache = r * Math.sin(theta);
        return r * Math.cos(theta);
    }
    /** A fresh independent stream derived from the current state and the salt; does not advance this one. */
    fork(salt) {
        return new AudioRng(hashSeed(this.s, salt));
    }
}
