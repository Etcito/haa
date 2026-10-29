export interface Mode {
    hz: number;
    t60: number;
    gain: number;
}
/**
 * Sum of 2-pole resonators y[n] = 2 r cos(w) y[n-1] - r^2 y[n-2] + gain sin(w) x[n],
 * so a unit impulse rings as gain r^n sin((n + 1) w). Pole radius r = 10^(-3 / (t60 sr)).
 * Modes at or above 0.45 sr (or with a non-positive frequency) are skipped.
 */
export declare class ModalBank {
    readonly maxModes: number;
    private readonly sr;
    private readonly b;
    private readonly a1;
    private readonly a2;
    private readonly y1;
    private readonly y2;
    private count;
    constructor(maxModes: number, sr: number);
    /**
     * Loads up to maxModes modes, scaling every frequency by freqScale and every t60 by
     * decayScale. Resonator states are kept so modes can be retuned while ringing; a mode
     * that becomes skipped is silenced.
     */
    setModes(modes: readonly Mode[], freqScale?: number, decayScale?: number): void;
    private silence;
    process(excitation: number): number;
    /** Sum of the squared resonator states (both delay taps of every mode). */
    energy(): number;
    reset(): void;
}
