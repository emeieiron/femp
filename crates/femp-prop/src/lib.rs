//! The host side of Fe property testing: generating cases from seeds,
//! running them on every target through [`femp_core::property::Executor`]s,
//! comparing the outcomes, and shrinking failures to minimal counterexamples.
//! Nothing here spawns processes or knows about a particular platform.

pub mod cases;
pub mod choices;
pub mod compare;
pub mod generators;
pub mod rng;
pub mod shrink;
pub mod spec;
pub mod verdict;
