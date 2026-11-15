use femp_core::property::{Ty, U256, Value};

use super::Generator;
use crate::choices::Choices;

const EDGE_CHANCE: (u64, u64) = (1, 4);
const POWERS: [usize; 6] = [7, 8, 16, 32, 64, 128];

/// Integers in an inclusive range, kept as ordinals so that signed and
/// unsigned types share one implementation.
///
/// A draw is one choice, a rank: rank 0 is the value closest to zero and
/// ranks alternate outward from it, so shrinking a rank moves the value
/// toward zero while staying in range. Random draws favour the ranks of edge
/// values, such as the bounds and powers of two, and small magnitudes.
#[derive(Debug)]
pub struct IntRange {
    ty: Ty,
    lo: U256,
    hi: U256,
    origin: U256,
    edges: Vec<U256>,
}

impl IntRange {
    pub fn new(ty: Ty, lo: U256, hi: U256) -> Self {
        let origin = ty.zero_ordinal().clamp(lo, hi);
        let mut edges: Vec<U256> = interesting(ty)
            .into_iter()
            .chain([
                lo,
                hi,
                lo.saturating_add(U256::from(1u8)),
                hi.saturating_sub(U256::from(1u8)),
            ])
            .filter(|ordinal| (lo..=hi).contains(ordinal))
            .collect();
        edges.sort();
        edges.dedup();
        Self {
            ty,
            lo,
            hi,
            origin,
            edges,
        }
    }

    pub fn full(ty: Ty) -> Self {
        Self::new(ty, U256::ZERO, ty.max_ordinal())
    }

    fn span(&self) -> U256 {
        self.hi - self.lo
    }

    fn at_rank(&self, rank: U256) -> Value {
        let up = self.hi - self.origin;
        let down = self.origin - self.lo;
        let near = up.min(down);
        let one = U256::from(1u8);
        let ordinal = if rank <= near.saturating_mul(U256::from(2u8)) {
            if rank == U256::ZERO {
                self.origin
            } else if rank.bit(0) {
                self.origin + ((rank + one) >> 1)
            } else {
                self.origin - (rank >> 1)
            }
        } else {
            let beyond = rank - near - near;
            if up > down {
                self.origin + near + beyond
            } else {
                self.origin - near - beyond
            }
        };
        Value::from_ordinal(self.ty, ordinal)
    }

    /// The inverse of `at_rank`.
    fn rank_of(&self, ordinal: U256) -> U256 {
        let up = self.hi - self.origin;
        let down = self.origin - self.lo;
        let near = up.min(down);
        let distance = ordinal.abs_diff(self.origin);
        if distance == U256::ZERO {
            U256::ZERO
        } else if distance <= near {
            let doubled = distance << 1;
            if ordinal > self.origin {
                doubled - U256::from(1u8)
            } else {
                doubled
            }
        } else {
            near + near + (distance - near)
        }
    }
}

impl Generator for IntRange {
    fn ty(&self) -> Ty {
        self.ty
    }

    fn draw(&self, choices: &mut Choices) -> Value {
        let span = self.span();
        let rank = choices.draw(span, |rng| {
            if !self.edges.is_empty() && rng.chance(EDGE_CHANCE.0, EDGE_CHANCE.1) {
                let edge = self.edges[rng.below_u64(self.edges.len() as u64) as usize];
                self.rank_of(edge)
            } else {
                rng.log_uniform(span)
            }
        });
        self.at_rank(rank)
    }

    fn domain_size(&self) -> Option<u64> {
        let span = self.span();
        (span < U256::from(u64::MAX)).then(|| span.as_limbs()[0] + 1)
    }

    fn choices_for(&self, rank: u64) -> Vec<U256> {
        vec![U256::from(rank)]
    }
}

