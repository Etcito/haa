/** A full reverb preset for one zone. */
function zone(size, decay, damping = 6000, predelay = 0.02, returnDb = -14) {
    return { size, decay, damping, predelay, returnDb };
}
/** One reverb preset per zone, the submerged zone included. */
export const REVERB_ZONES = {
    open: zone(0.3, 0.8),
    canyon: zone(0.8, 2.2, 6000, 0.06),
    snowfield: zone(0.5, 1.4, 3500),
    cavern: zone(0.9, 2.8),
    water: zone(0.6, 1.6),
    city: zone(0.7, 1.8),
    sky: zone(0.2, 0.6),
    forest: zone(0.5, 1.2, 3000),
    room: zone(0.4, 1),
    underwater: zone(0.6, 1.8, 1200)
};
/** Per-zone air absorption multiplier (0.6 snowfield / 0.7 forest / 1.4 sky / 1 elsewhere). */
export const ABSORPTION_SCALE = {
    open: 1,
    canyon: 1,
    snowfield: 0.6,
    cavern: 1,
    water: 1,
    city: 1,
    sky: 1.4,
    forest: 0.7,
    room: 1
};
/** Linear blend of a and b into out (t clamped to 0..1, NaN counts as 0). No allocation. */
export function blendReverb(a, b, t, out) {
    const k = t >= 1 ? 1 : t > 0 ? t : 0;
    out.size = a.size + (b.size - a.size) * k;
    out.decay = a.decay + (b.decay - a.decay) * k;
    out.damping = a.damping + (b.damping - a.damping) * k;
    out.predelay = a.predelay + (b.predelay - a.predelay) * k;
    out.returnDb = a.returnDb + (b.returnDb - a.returnDb) * k;
    return out;
}
