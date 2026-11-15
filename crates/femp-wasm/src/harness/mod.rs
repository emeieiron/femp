//! Property harnesses for WebAssembly: a generated module run by the
//! Node.js runner shipped with the host runtime.

mod exec;
mod render;

use femp_core::artifact::{of_kind, write_file};
use femp_core::platform::{Context, PlatformId};
use femp_core::property::harness_ingot::{HarnessIngot, write_harness_ingot};
use femp_core::property::{Executor, HarnessRequest};
use femp_core::{Error, Result};

use crate::bindings::{HOST_SCRIPT, HOST_SCRIPT_FILE};
use crate::build::{self, MANIFEST, MODULE};

pub use exec::WasmExecutor;

const RUNNER_SCRIPT: &str = include_str!("../../../../runtime/wasm-host/harness.mjs");
const RUNNER_FILE: &str = "harness.mjs";
const ENTRIES_FILE: &str = "entries.json";

pub fn start(
    cx: &Context<'_>,
    platform: &PlatformId,
    backend: &str,
    request: &HarnessRequest<'_>,
) -> Result<Box<dyn Executor>> {
    let suite = request.suite;
    let name = format!("{}_wasm_harness", suite.ingot);
    let module = write_harness_ingot(
        &request.out_dir.join("ingot"),
        platform,
        &HarnessIngot {
            name: &name,
            target: "wasm",
            dependencies: vec![(suite.ingot.clone(), request.subject.dir.clone())],
            source: render::render(suite),
        },
    )?;
    let out_dir = request.out_dir.join("bin");
    let mut project = cx.project.clone();
    project.modules.push(module.clone());
    let staged = Context {
        project: &project,
        environment: cx.environment,
        optimize: cx.optimize,
    };
    let built = build::build(&staged, backend, &module, &out_dir)?;
    let path_of = |kind| {
        of_kind(&built, kind)
            .next()
            .map(|artifact| artifact.path.clone())
            .ok_or_else(|| {
                Error::Artifact(format!("the WebAssembly harness build produced no {kind}"))
            })
    };
    let wasm = path_of(MODULE)?;
    let manifest = path_of(MANIFEST)?;

    let runner = out_dir.join(RUNNER_FILE);
    write_file(&runner, RUNNER_SCRIPT)?;
    write_file(&out_dir.join(HOST_SCRIPT_FILE), HOST_SCRIPT)?;
    let entries = out_dir.join(ENTRIES_FILE);
    write_file(
        &entries,
        serde_json::to_string_pretty(&render::entries(suite)).unwrap_or_default(),
    )?;
    Ok(Box::new(WasmExecutor::new(
        &runner,
        &wasm,
        &manifest,
        &entries,
        suite.clone(),
        request.timeout,
    )))
}
