//! Every generated case is a function of a tape of bounded choices. Random
//! generation records the tape; shrinking edits it and replays it, so a
//! smaller tape always regenerates a valid case through the same generators.

use std::cmp::Ordering;

use femp_core::property::U256;

use crate::rng::Rng;

/// A labelled range of choices, such as the choices of one machine step,
/// which shrink passes may delete as a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub label: &'static str,
    pub start: usize,
    pub end: usize,
}

/// Choices that draw the same value as the run of choices they replace,
/// such as a value drawn afresh in place of one that repeats an earlier
/// value. Shrinking applies them to untie values from one another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    pub start: usize,
    pub end: usize,
    pub choices: Vec<U256>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tape {
    pub choices: Vec<U256>,
    pub spans: Vec<Span>,
    pub rewrites: Vec<Rewrite>,
}

impl Tape {
    /// Shorter tapes are simpler; equally long ones compare choice by choice.
    pub fn simpler_than(&self, other: &Tape) -> bool {
        shortlex(&self.choices, &other.choices) == Ordering::Less
    }
}

pub fn shortlex(a: &[U256], b: &[U256]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

enum Source {
    Random(Rng),
    Replay { tape: Vec<U256>, position: usize },
}

pub struct Choices {
    source: Source,
    recorded: Vec<U256>,
    spans: Vec<Span>,
    open: Vec<(&'static str, usize)>,
    rewrites: Vec<Rewrite>,
}

impl Choices {
    pub fn random(seed: u64) -> Self {
        Self::new(Source::Random(Rng::new(seed)))
    }

    /// Replays `tape`; draws past its end read zero, the simplest choice.
    pub fn replay(tape: &[U256]) -> Self {
        Self::new(Source::Replay {
            tape: tape.to_vec(),
            position: 0,
        })
    }

    fn new(source: Source) -> Self {
        Self {
            source,
            recorded: Vec::new(),
            spans: Vec::new(),
            open: Vec::new(),
            rewrites: Vec::new(),
        }
    }

    /// Draws a choice in `[0, max]`. In random mode `random` picks it, and
    /// its result is clamped to `max`; replayed choices are clamped too.
    pub fn draw(&mut self, max: U256, random: impl FnOnce(&mut Rng) -> U256) -> U256 {
        let choice = match &mut self.source {
            Source::Random(rng) => random(rng),
            Source::Replay { tape, position } => {
                let choice = tape.get(*position).copied().unwrap_or(U256::ZERO);
                *position += 1;
                choice
            }
        }
        .min(max);
        self.recorded.push(choice);
        choice
    }

    pub fn draw_uniform(&mut self, max: U256) -> U256 {
        self.draw(max, |rng| rng.up_to(max))
    }

    pub fn draw_index(&mut self, count: usize, random: impl FnOnce(&mut Rng) -> u64) -> usize {
        let max = U256::from(count.saturating_sub(1) as u64);
        self.draw(max, |rng| U256::from(random(rng))).as_limbs()[0] as usize
    }

    pub fn start_span(&mut self, label: &'static str) {
        self.open.push((label, self.recorded.len()));
    }

    pub fn end_span(&mut self) {
        if let Some((label, start)) = self.open.pop() {
            self.spans.push(Span {
                label,
                start,
                end: self.recorded.len(),
            });
        }
    }

    /// How many choices have been drawn so far.
    pub fn position(&self) -> usize {
        self.recorded.len()
    }

    /// Notes that `choices` draw what the choices since `start` drew.
    pub fn rewrite(&mut self, start: usize, choices: Vec<U256>) {
        self.rewrites.push(Rewrite {
            start,
            end: self.recorded.len(),
            choices,
        });
    }

    pub fn into_tape(self) -> Tape {
        Tape {
            choices: self.recorded,
            spans: self.spans,
            rewrites: self.rewrites,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_reproduces_recorded_choices() {
        let mut random = Choices::random(5);
        let drawn: Vec<U256> = (0..4)
            .map(|_| random.draw_uniform(U256::from(99u8)))
            .collect();
        let tape = random.into_tape();
        assert_eq!(tape.choices, drawn);

        let mut replay = Choices::replay(&tape.choices);
        for expected in &drawn {
            assert_eq!(replay.draw_uniform(U256::from(99u8)), *expected);
        }
        assert_eq!(replay.draw_uniform(U256::from(99u8)), U256::ZERO);
    }

    #[test]
    fn replayed_choices_are_clamped() {
        let mut replay = Choices::replay(&[U256::from(500u16)]);
        assert_eq!(replay.draw_uniform(U256::from(9u8)), U256::from(9u8));
    }

    #[test]
    fn spans_cover_their_choices() {
        let mut choices = Choices::replay(&[]);
        choices.draw_uniform(U256::from(1u8));
        choices.start_span("step");
        choices.draw_uniform(U256::from(1u8));
        choices.draw_uniform(U256::from(1u8));
        choices.end_span();
        let tape = choices.into_tape();
        assert_eq!(
            tape.spans,
            [Span {
                label: "step",
                start: 1,
                end: 3
            }]
        );
    }

    #[test]
    fn shorter_tapes_are_simpler() {
        let one = U256::from(1u8);
        assert_eq!(shortlex(&[one], &[U256::ZERO, U256::ZERO]), Ordering::Less);
        assert_eq!(shortlex(&[U256::ZERO], &[one]), Ordering::Less);
        assert_eq!(shortlex(&[one], &[one]), Ordering::Equal);
    }
}
