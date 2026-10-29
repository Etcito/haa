// FLAT_SPATIAL: the spatial model used when no model is set. Written for HAA from the
// shared FLAT rule. DSP-safe: pure arithmetic on the caller's preallocated result.
import { MAX_LISTENERS, REVERB_SEND_BASE, SP, SP_LENGTH } from "../contract.js";
/**
 * Every listener l < N gets gain 1/N, pan 0 and lowpass 20000, with send REVERB_SEND_BASE
 * and rate 1, where N = max(1, min(listeners.length, MAX_LISTENERS)). Positions and
 * occlusion are ignored; slots of listeners >= N are zeroed.
 */
export const FLAT_SPATIAL = {
    evaluate(listeners, _src, _occlusion, out) {
        const sp = out.sp;
        const n = Math.max(1, Math.min(listeners.length, MAX_LISTENERS));
        for (let i = 0; i < SP_LENGTH; i++)
            sp[i] = 0;
        sp[SP.COUNT] = n;
        sp[SP.SEND] = REVERB_SEND_BASE;
        for (let l = 0; l < n; l++) {
            sp[SP.GAIN + l] = 1 / n;
            sp[SP.LOWPASS + l] = 20000;
        }
        out.rate = 1;
        return out;
    }
};
