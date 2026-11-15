//! A small deterministic generator, written here so that a seed keeps
//! producing the same cases whatever the dependencies do.

use femp_core::property::U256;

const GOLDEN_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A uniform value in `[0, max]`.
    pub fn up_to(&mut self, max: U256) -> U256 {
        if max == U256::ZERO {
            return U256::ZERO;
        }
        let bits = max.bit_len();
        loop {
            let limbs = [
                self.next_u64(),
                self.next_u64(),
                self.next_u64(),
                self.next_u64(),
            ];
            let candidate = U256::from_limbs(limbs) >> (256 - bits);
            if candidate <= max {
                return candidate;
            }
        }
    }

    /// A value in `[0, max]` whose bit length is uniform, so small and
    /// large magnitudes are drawn equally often.
    pub fn log_uniform(&mut self, max: U256) -> U256 {
        let bits = self.below_u64(max.bit_len() as u64 + 1) as usize;
        let ceiling = match bits {
            0 => U256::ZERO,
            256 => U256::MAX,
            bits => (U256::from(1u8) << bits) - U256::from(1u8),
        };
        self.up_to(ceiling.min(max))
    }

    /// A uniform value in `[0, bound)`; `bound` must be positive.
    pub fn below_u64(&mut self, bound: u64) -> u64 {
        self.up_to(U256::from(bound - 1)).as_limbs()[0]
    }

    pub fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        self.below_u64(denominator) < numerator
    }
}

/// FNV-1a, used to derive a stable per-entry seed from its name.
pub fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ byte as u64).wrapping_mul(FNV_PRIME)
    })
}

/// Mixes a seed with a salt into an independent seed.
pub fn derive(seed: u64, salt: u64) -> u64 {
    Rng::new(seed ^ salt.rotate_left(29)).next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stream_is_stable() {
        let mut rng = Rng::new(1234);
        let first: Vec<u64> = (0..3).map(|_| rng.next_u64()).collect();
        assert_eq!(
            first,
            [
                13478418381427711195,
                10936887474700444964,
                3728693401281897946
            ]
        );
        assert_eq!(fnv1a("core::claims"), fnv1a("core::claims"));
        assert_ne!(derive(7, fnv1a("a")), derive(7, fnv1a("b")));
    }

    #[test]
    fn bounded_draws_stay_in_range() {
        let mut rng = Rng::new(9);
        let max = U256::from(1000u32);
        for _ in 0..2000 {
            assert!(rng.up_to(max) <= max);
            assert!(rng.log_uniform(max) <= max);
            assert!(rng.below_u64(3) < 3);
        }
        assert_eq!(rng.up_to(U256::ZERO), U256::ZERO);
        rng.up_to(U256::MAX);
        rng.log_uniform(U256::MAX);
    }

    #[test]
    fn log_uniform_reaches_small_and_large_values() {
        let mut rng = Rng::new(3);
        let draws: Vec<U256> = (0..400).map(|_| rng.log_uniform(U256::MAX)).collect();
        assert!(draws.iter().any(|value| *value < U256::from(256u32)));
        assert!(draws.iter().any(|value| value.bit_len() > 200));
    }
}
