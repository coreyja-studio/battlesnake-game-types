//! MINSTD Park-Miller PRNG for deterministic game simulation.
//!
//! This implements the revised MINSTD algorithm (Park & Miller, 1993) using
//! multiplier 48271 and modulus 2^31 - 1. This is the same algorithm used in
//! C++11's `std::minstd_rand`.
//!
//! Per-turn seeding uses splitmix64 to derive independent seeds from a game
//! seed and turn number, allowing any turn to be simulated independently.

const MINSTD_A: u64 = 48271;
const MINSTD_M: u64 = 2_147_483_647; // 2^31 - 1
const SPLITMIX64_GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;

/// A MINSTD Park-Miller PRNG instance.
///
/// State is always in the range [1, 2^31 - 2]. The algorithm is:
/// `state(n+1) = (state(n) * 48271) mod (2^31 - 1)`
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MinstdRand {
    state: u64,
}

impl MinstdRand {
    /// Create a new MINSTD PRNG from a seed.
    ///
    /// The seed is mapped to the valid range [1, 2^31 - 2]. A seed of 0
    /// maps to 1 (state 0 is absorbing under the MINSTD recurrence).
    pub fn new(seed: i64) -> Self {
        let mut s = (seed as u64) % (MINSTD_M - 1);
        if s == 0 {
            s = 1;
        }
        Self { state: s }
    }

    /// Advance the PRNG state and return the new value.
    pub fn next(&mut self) -> u64 {
        self.state = (self.state * MINSTD_A) % MINSTD_M;
        self.state
    }

    /// Return a random integer in [0, n).
    pub fn intn(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// Return a random integer in [min, max] (inclusive).
    pub fn range(&mut self, min: i32, max: i32) -> i32 {
        self.intn((max - min + 1) as usize) as i32 + min
    }

    /// Fisher-Yates shuffle of a slice.
    pub fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = self.intn(i + 1);
            slice.swap(i, j);
        }
    }
}

/// Splitmix64 mixing function.
///
/// Provides excellent avalanche properties — a single bit change in the
/// input affects ~50% of output bits.
fn splitmix64_mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    x
}

/// Derive a per-turn MINSTD seed from a game seed and turn number.
///
/// Uses splitmix64 with a golden ratio pre-addition to avoid the 0→0
/// fixpoint in the raw finalizer. The result is always in the valid
/// MINSTD seed range [1, 2^31 - 2].
pub fn turn_seed(game_seed: u64, turn: u32) -> i64 {
    let combined = (game_seed ^ turn as u64).wrapping_add(SPLITMIX64_GOLDEN);
    let raw = splitmix64_mix(combined);
    let bounded = (raw % (MINSTD_M - 1)) + 1;
    bounded as i64
}

/// Create a MINSTD PRNG seeded for a specific game turn.
pub fn rng_for_turn(game_seed: u64, turn: u32) -> MinstdRand {
    MinstdRand::new(turn_seed(game_seed, turn))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequence_from_seed_1() {
        let mut rng = MinstdRand::new(1);
        assert_eq!(rng.next(), 48_271);
        assert_eq!(rng.next(), 182_605_794);
        assert_eq!(rng.next(), 1_291_394_886);
        assert_eq!(rng.next(), 1_914_720_637);
        assert_eq!(rng.next(), 2_078_669_041);
    }

    #[test]
    fn test_seed_zero_maps_to_one() {
        let rng_zero = MinstdRand::new(0);
        let rng_one = MinstdRand::new(1);
        // Seed 0 should produce the same sequence as seed 1
        assert_eq!(rng_zero.state, rng_one.state);
    }

    #[test]
    fn test_intn() {
        let mut rng = MinstdRand::new(42);
        for _ in 0..100 {
            let val = rng.intn(10);
            assert!(val < 10);
        }
    }

    #[test]
    fn test_range_inclusive() {
        let mut rng = MinstdRand::new(42);
        for _ in 0..100 {
            let val = rng.range(5, 10);
            assert!((5..=10).contains(&val));
        }
    }

    #[test]
    fn test_shuffle_preserves_elements() {
        let mut rng = MinstdRand::new(42);
        let mut data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let original = data.clone();
        rng.shuffle(&mut data);
        // Same elements, possibly different order
        let mut sorted = data.clone();
        sorted.sort();
        assert_eq!(sorted, original);
    }

    #[test]
    fn test_shuffle_deterministic() {
        let mut rng1 = MinstdRand::new(42);
        let mut rng2 = MinstdRand::new(42);
        let mut a = vec![1, 2, 3, 4, 5];
        let mut b = vec![1, 2, 3, 4, 5];
        rng1.shuffle(&mut a);
        rng2.shuffle(&mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn test_turn_seed_varies() {
        let s0 = turn_seed(12345, 0);
        let s1 = turn_seed(12345, 1);
        let s2 = turn_seed(12345, 2);
        assert_ne!(s0, s1);
        assert_ne!(s1, s2);
        assert_ne!(s0, s2);
    }

    #[test]
    fn test_turn_seed_zero_game_seed() {
        // The golden ratio pre-addition ensures seed 0, turn 0 doesn't map to 0
        let s = turn_seed(0, 0);
        assert!(s >= 1);
        assert!(s < MINSTD_M as i64);
    }

    #[test]
    fn test_rng_for_turn_deterministic() {
        let mut rng1 = rng_for_turn(12345, 42);
        let mut rng2 = rng_for_turn(12345, 42);
        let seq1: Vec<u64> = (0..10).map(|_| rng1.next()).collect();
        let seq2: Vec<u64> = (0..10).map(|_| rng2.next()).collect();
        assert_eq!(seq1, seq2);
    }

    // Cross-language verification: these exact values must match the Go implementation
    #[test]
    fn test_cross_language_vectors() {
        let mut rng = MinstdRand::new(1);
        let first_10: Vec<u64> = (0..10).map(|_| rng.next()).collect();
        assert_eq!(
            first_10,
            vec![
                48_271,
                182_605_794,
                1_291_394_886,
                1_914_720_637,
                2_078_669_041,
                407_355_683,
                1_105_902_161,
                854_716_505,
                564_586_691,
                1_596_680_831,
            ]
        );
    }
}
