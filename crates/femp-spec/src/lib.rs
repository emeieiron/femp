//! What a module claims, shared by the tools that check the claims:
//! `femp prop` tests them with generated inputs and `femp prove` proves them.
//! Both read the same declarations, interpret `#[gen(..)]` the same way and
//! exchange counterexamples in one format.

pub mod claims;
pub mod counterexample;
pub mod discover;
pub mod domain;

pub use claims::{
    Claims, Contract, ContractMode, SpecParam, StorageClaim, StorageInvariant, TargetRef,
};
pub use counterexample::{Arg, Counterexample, StepReport};
pub use discover::{DiscoveryError, discover};
pub use domain::Domain;
