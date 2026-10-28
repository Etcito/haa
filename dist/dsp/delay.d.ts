/**
 * Circular buffer. read(d) returns the signal d samples before the most recent write
 * (d in [1, maxSamples], fractional, 4-point 3rd-order Hermite interpolation).
 */
export declare class DelayLine {
    readonly maxSamples: number;
    private readonly buf;
    private readonly mask;
    private pos;
    constructor(maxSamples: number);
    write(x: number): void;
    read(delay: number): number;
    reset(): void;
}
/**
 * Tube resonator: a delay of sr / hz with a one-pole damping lowpass and a reflection
 * coefficient in the loop. y[n] = x[n] + reflection * lowpass(y[n - sr / hz]).
 * The lowpass phase delay at hz is subtracted so the loop stays tuned.
 */
export declare class Waveguide {
    private readonly line;
    private readonly damp;
    private readonly sr;
    private delay;
    private reflection;
    constructor(maxSamples: number, sr: number);
    set(hz: number, reflection: number, dampingHz: number): void;
    process(x: number): number;
    reset(): void;
}
