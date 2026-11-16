//! EVM tests: each test is a contract, deployed and called once in an
//! in-process EVM.

use std::time::Instant;

use codegen::{SonatinaTestOptions, TestMetadata};
use contract_harness::{ExecutionOptions, HarnessError, RuntimeInstance, U256};

use super::{Outcome, TestOptions, failed, passed, revert_mismatch};
use crate::error::CompileError;
use crate::guard::{generate, opt_level};
use crate::session::Session;

pub(super) fn run(
    session: &Session,
    options: &TestOptions<'_>,
) -> Result<Vec<Outcome>, CompileError> {
    let db = &session.db;
    let ingot = session.ingot();
    let level = opt_level(options.optimize);
    let output = generate("Sonatina", || {
        codegen::emit_test_ingot_sonatina(
            db,
            ingot,
            level,
            SonatinaTestOptions::default(),
            options.filter,
        )
    })?;
    Ok(output
        .tests
        .iter()
        .filter(|case| matches(case, options.filter))
        .map(run_case)
        .collect())
}

fn matches(case: &TestMetadata, filter: Option<&str>) -> bool {
    filter.is_none_or(|pattern| {
        case.hir_name.contains(pattern)
            || case.symbol_name.contains(pattern)
            || case.display_name.contains(pattern)
    })
}

fn run_case(case: &TestMetadata) -> Outcome {
    let started = Instant::now();
    let name = case.display_name.as_str();
    let fail = |message: String| failed(name, message, started.elapsed());
    if case.value_param_count > 0 {
        return fail(format!(
            "tests with value parameters are not supported (found {})",
            case.value_param_count
        ));
    }
    if case.object_name.trim().is_empty() {
        return fail(format!("missing test object name for `{name}`"));
    }
    let balance = match case.initial_balance.as_deref().map(balance).transpose() {
        Ok(balance) => balance,
        Err(error) => {
            return fail(format!(
                "invalid initial balance for test `{name}`: {error}"
            ));
        }
    };
    if case.bytecode.is_empty() {
        return fail(format!("missing test bytecode for `{name}`"));
    }
    let mut instance = match RuntimeInstance::deploy_tracked(&hex::encode(&case.bytecode)) {
        Ok((instance, _)) => instance,
        Err(error) => return fail(format!("Failed to deploy test: {error}")),
    };
    if let Some(balance) = balance {
        instance.fund_contract(balance);
    }
    // Test functions take no arguments, so the call carries no data.
    let called = instance
        .call_raw(&[], ExecutionOptions::default())
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
