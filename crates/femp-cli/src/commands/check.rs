use anyhow::Result;
use femp_core::environment::OptLevel;

use crate::cli::TargetArgs;
use crate::console;
use crate::session::Session;

pub fn run(args: &TargetArgs) -> Result<bool> {
    let session = Session::open()?;
    let platforms = session.select(&args.targets)?;
    let inputs = session.prepare(&platforms, true)?;
    let cx = inputs.context(OptLevel::default());

    let mut passed = true;
    for platform in platforms {
        let modules = cx
            .project
            .shared_modules()
            .chain(cx.project.modules_for(platform.id()));
        for module in modules {
            console::step("Checking", &format!("{} [{}]", module.name, platform.id()));
            if let Err(error) = platform.check(&cx, module) {
                console::error(&module.compiler_error(error).to_string());
                passed = false;
            }
        }
    }
    Ok(passed)
}
