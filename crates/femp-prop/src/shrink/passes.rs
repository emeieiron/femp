use femp_core::property::U256;

use super::{Probe, ShrinkPass};
use crate::cases::{CLOCK_SPAN, VALUE_SPAN};
use crate::choices::{Span, Tape};

/// Whether a span is one value a run drew, rather than a part of the
/// case's structure such as a step.
fn is_value(span: &Span) -> bool {
    span.label == VALUE_SPAN || span.label == CLOCK_SPAN
}

/// Where each value the tape draws afresh holds its magnitude, in order:
/// its last choice, after the choice that says it repeats nothing.
fn magnitudes(tape: &Tape) -> Vec<usize> {
    let mut found: Vec<usize> = tape
        .spans
        .iter()
        .filter(|span| is_value(span) && span.end <= tape.choices.len())
        .filter(|span| match span.end - span.start {
            1 => true,
            2 => tape.choices[span.start] == U256::ZERO,
            _ => false,
        })
        .map(|span| span.end - 1)
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

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
            let mut spans: Vec<Span> = best
                .spans
                .iter()
                .filter(|span| !is_value(span))
                .cloned()
                .collect();
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
            if !is_value(&span)
                && best.choices[range.clone()]
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

/// Applies the tape's rewrites, which draw the same case with values that
/// repeat earlier ones drawn afresh instead: all at once, or else one at a
/// time. A value tied to another moves with it when either shrinks, and a
/// deleted step shifts which value it repeats; drawn afresh, each shrinks
/// on its own.
pub struct DrawFresh;

impl ShrinkPass for DrawFresh {
    fn name(&self) -> &'static str {
        "draw fresh"
    }

    fn run(&self, current: &Tape, probe: &mut Probe<'_>) -> Option<Tape> {
        if current.rewrites.is_empty() {
            return None;
        }
        let rewritten = |tape: &Tape, rewrites: &[crate::choices::Rewrite]| {
            let mut choices = tape.choices.clone();
            for rewrite in rewrites.iter().rev() {
                if rewrite.end <= choices.len() {
                    choices.splice(rewrite.start..rewrite.end, rewrite.choices.iter().copied());
                }
            }
            choices
        };
        if let Some(tape) = probe.attempt(rewritten(current, &current.rewrites), current) {
            return Some(tape);
        }
        let mut best = current.clone();
        let mut improved = false;
        let mut index = 0;
        while index < best.rewrites.len() && !probe.exhausted() {
            let rewrite = best.rewrites[index].clone();
            if let Some(tape) = probe.attempt(rewritten(&best, &[rewrite]), &best) {
                best = tape;
                improved = true;
            } else {
                index += 1;
            }
        }
        improved.then_some(best)
    }
}

/// Deletes a step and adds its clock advance to the next step's, so that
/// a step that only lets time pass gives way without changing when the
/// steps after it run. A clock drawn afresh ends with its value, which is
/// what it adds; one that repeats an earlier value is left alone.
pub struct CarryClock;

impl CarryClock {
    /// Where a step's clock holds its advance, when drawn afresh.
    fn advance(tape: &Tape, step: &Span) -> Option<usize> {
        let clock = tape.spans.iter().find(|span| {
            span.label == CLOCK_SPAN && span.start >= step.start && span.end <= step.end
        })?;
        let fresh = match clock.end - clock.start {
            1 => true,
            2 => tape.choices.get(clock.start) == Some(&U256::ZERO),
            _ => false,
        };
        (fresh && clock.end <= tape.choices.len()).then_some(clock.end - 1)
    }
}

impl ShrinkPass for CarryClock {
    fn name(&self) -> &'static str {
        "carry clock"
    }

    fn run(&self, current: &Tape, probe: &mut Probe<'_>) -> Option<Tape> {
        let mut best = current.clone();
        let mut improved = false;
        'restart: loop {
            let mut steps: Vec<Span> = best
                .spans
                .iter()
                .filter(|span| !is_value(span) && span.end > span.start)
                .cloned()
                .collect();
            steps.sort_by_key(|span| span.start);
            for pair in steps.windows(2) {
                let (step, next) = (&pair[0], &pair[1]);
                if probe.exhausted() {
                    break 'restart;
                }
                if next.start != step.end {
                    continue;
                }
                let (Some(from), Some(to)) = (Self::advance(&best, step), Self::advance(&best, next))
                else {
                    continue;
                };
                if best.choices[from] == U256::ZERO {
                    continue;
                }
                let mut choices = best.choices.clone();
                choices[to] = choices[to].saturating_add(choices[from]);
                choices.drain(step.start..step.end);
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

/// Lowers one value and every value drawn after it in proportion, rounding
/// up so that none becomes zero: the smallest value it can lower the first
/// to that still fails. A case whose values must keep their ratios, such
/// as amounts that add up past a total, shrinks in one step instead of
/// trading one value against another.
pub struct Rescale;

impl ShrinkPass for Rescale {
    fn name(&self) -> &'static str {
        "rescale"
    }

    fn run(&self, current: &Tape, probe: &mut Probe<'_>) -> Option<Tape> {
        let mut best = current.clone();
        let mut improved = false;
        let mut at = 0;
        loop {
            let places = magnitudes(&best);
            let Some(&first) = places.get(at) else {
                break;
            };
            at += 1;
            if probe.exhausted() {
                break;
            }
            let from = best.choices[first];
            let one = U256::from(1u8);
            if from <= one {
                continue;
            }
            let base = best.choices.clone();
            let later: Vec<usize> = places[at..].to_vec();
            let scaled = |to: U256| {
                let mut choices = base.clone();
                choices[first] = to;
                for index in &later {
                    let value = base[*index];
                    // ceil(value * to / from), unchanged where it overflows.
                    if let Some(product) = value.checked_mul(to) {
                        choices[*index] = product.div_ceil(from);
                    }
                }
                choices
            };
            if let Some(tape) = probe.attempt(scaled(one), &best) {
                best = tape;
                improved = true;
                continue;
            }
            // Scaling to `low` is known to pass and to `high` to fail.
            let (mut low, mut high) = (one, from);
            while high - low > one && !probe.exhausted() {
                let middle = low + ((high - low) >> 1);
                match probe.attempt(scaled(middle), &best) {
                    Some(tape) => {
                        best = tape;
                        improved = true;
                        high = middle;
                    }
                    None => low = middle,
                }
            }
        }
        improved.then_some(best)
    }
}
