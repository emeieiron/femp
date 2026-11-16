//! Calls into the specification library, `femp_spec`, which the prover
//! reads as logic rather than as code.

use femp_vir::SpecOp;

pub const SPEC_INGOT: &str = "femp_spec";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpecCall {
    Statement(SpecOp),
    Implies,
}

pub(crate) fn spec_call(name: &str) -> Option<SpecCall> {
    Some(match name {
        "requires" => SpecCall::Statement(SpecOp::Requires),
        "ensures" => SpecCall::Statement(SpecOp::Ensures),
        "invariant" => SpecCall::Statement(SpecOp::Invariant),
        "implies" => SpecCall::Implies,
        _ => return None,
    })
}