/// Ordinals of the values where integer code tends to break: zero and its
/// neighbours, the type's limits, and powers of two with their neighbours.
fn interesting(ty: Ty) -> Vec<U256> {
    let one = U256::from(1u8);
    let zero = ty.zero_ordinal();
    let mut magnitudes = vec![U256::ZERO, one];
    for power in POWERS.iter().filter(|power| **power < ty.bits() as usize) {
        let value = one << *power;
        magnitudes.extend([value - one, value, value + one]);
    }
    let mut ordinals = vec![U256::ZERO, ty.max_ordinal()];
    for magnitude in magnitudes {
        if let Some(up) = zero.checked_add(magnitude) {
            ordinals.push(up);
        }
        if ty.is_signed()
            && let Some(down) = zero.checked_sub(magnitude)
        {
            ordinals.push(down);
        }
    }
    ordinals.retain(|ordinal| *ordinal <= ty.max_ordinal());
    ordinals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ordinal(ty: Ty, text: &str) -> U256 {
        Value::parse(ty, text).unwrap().ordinal()
    }

    fn ranked(range: &IntRange, count: u64) -> Vec<String> {
        (0..count)
            .map(|rank| range.at_rank(U256::from(rank)).to_string())
            .collect()
    }

    #[test]
    fn ranks_alternate_outward_from_zero() {
        let ty = Ty::Int(8);
        let range = IntRange::new(ty, ordinal(ty, "-2"), ordinal(ty, "4"));
        assert_eq!(ranked(&range, 7), ["0", "1", "-1", "2", "-2", "3", "4"]);

        let range = IntRange::new(ty, ordinal(ty, "-5"), ordinal(ty, "1"));
        assert_eq!(ranked(&range, 7), ["0", "1", "-1", "-2", "-3", "-4", "-5"]);
    }

    #[test]
    fn rank_of_inverts_at_rank() {
        let ty = Ty::Int(8);
        for (lo, hi) in [("-2", "4"), ("-5", "1"), ("-128", "127"), ("3", "9")] {
            let range = IntRange::new(ty, ordinal(ty, lo), ordinal(ty, hi));
            for rank in 0..=range.span().as_limbs()[0] {
                let rank = U256::from(rank);
                assert_eq!(range.rank_of(range.at_rank(rank).ordinal()), rank);
            }
        }
    }

    #[test]
    fn unsigned_ranks_count_up_from_the_low_bound() {
        let ty = Ty::Uint(16);
        let range = IntRange::new(ty, U256::from(10u8), U256::from(13u8));
        assert_eq!(ranked(&range, 4), ["10", "11", "12", "13"]);
        assert_eq!(range.domain_size(), Some(4));
    }

    #[test]
    fn draws_stay_in_range_and_hit_edges() {
        let ty = Ty::Uint(64);
        let range = IntRange::new(ty, U256::from(5u8), U256::from(1_000_000u32));
        let mut choices = Choices::random(11);
        let drawn: Vec<Value> = (0..500).map(|_| range.draw(&mut choices)).collect();
        assert!(drawn.iter().all(|value| {
            let ordinal = value.ordinal();
            ordinal >= U256::from(5u8) && ordinal <= U256::from(1_000_000u32)
        }));
        for edge in ["5", "1000000", "256"] {
            assert!(
                drawn.iter().any(|value| value.to_string() == edge),
                "{edge}"
            );
        }
    }

    #[test]
    fn the_zero_tape_is_the_simplest_value() {
        let ty = Ty::Int(64);
        let range = IntRange::full(ty);
        let mut choices = Choices::replay(&[]);
        assert_eq!(range.draw(&mut choices).to_string(), "0");
        assert_eq!(range.domain_size(), None);

        let mut choices = Choices::replay(&range.choices_for(4));
        assert_eq!(range.draw(&mut choices).to_string(), "-2");
    }

    #[test]
    fn full_width_ranges_work() {
        for ty in [Ty::Uint(256), Ty::Int(256)] {
            let range = IntRange::full(ty);
            let mut choices = Choices::random(2);
            for _ in 0..50 {
                range.draw(&mut choices);
            }
            assert_eq!(range.at_rank(U256::MAX).ty(), ty);
        }
    }
}
