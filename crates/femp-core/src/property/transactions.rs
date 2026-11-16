//! A sequence of transactions sent to a deployed contract, as a refutation
//! of its storage invariants describes it, so that a platform can run it
//! on the compiled contract.

use super::outcome::Outcome;
use super::value::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    /// The selector of the message, or `None` for the deployment.
    pub selector: Option<u32>,
    /// The message's name, for reports.
    pub name: String,
    pub args: Vec<Value>,
    /// The context the transaction runs in, by the names reports give it,
    /// such as `ctx.timestamp`.
    pub context: Vec<(String, Value)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transactions {
    /// The contract to deploy.
    pub contract: String,
    pub deploy: Transaction,
    pub calls: Vec<Transaction>,
}

/// What became of one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Accepted,
    /// The transaction did not change the contract, and why.
    Rejected(Outcome),
}
