mod bindings;
mod build;
mod bundle;
mod harness;
mod manifest;

use std::path::Path;

use femp_core::artifact::{Artifact, copy_flat};
use femp_core::environment::Environment;
use femp_core::outcome::TestRun;
use femp_core::platform::{Capability, Context, Platform, PlatformId, Stability};
use femp_core::project::Module;
use femp_core::property::{Executor, HarnessRequest};
use femp_core::{Error, Result};
use femp_toolchain::module_ops;
use femp_toolchain::node::MIN_MAJOR as MIN_NODE_MAJOR;

pub use build::{MANIFEST, MODULE};

pub const ID: &str = "wasm";
const BACKEND: &str = "wasm";

pub struct WasmPlatform {
    id: PlatformId,
}

impl WasmPlatform {
    pub fn new() -> Self {
        Self {
            id: PlatformId::new(ID),
        }
    }
}

impl Default for WasmPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for WasmPlatform {
    fn id(&self) -> &PlatformId {
        &self.id
    }

    fn fe_target(&self) -> &str {
        "wasm"
    }

    fn stability(&self) -> Stability {
        Stability::Experimental
    }

    fn capability(&self, environment: &Environment) -> Capability {
        if environment.toolchain.supports(BACKEND) {
            Capability::Available
        } else {
            Capability::unavailable(
                "this build of the Fe compiler has no WebAssembly backend \
                 (released binaries are built without it)",
                "run `femp toolchain install`",
            )
        }
    }

    fn binding_kinds(&self) -> &[&'static str] {
        &[bindings::TYPESCRIPT]
    }

    fn check(&self, cx: &Context<'_>, module: &Module) -> Result<()> {
        module_ops::check(cx, BACKEND, module)?;
        if module.platform() == Some(&self.id) {
            build::check_bundle(cx, BACKEND, module)?;
        }
        Ok(())
    }

    fn build(&self, cx: &Context<'_>, module: &Module, out_dir: &Path) -> Result<Vec<Artifact>> {
        build::build(cx, BACKEND, module, out_dir)
    }

    fn test(&self, cx: &Context<'_>, module: &Module, filter: Option<&str>) -> Result<TestRun> {
        require_node(cx)?;
        module_ops::test(cx, BACKEND, &self.id, module, filter)
    }

    fn bindings(
        &self,
        cx: &Context<'_>,
        module: &Module,
        built: &[Artifact],
        kinds: &[String],
        out_dir: &Path,
    ) -> Result<Vec<Artifact>> {
        if kinds.iter().any(|kind| kind == bindings::TYPESCRIPT) {
            bindings::generate(cx.project, module, built, out_dir)
        } else {
            Ok(Vec::new())
        }
    }

    fn package(
        &self,
        _cx: &Context<'_>,
        module: &Module,
        built: &[Artifact],
        dist_dir: &Path,
    ) -> Result<Vec<Artifact>> {
        copy_flat(built, &dist_dir.join(&module.name))
    }

    fn property_harness(
        &self,
        cx: &Context<'_>,
        request: &HarnessRequest<'_>,
    ) -> Result<Box<dyn Executor>> {
        require_node(cx)?;
        harness::start(cx, &self.id, BACKEND, request)
    }
}

fn require_node(cx: &Context<'_>) -> Result<()> {
    let node = cx.environment.host.node_major;
    if node.is_none_or(|major| major < MIN_NODE_MAJOR) {
        return Err(Error::Unavailable {
            platform: ID.to_string(),
            reason: format!(
                "running WebAssembly code needs Node.js {MIN_NODE_MAJOR} or newer on PATH (the femp installer provides one)"
            ),
        });
    }
    Ok(())
}
