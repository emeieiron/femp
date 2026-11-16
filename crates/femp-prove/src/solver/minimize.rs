//! Moves a counterexample toward the simplest inputs, one input at a time
//! in declaration order, by asking the solver for ever smaller bounds. The
//! order of simplicity is property testing's: integers by distance from
//! zero (clamped into their domain), positive first; `false` before `true`;
//! earlier `one_of` values first. So both tools report the same minimum.

use std::time::Duration;

use femp_core::property::{Ty, U256, Value};
use femp_spec::Domain;

use super::model::primitive;
use super::{Answer, Solver};
use crate::vc::ints;

pub struct Input<'a> {
    pub term: String,
    pub ty: Ty,
    pub domain: &'a Domain,
}

/// Fixes each input to its simplest value that still breaks the claim,
/// leaving those equalities asserted. The solver must be in a satisfiable
/// state; the caller pops what this pushes.
pub fn minimize(
    solver: &mut dyn Solver,
    inputs: &[Input<'_>],
    timeout: Duration,
) -> Result<Vec<Value>, String> {
    let mut chosen = Vec::new();
    for input in inputs {
        // A nested check discards the model, so ask for one again.
        if solver.check(timeout)? != Answer::Sat {
            return Err("the counterexample was lost while minimizing it".to_string());
        }
        let current = current(solver, input)?;
        let value = match input.domain {
            Domain::Const(value) => *value,
            Domain::OneOf(values) => {
                first_possible(solver, input, values, timeout)?.unwrap_or(current)
            }
            _ if input.ty == Ty::Bool => {
                let options = [Value::boolean(false), Value::boolean(true)];
                first_possible(solver, input, &options, timeout)?.unwrap_or(current)
            }
            domain => nearest_to_origin(solver, input, domain, current, timeout)?,
        };
        solver.assert(&format!("(= {} {})", input.term, ints::value(&value)))?;
        chosen.push(value);
    }
    // Leaves the solver with a model of the fixed inputs.
    if solver.check(timeout)? != Answer::Sat {
        return Err("the counterexample was lost while minimizing it".to_string());
    }
    Ok(chosen)
}

fn current(solver: &mut dyn Solver, input: &Input<'_>) -> Result<Value, String> {
    let values = solver.values(std::slice::from_ref(&input.term))?;
    values
        .first()
        .and_then(|sexp| primitive(sexp, input.ty))
        .ok_or_else(|| format!("the model has no value for `{}`", input.term))
}

fn possible(solver: &mut dyn Solver, condition: &str, timeout: Duration) -> Result<bool, String> {
    solver.push()?;
    solver.assert(condition)?;
    let answer = solver.check(timeout);
    solver.pop()?;
    Ok(answer? == Answer::Sat)
}

fn first_possible(
    solver: &mut dyn Solver,
    input: &Input<'_>,
    options: &[Value],
    timeout: Duration,
) -> Result<Option<Value>, String> {
    for option in options {
        let condition = format!("(= {} {})", input.term, ints::value(option));
        if possible(solver, &condition, timeout)? {
            return Ok(Some(*option));
        }
    }
    Ok(None)
}

fn nearest_to_origin(
    solver: &mut dyn Solver,
    input: &Input<'_>,
    domain: &Domain,
    current: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let ty = input.ty;
    let (lo, hi) = domain.hull(ty);
    let origin = ty.zero_ordinal().clamp(lo, hi);
    let ordinal = current.ordinal();
    let max = ty.max_ordinal();
    let within = |distance: U256| {
        let low = Value::from_ordinal(ty, origin.saturating_sub(distance));
        let high = Value::from_ordinal(ty, origin.saturating_add(distance).min(max));
        format!(
            "(and (<= {} {}) (<= {} {}))",
            ints::value(&low),
            input.term,
            input.term,
            ints::value(&high)
        )
    };
    let (mut low, mut high) = (U256::ZERO, ordinal.abs_diff(origin));
    while low < high {
        let middle = low + (high - low) / U256::from(2u8);
        if possible(solver, &within(middle), timeout)? {
            high = middle;
        } else {
            low = middle + U256::from(1u8);
        }
    }
    let above = Value::from_ordinal(ty, origin.saturating_add(low).min(max));
    let below = Value::from_ordinal(ty, origin.saturating_sub(low));
    let condition = format!("(= {} {})", input.term, ints::value(&above));
    if origin.checked_add(low).is_some_and(|sum| sum <= max)
        && possible(solver, &condition, timeout)?
    {
        Ok(above)
    } else if possible(
        solver,
        &format!("(= {} {})", input.term, ints::value(&below)),
        timeout,
    )? {
        Ok(below)
    } else {
        Ok(current)
    }
}
