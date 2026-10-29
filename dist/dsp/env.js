// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe envelopes and detectors. Sample-rate agnostic (sr passed in). No allocation
// after construction; every class has reset(). Runs unchanged in node, on the main thread
// and in AudioWorkletGlobalScope.
const MIN_TIME = 1 / 192000;
/** One-pole approach coefficient for a time constant of `timeSec`. */
function coefFor(timeSec, sr) {
    const t = timeSec > MIN_TIME ? timeSec : MIN_TIME;
    return Math.exp(-1 / (t * (sr > 1 ? sr : 48000)));
}
/**
 * Linear-attack, exponential-decay/release ADSR. `active` turns false once the release
 * has run its full length.
 */
export class Adsr {
    stage = 0; // 0 idle, 1 attack, 2 decay, 3 sustain, 4 release
    level = 0;
    aSamples = 1;
    dSamples = 0;
    rSamples = 0;
    aLeft = 0;
    dLeft = 0;
    rLeft = 0;
    aInc = 1;
    decayCoef = 0;
    releaseCoef = 0;
    sus = 0;
    set(a, d, s, r, sr) {
        const rate = sr > 1 ? sr : 48000;
        const at = a > MIN_TIME ? a : MIN_TIME;
        const dt = d > MIN_TIME ? d : MIN_TIME;
        this.aSamples = Math.max(1, Math.round(at * rate));
        this.dSamples = Math.max(1, Math.round(dt * rate));
        this.rLeft = Math.max(1, Math.round((r > MIN_TIME ? r : MIN_TIME) * rate));
        this.aInc = 1 / this.aSamples;
        this.decayCoef = coefFor(dt, rate);
        this.releaseCoef = coefFor(r, rate);
        this.sus = Math.min(1, Math.max(0, s));
    }
    gate(on) {
        if (on) {
            this.stage = 1;
            this.aLeft = Math.max(1, Math.round((1 - this.level) / this.aInc));
        }
        else if (this.stage !== 0) {
            this.stage = 4;
            this.dLeft = 0;
            this.rSamples = 0;
        }
    }
    get active() {
        return this.stage !== 0;
    }
    next() {
        switch (this.stage) {
            case 1: {
                this.level += this.aInc;
                if (this.level >= 1 || --this.aLeft <= 0) {
                    this.level = 1;
                    this.stage = 2;
                    this.dLeft = this.dSamples;
                }
                return this.level;
            }
            case 2: {
                this.level = this.sus + (this.level - this.sus) * this.decayCoef;
                if (--this.dLeft <= 0) {
                    this.level = this.sus;
                    this.stage = 3;
                }
                return this.level;
            }
            case 3:
                return this.sus;
            case 4: {
                this.level *= this.releaseCoef;
                this.rSamples += 1;
                if (this.rSamples >= this.rLeft) {
                    this.level = 0;
                    this.stage = 0;
                }
                return this.level;
            }
            default:
                return 0;
        }
    }
    reset() {
        this.stage = 0;
        this.level = 0;
        this.aLeft = 0;
        this.dLeft = 0;
        this.rSamples = 0;
    }
}
/** Exponential decay that reaches -60 dB (0.001) after t60 seconds. */
export class ExpDecay {
    level = 0;
    coef = 0;
    trigger(level, t60, sr) {
        const rate = sr > 1 ? sr : 48000;
        this.level = Number.isFinite(level) ? level : 0;
        this.coef = t60 > MIN_TIME ? Math.pow(10, -3 / (t60 * rate)) : 0;
    }
    next() {
        const v = this.level;
        this.level *= this.coef;
        return v;
    }
    reset() {
        this.level = 0;
        this.coef = 0;
    }
}
/** Peak detector with independent attack and release coefficients. */
export class EnvFollower {
    env = 0;
    attack;
    release;
    constructor(attackMs, releaseMs, sr) {
        const rate = sr > 1 ? sr : 48000;
        this.attack = coefFor((attackMs > 0 ? attackMs : 1) / 1000, rate);
        this.release = coefFor((releaseMs > 0 ? releaseMs : 1) / 1000, rate);
    }
    process(x) {
        const a = Math.abs(x);
        const c = a > this.env ? this.attack : this.release;
        this.env = a + (this.env - a) * c;
        return this.env;
    }
    reset() {
        this.env = 0;
    }
}
