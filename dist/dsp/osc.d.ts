/** A sample-rate locked phase accumulator whose next() returns the phase in [0, 1). */
export declare class Phasor {
    private inc;
    private ph;
    /** Set the frequency and sample rate; the current phase is preserved. */
    set(hz: number, sr: number): void;
    get phase(): number;
    set phase(v: number);
    /** The phase before advancing, in [0, 1). */
    next(): number;
    reset(): void;
}
/**
 * Additive partial bank with one phase accumulator per partial. render() ADDS its
 * band-limited sum into the output buffer; partials with f0 * ratio >= 0.45 sr are
 * skipped. This is an internal building block, never routed raw to an output.
 */
export declare class AdditiveBank {
    private readonly ratios;
    private readonly amps;
    private readonly phases;
    private readonly initial;
    private readonly maxPartials;
    private n;
    constructor(maxPartials: number);
    /** Number of active partials (highest configured index + 1). */
    get count(): number;
    /** Configure partial i; the phase defaults to 0 and is also the reset phase. */
    setPartial(i: number, ratio: number, amp: number, phase?: number): void;
    /**
     * Add gain * sum(amp_i * sin(phase_i)) into out[0..n) for every partial below the
     * Nyquist guard. Phase accumulators advance by f0 * ratio / sr each sample.
     */
    render(f0: number, out: Float32Array, n: number, sr: number, gain: number): void;
    /** Reset every phase accumulator to its configured phase. */
    reset(): void;
}
