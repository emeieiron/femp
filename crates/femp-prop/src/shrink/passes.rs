use femp_core::property::U256;

use super::{Probe, ShrinkPass};
use crate::choices::Tape;

/// Deletes labelled spans, largest first: for machines, whole steps.
pub struct DeleteSpans;

impl ShrinkPass for DeleteSpans {
    fn name(&self) -> &'static str {
        "delete spans"
    }

    fn run(&self, current: &Tape, probe: &mut Probe<'_>) -> Option<Tape> {
        let mut best = current.clone();
        let mut improved = false;
        'restart: loop {
            let mut spans = best.spans.clone();
            spans.sort_by_key(|span| (std::cmp::Reverse(span.end - span.start), span.start));
            for span in spans.iter().filter(|span| span.end > span.start) {
                let mut choices = best.choices.clone();
                choices.drain(span.start..span.end);
                if let Some(tape) = probe.attempt(choices, &best) {
                    best = tape;
                    improved = true;
                    continue 'restart;
                }
            }
            break;
        }
        improved.then_some(best)
    }
}

/// Sets every choice of a span to zero at once.
pub struct ZeroSpans;

impl ShrinkPass for ZeroSpans {
    fn name(&self) -> &'static str {
        "zero spans"
    }

    fn run(&self, current: &Tape, probe: &mut Probe<'_>) -> Option<Tape> {
        let mut best = current.clone();
        let mut improved = false;
        let mut index = 0;
        while index < best.spans.len() {
            let span = best.spans[index].clone();
            let range = span.start..span.end.min(best.choices.len());
            if best.choices[range.clone()]
                .iter()
                .any(|choice| *choice != U256::ZERO)
            {
                let mut choices = best.choices.clone();
                choices[range].fill(U256::ZERO);
                if let Some(tape) = probe.attempt(choices, &best) {
                    best = tape;
                    improved = true;
                }
            }
            index += 1;
        }
        improved.then_some(best)
    }
}

/// Lowers each choice to the smallest value that still fails, trying zero
/// first and then a binary search.
pub struct MinimizeChoices;

impl ShrinkPass for MinimizeChoices {
    fn name(&self) -> &'static str {
        "minimize choices"
    }

    fn run(&self, current: &Tape, probe: &mut Probe<'_>) -> Option<Tape> {
        let mut best = current.clone();
        let mut improved = false;
        let mut index = 0;
        while index < best.choices.len() && !probe.exhausted() {
            let value = best.choices[index];
            if value != U256::ZERO {
                let with = |best: &Tape, choice: U256| {
                    let mut choices = best.choices.clone();
                    choices[index] = choice;
                    choices
                };
                if let Some(tape) = probe.attempt(with(&best, U256::ZERO), &best) {
                    best = tape;
                    improved = true;
                } else {
                    // `low` is known to pass and `high` to fail.
                    let mut low = U256::ZERO;
                    let mut high = value;
                    while high - low > U256::from(1u8) && index < best.choices.len() {
                        let middle = low + ((high - low) >> 1);
                        match probe.attempt(with(&best, middle), &best) {
                            Some(tape) => {
                                best = tape;
                                improved = true;
                                high = middle;
                            }
                            None if probe.exhausted() => break,
                            None => low = middle,
                        }
                    }
                }
            }
            index += 1;
        }
        improved.then_some(best)
    }
}

/// Removes single choices, which realigns the rest of the tape.
pub struct DeleteChoices;

impl ShrinkPass for DeleteChoices {
    fn name(&self) -> &'static str {
        "delete choices"
    }

    fn run(&self, current: &Tape, probe: &mut Probe<'_>) -> Option<Tape> {
        let mut best = current.clone();
        let mut improved = false;
        let mut index = best.choices.len();
        while index > 0 && !probe.exhausted() {
            index -= 1;
            if index >= best.choices.len() {
                continue;
            }
            let mut choices = best.choices.clone();
            choices.remove(index);
            if let Some(tape) = probe.attempt(choices, &best) {
                best = tape;
                improved = true;
            }
        }
        improved.then_some(best)
    }
}

/// Moves magnitude from a choice to one of the next few, which reaches
/// minima that need two choices to change together, such as `a + b > n`.
pub struct Redistribute;

const REACH: usize = 3;

impl ShrinkPass for Redistribute {
    fn name(&self) -> &'static str {
        "redistribute"
    }

    fn run(&self, current: &Tape, probe: &mut Probe<'_>) -> Option<Tape> {
        let mut best = current.clone();
        let mut improved = false;
        let mut from = 0;
        while from < best.choices.len() && !probe.exhausted() {
            for to in from + 1..(from + 1 + REACH).min(best.choices.len()) {
                let amount = best.choices[from];
                if amount == U256::ZERO || to >= best.choices.len() {
                    break;
                }
                let moved = |best: &Tape, by: U256| {
                    let mut choices = best.choices.clone();
                    if to < choices.len() {
                        let by = by.min(choices[from]);
                        choices[from] -= by;
                        choices[to] = choices[to].saturating_add(by);
                    }
                    choices
                };
                if let Some(tape) = probe.attempt(moved(&best, amount), &best) {
                    best = tape;
                    improved = true;
                    continue;
                }
                // Moving all of `amount` fails to reproduce; halve the move
                // until one works, then try to move more of what is left.
                let mut limit = amount;
                while limit > U256::from(1u8) && !probe.exhausted() {
                    let by = limit >> 1;
                    match probe.attempt(moved(&best, by), &best) {
                        Some(tape) => {
                            best = tape;
                            improved = true;
                            limit -= by;
                            if to >= best.choices.len() {
                                break;
                            }
                        }
                        None => limit = by,
                    }
                }
            }
            from += 1;
        }
        improved.then_some(best)
    }
}
