use femp_core::project::{Module, Project};
use serde_json::json;

pub const FILE: &str = "package.json";

pub fn render(
    project: &Project,
    module: &Module,
    script: &str,
    types: &str,
    files: &[String],
) -> String {
    let description = json!({
        "name": format!("{}-{}", project.name(), module.name),
        "version": project.version(),
        "type": "module",
        "main": format!("./{script}"),
        "types": format!("./{types}"),
        "files": files,
    });
    let text = serde_json::to_string_pretty(&description).unwrap_or_default();
    format!("{text}\n")
}
