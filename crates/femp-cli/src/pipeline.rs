use anyhow::Result;
use femp_core::artifact::Artifact;
use femp_core::platform::{Context, Platform};
use femp_core::project::Module;

use crate::console;

pub struct BuiltModule<'a> {
    pub module: &'a Module,
    pub platform: &'a dyn Platform,
    pub artifacts: Vec<Artifact>,
}

pub fn build_all<'a>(
    cx: &Context<'a>,
    platforms: &[&'a dyn Platform],
) -> Result<Vec<BuiltModule<'a>>> {
    let mut built = Vec::new();
    for platform in platforms {
        let kinds = binding_kinds(cx, *platform);
        for module in cx.project.modules_for(platform.id()) {
            console::step("Building", &format!("{} [{}]", module.name, platform.id()));
            let out_dir = cx.project.build_dir(platform.id(), module);
            let mut artifacts = platform
                .build(cx, module, &out_dir)
                .map_err(|error| module.compiler_error(error))?;
            let bindings = platform.bindings(cx, module, &artifacts, &kinds, &out_dir)?;
            artifacts.extend(bindings);
            built.push(BuiltModule {
                module,
                platform: *platform,
                artifacts,
            });
        }
    }
    Ok(built)
}

fn binding_kinds(cx: &Context<'_>, platform: &dyn Platform) -> Vec<String> {
    match cx.project.requested_bindings(platform.id()) {
        Some(requested) => requested.to_vec(),
        None => platform
            .binding_kinds()
            .iter()
            .map(|kind| kind.to_string())
            .collect(),
    }
}
