import type { MixPreset } from '../contract.js';
/** Loads each family once and retains frame counts even after the sink transfers its buffers. */
export declare class BankLoader {
    private readonly ctx;
    private readonly manifestUrl;
    private readonly preset;
    private readonly sink;
    private readonly pending;
    private readonly lengths;
    private readonly warned;
    private manifestPromise;
    private tail;
    /** Construct a browser loader; network activity starts with ensure(). */
    constructor(ctx: BaseAudioContext, manifestUrl: string, preset: MixPreset, sink: (id: string, variants: Float32Array[], rate: number) => void);
    /** Queue requested families (all registered families by default); failures never reject. */
    ensure(ids?: readonly string[]): Promise<void>;
    /** Number of loaded variants, or zero until loading succeeds. */
    loaded(id: string): number;
    /** Variant length in frames at the context sample rate, or zero when unknown. */
    variantLength(id: string, v: number): number;
    private warn;
    private manifest;
    private load;
}
