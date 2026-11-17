//! A sequence of transactions sent to a deployed contract, as a refutation
//! of its storage invariants describes it, so that a platform can run it
//! on the compiled contract.

use crate::error::Result;

use super::outcome::Outcome;
use super::value::Value;

/// A module's compiled contracts, built once, which run any number of
/// sequences of transactions, each against fresh deployments.
pub trait Deployer: Send {
    fn run(&mut self, transactions: &Transactions) -> Result<ContractRun>;
}

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
    /// The storage slots of the contract to read after each transaction.
    pub reads: Vec<[u8; 32]>,
}

/// What became of each transaction, and the contract's storage after each
/// one that ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRun {
    pub deliveries: Vec<Delivery>,
    /// For each transaction, the word at each slot of `reads` after it.
    pub storage: Vec<Vec<[u8; 32]>>,
}

/// What became of one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Accepted,
    /// The transaction did not change the contract, and why.
    Rejected(Outcome),
}
