// Voice allocation and per-play variation for the conductor. Written for HAA from the
// ARCHITECTURE formulas (not ported from the origin engine's director). DSP-safe: no clocks,
// no global randomness, no DOM; the caller supplies the time and the seed.
import { MICRO_VARIATION, SAMPLER_VOICES, VOICE_CATEGORIES } from "../contract.js";
import { AudioRng } from "../dsp/rng.js";
/** Category index stored for a free slot. */
const FREE = -1;
/**
 * Hands out sampler voice ids 0..SAMPLER_VOICES-1 with per-category limits. When a
 * category (or the whole pool) is full, a request with a higher score steals the
 * lowest-score live voice of that category (or of all voices) and reuses its id.
 * All state is preallocated; request and expire never allocate beyond the result.
 */
export class VoiceAllocator {
    #limits;
    #cat = new Int32Array(SAMPLER_VOICES).fill(FREE);
    #score = new Float64Array(SAMPLER_VOICES);
    #end = new Float64Array(SAMPLER_VOICES);
    #counts = new Int32Array(VOICE_CATEGORIES.length);
    #free = new Int32Array(SAMPLER_VOICES);
    #freeTop = SAMPLER_VOICES;
    #dropped = 0;
    #stolen = 0;
    /** Builds an allocator with the maximum live voices per category. */
    constructor(limits) {
        this.#limits = new Int32Array(VOICE_CATEGORIES.length);
        for (let c = 0; c < VOICE_CATEGORIES.length; c++)
            this.#limits[c] = Math.max(0, Math.floor(limits[VOICE_CATEGORIES[c]] ?? 0));
        // Pop order hands out 0 first.
        for (let i = 0; i < SAMPLER_VOICES; i++)
            this.#free[i] = SAMPLER_VOICES - 1 - i;
    }
    /** Requests dropped because nothing could be freed or stolen. */
    get dropped() { return this.#dropped; }
    /** Requests served by stealing a live voice. */
    get stolenCount() { return this.#stolen; }
    /** Live voices of a category. */
    active(cat) {
        const c = VOICE_CATEGORIES.indexOf(cat);
        return c < 0 ? 0 : this.#counts[c];
    }
    /** Frees every voice whose end time is at or before now. */
    expire(now) {
        for (let id = 0; id < SAMPLER_VOICES; id++) {
            const c = this.#cat[id];
            if (c !== FREE && this.#end[id] <= now) {
                this.#cat[id] = FREE;
                this.#counts[c]--;
                this.#free[this.#freeTop++] = id;
            }
        }
    }
    /**
     * Allocates a voice lasting `duration` seconds from `now`. Returns its id and the id
     * that was stolen (-1 when none), or null when the request is dropped.
     */
    request(cat, score, now, duration) {
        const c = VOICE_CATEGORIES.indexOf(cat);
        if (c < 0) {
            this.#dropped++;
            return null;
        }
        const s = Number.isFinite(score) ? score : 0;
        const end = now + (Number.isFinite(duration) && duration > 0 ? duration : 0);
        if (this.#counts[c] < this.#limits[c] && this.#freeTop > 0) {
            const id = this.#free[--this.#freeTop];
            this.#assign(id, c, s, end);
            return { id, stolen: -1 };
        }
        // Full category: steal within it; global exhaustion: steal over all voices.
        const within = this.#counts[c] >= this.#limits[c];
        let victim = -1;
        for (let id = 0; id < SAMPLER_VOICES; id++) {
            const vc = this.#cat[id];
            if (vc === FREE || (within && vc !== c))
                continue;
            if (victim < 0 || this.#score[id] < this.#score[victim])
                victim = id;
        }
        if (victim < 0 || s <= this.#score[victim]) {
            this.#dropped++;
            return null;
        }
        this.#counts[this.#cat[victim]]--;
        this.#assign(victim, c, s, end);
        this.#stolen++;
        return { id: victim, stolen: victim };
    }
    #assign(id, c, score, end) {
        this.#cat[id] = c;
        this.#score[id] = score;
        this.#end[id] = end;
        this.#counts[c]++;
    }
}
/**
 * Seeded variant choice and micro-variation. One AudioRng drives every draw, so the
 * same seed and call sequence always give the same picks and jitters.
 */
export class VariationPicker {
    #rng;
    #last = new Map();
    /** Builds a picker on one seeded stream. */
    constructor(seed) {
        this.#rng = new AudioRng(seed);
    }
    /** Uniform over 0..count-1, never repeating the last pick of `id` when count > 1. */
    pick(id, count) {
        const n = Math.floor(count);
        if (!(n > 1)) {
            this.#last.set(id, 0);
            return 0;
        }
        const last = this.#last.get(id);
        let v;
        if (last === undefined || last < 0 || last >= n) {
            v = Math.min(n - 1, Math.floor(this.#rng.next() * n));
        }
        else {
            v = Math.min(n - 2, Math.floor(this.#rng.next() * (n - 1)));
            if (v >= last)
                v++;
        }
        this.#last.set(id, v);
        return v;
    }
    /** Writes rate = 1 ± MICRO_VARIATION.rate and gainDb = ± MICRO_VARIATION.gainDb. */
    jitter(out) {
        out.rate = 1 + this.#rng.range(-MICRO_VARIATION.rate, MICRO_VARIATION.rate);
        out.gainDb = this.#rng.range(-MICRO_VARIATION.gainDb, MICRO_VARIATION.gainDb);
    }
}
