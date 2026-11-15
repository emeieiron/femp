use femp_core::Result;
use femp_core::outcome::TestRun;
use femp_core::platform::{Context, PlatformId};
use femp_core::project::Module;

use crate::invoke::{self, TestRequest};
use crate::test_output;

pub fn check(cx: &Context<'_>, backend: &str, module: &Module) -> Result<()> {
    invoke::check(&cx.environment.toolchain, backend, &module.dir)
}

pub fn test(
    cx: &Context<'_>,
    backend: &str,
    platform: &PlatformId,
    module: &Module,
    filter: Option<&str>,
) -> Result<TestRun> {
    let mut captured = invoke::test(
        &cx.environment.toolchain,
        &TestRequest {
            backend,
            input: &module.dir,
            filter,
            optimize: cx.optimize,
        },
    )?;
    captured.stderr = module.diagnostics(&captured.stderr);
    captured.stdout = module.diagnostics(&captured.stdout);
    Ok(test_output::into_run(&module.name, platform, &captured))
}
