//! Property harnesses for native executables: a generated program that
//! reads cases from standard input and prints each result.

mod exec;
mod render;

use femp_core::Result;
use femp_core::artifact::of_kind;
use femp_core::platform::{Context, PlatformId};
use femp_core::property::harness_ingot::{HarnessIngot, write_harness_ingot};
use femp_core::property::{Executor, HarnessRequest};

use crate::build::{self, EXECUTABLE};

pub use exec::NativeExecutor;
pub use render::render;

pub fn start(
    cx: &Context<'_>,
    platform: &PlatformId,
    request: &HarnessRequest<'_>,
) -> Result<Box<dyn Executor>> {
    let suite = request.suite;
    let name = format!("{}_native_harness", suite.ingot);
    let module = write_harness_ingot(
        &request.out_dir.join("ingot"),
        platform,
        &HarnessIngot {
            name: &name,
            target: "native",
            dependencies: vec![(suite.ingot.clone(), request.subject.dir.clone())],
            source: render(suite),
        },
    )?;
    let built = build::build(cx, &module, &request.out_dir.join("bin"))?;
    let executable = of_kind(&built, EXECUTABLE)
        .next()
        .map(|artifact| artifact.path.clone())
        .expect("a native build produces an executable");
    Ok(Box::new(NativeExecutor::new(
        executable,
        suite.clone(),
        request.timeout,
    )))
}
