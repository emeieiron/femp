//! Property harnesses for the EVM: a generated contract run in an
//! in-process EVM.

mod exec;
mod render;

use std::fs;

use femp_core::artifact::of_kind;
use femp_core::platform::{Context, PlatformId};
use femp_core::property::harness_ingot::{HarnessIngot, write_harness_ingot};
use femp_core::property::{Executor, HarnessRequest};
use femp_core::{Error, Result};

use crate::build::{self, BYTECODE, contract_of};

pub use exec::EvmExecutor;

pub fn start(
    cx: &Context<'_>,
    platform: &PlatformId,
    request: &HarnessRequest<'_>,
) -> Result<Box<dyn Executor>> {
    let suite = request.suite;
    let name = format!("{}_evm_harness", suite.ingot);
    let module = write_harness_ingot(
        &request.out_dir.join("ingot"),
        platform,
        &HarnessIngot {
            name: &name,
            target: "evm",
            dependencies: vec![(suite.ingot.clone(), request.subject.dir.clone())],
            source: render::render(suite),
        },
    )?;
    let built = build::build(cx, &module, &request.out_dir.join("bin"))?;
    let bytecode = of_kind(&built, BYTECODE)
        .find(|artifact| contract_of(artifact) == render::CONTRACT)
        .ok_or_else(|| Error::Artifact("the EVM harness build produced no bytecode".to_string()))?;
    let hex = fs::read_to_string(&bytecode.path)
        .map_err(|source| Error::io("read", &bytecode.path, source))?;
    Ok(Box::new(EvmExecutor::deploy(&hex, suite.clone())?))
}
