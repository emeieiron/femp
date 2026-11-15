use anyhow::Result;
use femp_core::artifact::{Artifact, reset_dir, sha256_hex, write_file};
use femp_core::project::DIST_DIR;
use serde_json::{Value, json};

use crate::cli::BuildArgs;
use crate::console;
use crate::pipeline::{self, BuiltModule};
use crate::session::Session;

const INDEX_FILE: &str = "manifest.json";

pub fn run(args: &BuildArgs) -> Result<bool> {
    let session = Session::open()?;
    let platforms = session.select(&args.target.targets)?;
    let cx = session.context(args.optimize);
    let built = pipeline::build_all(&cx, &platforms)?;

    let dist = session.project.root.join(DIST_DIR);
    reset_dir(&dist)?;

    let mut packages = Vec::new();
    for item in &built {
        console::step(
            "Packaging",
            &format!("{} [{}]", item.module.name, item.platform.id()),
        );
        let target_dir = session.project.dist_dir(item.platform.id());
        let files = item
            .platform
            .package(&cx, item.module, &item.artifacts, &target_dir)?;
        packages.push(describe_package(&session, item, &files)?);
    }

    let index = json!({
        "project": { "name": session.project.name(), "version": session.project.version() },
        "compiler": format!("fe {}", session.environment.toolchain.version),
        "packages": packages,
    });
    let index_path = dist.join(INDEX_FILE);
    write_file(
        &index_path,
        format!("{}\n", serde_json::to_string_pretty(&index)?),
    )?;
    console::step(
        "Packaged",
        &console::relative(&index_path, &session.project.root),
    );
    Ok(true)
}

fn describe_package(
    session: &Session,
    item: &BuiltModule<'_>,
    files: &[Artifact],
) -> Result<Value> {
    let dist = session.project.root.join(DIST_DIR);
    let files = files
        .iter()
        .map(|file| {
            Ok(json!({
                "path": console::relative(&file.path, &dist),
                "kind": file.kind.to_string(),
                "sha256": sha256_hex(&file.path)?,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "platform": item.platform.id().as_str(),
        "module": item.module.name,
        "files": files,
    }))
}
