use std::fs;
use std::path::Path;

use crate::artifact::Artifact;
use crate::environment::Environment;
use crate::error::Result;
use crate::outcome::TestRun;
use crate::platform::{Capability, Context, Platform, PlatformId, PlatformRegistry, Stability};
use crate::project::Module;

pub struct StubPlatform {
    id: PlatformId,
}

impl StubPlatform {
    pub fn new(id: &str) -> Self {
        Self {
            id: PlatformId::new(id),
        }
    }
}

impl Platform for StubPlatform {
    fn id(&self) -> &PlatformId {
        &self.id
    }

    fn fe_target(&self) -> &str {
        self.id.as_str()
    }

    fn stability(&self) -> Stability {
        Stability::Stable
    }

    fn capability(&self, _: &Environment) -> Capability {
        Capability::Available
    }

    fn binding_kinds(&self) -> &[&'static str] {
        &["typescript"]
    }

    fn supports_stack_size(&self) -> bool {
        self.id.as_str() == "wasm"
    }

    fn check(&self, _: &Context<'_>, _: &Module) -> Result<()> {
        Ok(())
    }

    fn build(&self, _: &Context<'_>, _: &Module, _: &Path) -> Result<Vec<Artifact>> {
        Ok(Vec::new())
    }

    fn test(&self, _: &Context<'_>, module: &Module, _: Option<&str>) -> Result<TestRun> {
        Ok(TestRun {
            module: module.name.clone(),
            platform: self.id.clone(),
            cases: Vec::new(),
            suite_error: None,
            log: String::new(),
        })
    }

    fn bindings(
        &self,
        _: &Context<'_>,
        _: &Module,
        _: &[Artifact],
        _: &[String],
        _: &Path,
    ) -> Result<Vec<Artifact>> {
        Ok(Vec::new())
    }

    fn package(
        &self,
        _: &Context<'_>,
        _: &Module,
        _: &[Artifact],
        _: &Path,
    ) -> Result<Vec<Artifact>> {
        Ok(Vec::new())
    }
}

pub fn registry(ids: &[&str]) -> PlatformRegistry {
    let mut registry = PlatformRegistry::new();
    for id in ids {
        registry.register(Box::new(StubPlatform::new(id)));
    }
    registry
}

pub fn write_project(root: &Path, manifest: &str, ingots: &[(&str, &str)]) {
    fs::write(root.join("femp.toml"), manifest).unwrap();
    for (dir, ingot) in ingots {
        fs::create_dir_all(root.join(dir)).unwrap();
        fs::write(root.join(dir).join("fe.toml"), ingot).unwrap();
    }
}
