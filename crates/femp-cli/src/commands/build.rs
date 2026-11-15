use anyhow::Result;

use crate::cli::BuildArgs;
use crate::console;
use crate::pipeline;
use crate::session::Session;

pub fn run(args: &BuildArgs) -> Result<bool> {
    let session = Session::open()?;
    let platforms = session.select(&args.target.targets)?;
    let cx = session.context(args.optimize);

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
