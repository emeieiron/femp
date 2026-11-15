//! The data shared by the property-testing engine and the platform
//! harnesses: primitive types and values, normalized outcomes, the entry
//! points of a suite, and the executor interface each platform implements.

pub mod executor;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod harness_ingot;
pub mod outcome;
pub mod suite;
pub mod ty;
pub mod value;

pub use executor::{
    Executor, Halt, HarnessRequest, Record, Sequence, Step, StepStatus, Support, Trace,
};
pub use outcome::{Outcome, PanicKind, Trap};
pub use suite::{
    Command, Effect, Location, Machine, Method, Observation, Param, Property, PropertyKind, Suite,
};
pub use ty::Ty;
pub use value::Value;

pub use ruint::aliases::U256;
