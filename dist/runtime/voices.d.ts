import type { VoiceCategory } from '../contract.js';
/**
 * Hands out sampler voice ids 0..SAMPLER_VOICES-1 with per-category limits. When a
 * category (or the whole pool) is full, a request with a higher score steals the
 * lowest-score live voice of that category (or of all voices) and reuses its id.
 * All state is preallocated; request and expire never allocate beyond the result.
 */
export declare class VoiceAllocator {
    #private;
    /** Builds an allocator with the maximum live voices per category. */
    constructor(limits: Record<VoiceCategory, number>);
    /** Requests dropped because nothing could be freed or stolen. */
    get dropped(): number;
    /** Requests served by stealing a live voice. */
    get stolenCount(): number;
    /** Live voices of a category. */
    active(cat: VoiceCategory): number;
    /** Frees every voice whose end time is at or before now. */
    expire(now: number): void;
    /**
     * Allocates a voice lasting `duration` seconds from `now`. Returns its id and the id
     * that was stolen (-1 when none), or null when the request is dropped.
     */
    request(cat: VoiceCategory, score: number, now: number, duration: number): {
        id: number;
        stolen: number;
    } | null;
}
/**
 * Seeded variant choice and micro-variation. One AudioRng drives every draw, so the
 * same seed and call sequence always give the same picks and jitters.
 */
export declare class VariationPicker {
    #private;
    /** Builds a picker on one seeded stream. */
    constructor(seed: number);
    /** Uniform over 0..count-1, never repeating the last pick of `id` when count > 1. */
    pick(id: string, count: number): number;
    /** Writes rate = 1 ± MICRO_VARIATION.rate and gainDb = ± MICRO_VARIATION.gainDb. */
    jitter(out: {
        rate: number;
        gainDb: number;
    }): void;
}
