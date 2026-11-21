//! EVM tests: every selected test of an ingot is compiled into one contract,
//! deployed once, and called with its 4-byte selector in an isolated instance
//! of that deployment.

use std::time::Instant;

use codegen::{SonatinaTestOptions, TestMetadata};
use contract_harness::{ExecutionOptions, HarnessError, RuntimeInstance, RuntimeSnapshot, U256};
use hir::hir_def::HirIngot;

use super::{Outcome, TestOptions, failed, passed, revert_mismatch};
use crate::error::CompileError;
use crate::guard::{generate, opt_level};
use crate::session::Session;

pub(super) fn run(
    session: &Session,
    options: &TestOptions<'_>,
) -> Result<Vec<Outcome>, CompileError> {
    let db = &session.db;
    let mut top_mods = session.ingot().all_modules(db).to_vec();
    top_mods.sort_by(|left, right| left.name(db).cmp(&right.name(db)));
    let level = opt_level(options.optimize);
    let suite = generate("Sonatina", || {
        codegen::emit_test_suite_sonatina(
            db,
            &top_mods,
            level,
            SonatinaTestOptions::default(),
            options.filter,
        )
    })?;
    let Some(suite) = suite else {
        return Ok(Vec::new());
    };
    let deployed = RuntimeInstance::deploy(&hex::encode(&suite.bytecode))
        .map_err(|error| CompileError::Failed(format!("Failed to deploy the test suite: {error}")))?
        .snapshot();
    Ok(suite
        .tests
        .iter()
        .map(|case| run_case(&deployed, case))
        .collect())
}

fn run_case(deployed: &RuntimeSnapshot, case: &TestMetadata) -> Outcome {
    let started = Instant::now();
    let name = case.name.as_str();
    let fail = |message: String| failed(name, message, started.elapsed());
    let balance = match case.initial_balance.as_deref().map(balance).transpose() {
        Ok(balance) => balance,
        Err(error) => {
            return fail(format!(
                "invalid initial balance for test `{name}`: {error}"
            ));
        }
    };
    let mut instance = deployed.instance();
    if let Some(balance) = balance {
        instance.fund_contract(balance);
    }
    let calldata = case.selector.to_be_bytes();
    let called = instance
        .call_raw(&calldata, ExecutionOptions::default())
        .map(drop);
    let message = match (called, &case.expected_revert) {
        (Ok(()), None) => None,
        (Err(error), None) => Some(describe(error)),
        (Ok(()), Some(_)) => Some("Expected test to revert, but it succeeded".to_string()),
        (Err(HarnessError::Revert(data)), Some(expected)) => revert_mismatch(expected, &data.0),
        (Err(error), Some(_)) => Some(format!(
            "Expected test to revert, but it failed with: {}",
            describe(error)
        )),
    };
    match message {
        None => passed(name, started.elapsed()),
        Some(message) => fail(message),
    }
}

/// A big-endian balance of at most 32 bytes.
fn balance(bytes: &[u8]) -> Result<U256, &'static str> {
    if bytes.len() > 32 {
        return Err("initial balance exceeds u256");
    }
    let mut word = [0u8; 32];
    word[32 - bytes.len()..].copy_from_slice(bytes);
    Ok(U256::from_be_bytes(word))
}

fn describe(error: HarnessError) -> String {
    match error {
        HarnessError::Revert(data) => format!("Test reverted: {data}"),
        HarnessError::Halted { reason, gas_used } => {
            format!("Test halted: {reason:?} (gas: {gas_used})")
        }
        other => format!("Test execution error: {other}"),
    }
}
