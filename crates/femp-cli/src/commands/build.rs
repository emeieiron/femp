use anyhow::{Result, bail};

use crate::cli::BuildArgs;
use crate::console;
use crate::pipeline;
use crate::session::Session;

pub fn run(args: &BuildArgs) -> Result<bool> {
    let session = Session::open()?;
    let platforms = session.select(&args.target.targets)?;
    if args.wasm_stack_size.is_some()
        && !platforms
            .iter()
            .any(|platform| platform.supports_stack_size())
    {
        bail!("`--wasm-stack-size` requires the `wasm` target");
    }
    let inputs = session.prepare(&platforms, false)?;
    let cx = inputs.context_with(args.optimize, args.wasm_stack_size);

    let built = pipeline::build_all(&cx, &platforms)?;
    for item in &built {
        console::step(
            "Built",
            &format!("{} [{}]", item.module.name, item.platform.id()),
        );
        for artifact in &item.artifacts {
            console::detail(&format!(
                "{:<16} {}",
                artifact.kind,
                console::relative(&artifact.path, &session.project.root)
            ));
        }
    }
    Ok(true)
}
