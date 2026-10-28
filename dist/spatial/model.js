// createSpatial: the HAA spatial model. It runs spatializeOne per listener, applies the 1/N
// per-listener factor, the dominant-listener rule for send and rate, and the null-source rule.
// Written for HAA from the shared 'createSpatial formulas'. DSP-safe: every record is
// preallocated at creation, so evaluate never allocates.
import { MAX_LISTENERS, REVERB_SEND_BASE, SP, SP_LENGTH, spGain, spLowpass, spPan } from "../contract.js";
import { OPEN_LOWPASS_HZ, spatializeOne } from "./spatial.js";
/**
 * Create the spatial model. absorptionScale (default 1) scales the air absorption distance.
 * With 0 listeners, one listener at the origin with the identity quat is used; N =
 * min(listeners.length, MAX_LISTENERS). Listener l gets gain × 1/N; sp[SP.SEND] and rate come
 * from the listener with the highest gain (ties: the lowest index). A null source gets gain
 * 1/N, pan 0 and lowpass 20000 per listener, send REVERB_SEND_BASE and rate 1.
 */
export function createSpatial(o) {
    const env = { absorptionScale: o?.absorptionScale ?? 1 };
    const one = [];
    for (let l = 0; l < MAX_LISTENERS; l++)
        one.push({ gain: 0, pan: 0, lowpass: 0, send: 0, doppler: 1, dist: 0 });
    const defaultListeners = [{ pos: { x: 0, y: 0, z: 0 }, quat: { x: 0, y: 0, z: 0, w: 1 } }];
    return {
        evaluate(listeners, src, occlusion, out) {
            const ls = listeners.length === 0 ? defaultListeners : listeners;
            const n = Math.min(ls.length, MAX_LISTENERS);
            const inv = 1 / n;
            const sp = out.sp;
            for (let i = 0; i < SP_LENGTH; i++)
                sp[i] = 0;
            sp[SP.COUNT] = n;
            if (src === null) {
                for (let l = 0; l < n; l++) {
                    sp[spGain(l)] = inv;
                    sp[spLowpass(l)] = OPEN_LOWPASS_HZ;
                }
                sp[SP.SEND] = REVERB_SEND_BASE;
                out.rate = 1;
                return out;
            }
            let best = 0;
            for (let l = 0; l < n; l++) {
                const r = spatializeOne(ls[l], src, n, occlusion?.(src.pos, l) ?? 0, env, one[l]);
                sp[spGain(l)] = r.gain * inv;
                sp[spPan(l)] = r.pan;
                sp[spLowpass(l)] = r.lowpass;
                if (r.gain > one[best].gain)
                    best = l;
            }
            sp[SP.SEND] = one[best].send;
            out.rate = one[best].doppler;
            return out;
        }
    };
}
