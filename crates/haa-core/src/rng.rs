//! Seeded random numbers for nodes: Sona's FNV-1a node seed and xorshift32 generator.
//! Ported from Sona v11 `dist/dsp.js` (`seedFromId`, `random`) and `dist/primitives.js` (`random` scaling to 0..1).
//! Deviation: Sona hashes `charCodeAt` units, so the hash walks `encode_utf16`; the arithmetic is the same u32 wrapping arithmetic.

const FNV_OFFSET: u32 = 2_166_136_261;
const FNV_PRIME: u32 = 16_777_619;

/// FNV-1a 32 over the UTF-16 code units of `s`, with Sona's zero fallback.
pub fn fnv1a32(s: &str) -> u32 {
    let mut hash = FNV_OFFSET;
    for unit in s.encode_utf16() {
        hash = (hash ^ u32::from(unit)).wrapping_mul(FNV_PRIME);
    }
    if hash == 0 { 1 } else { hash }
}

/// Xorshift32 generator (shifts 13, 17, 5), Sona's `random` state update.
pub struct Rng {
    state: u32,
}

impl Rng {
    /// A zero seed falls back to 1, because xorshift never leaves the zero state.
    pub fn new(seed: u32) -> Rng {
        Rng {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// Next value in 0..1, scaled by 2^32 as Sona's `primitives.js` does.
    pub fn next_f64(&mut self) -> f64 {
        f64::from(self.next_u32()) / 4_294_967_296.0
    }
}

/// Seed of one node in one voice: the hash of its id, XOR the voice seed.
pub fn node_seed(node_id: &str, voice_seed: u32) -> u32 {
    fnv1a32(node_id) ^ voice_seed
}

#[cfg(test)]
mod tests {
    use super::*;

    // Seed 1, xorshift32 with shifts 13, 17, 5 (hex, 32-bit):
    // 1: x ^= x<<13 -> 00002001; x ^= x>>17 -> 00002001; x ^= x<<5 -> 00042021 = 270369
    // 2: x ^= x<<13 -> 84000021 (x<<13 = 84042000); x ^= x>>17 -> 84004221 (x>>17 = 4200);
    //    x ^= x<<5 -> 04080601 (x<<5 = 80084420) = 67634689
    // 3: x ^= x<<13 -> 04C82601 (x<<13 = 00C02000); x ^= x>>17 -> 04C82465 (x>>17 = 0264);
    //    x ^= x<<5 -> 9DCCA8C5 (x<<5 = 99048CA0) = 2647435461
    #[test]
    fn first_three_values_for_seed_one() {
        let mut rng = Rng::new(1);
        assert_eq!(rng.next_u32(), 270_369);
        assert_eq!(rng.next_u32(), 67_634_689);
        assert_eq!(rng.next_u32(), 2_647_435_461);
    }

    #[test]
    fn zero_seed_never_gives_zero_state() {
        let mut zero = Rng::new(0);
        let mut one = Rng::new(1);
        for _ in 0..10_000 {
            let value = zero.next_u32();
            assert_ne!(value, 0);
            assert_eq!(value, one.next_u32());
        }
    }

    #[test]
    fn fnv_hash_matches_fnv1a_vectors() {
        assert_eq!(fnv1a32(""), 0x811c_9dc5);
        assert_eq!(fnv1a32("a"), 0xe40c_292c);
        assert_eq!(node_seed("a", 0), 0xe40c_292c);
    }
}
