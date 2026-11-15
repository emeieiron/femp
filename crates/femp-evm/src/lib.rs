mod abi;
mod bindings;
mod build;

use std::path::Path;

use femp_core::Result;
use femp_core::artifact::{Artifact, copy_flat};
use femp_core::environment::Environment;
use femp_core::outcome::TestRun;
use femp_core::platform::{Capability, Context, Platform, PlatformId, Stability};
use femp_core::project::Module;
use femp_toolchain::module_ops;

pub use build::{ABI, BYTECODE, METADATA, RUNTIME_BYTECODE};

pub const ID: &str = "evm";
const BACKEND: &str = "sonatina";

pub struct EvmPlatform {
    id: PlatformId,
}

impl EvmPlatform {
    pub fn new() -> Self {
        Self {
            id: PlatformId::new(ID),
        }
    }
}

impl Default for EvmPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for EvmPlatform {
    fn id(&self) -> &PlatformId {
        &self.id
    }

    fn fe_target(&self) -> &str {
        "evm"
    }

    fn stability(&self) -> Stability {
        Stability::Stable
    }

    fn capability(&self, environment: &Environment) -> Capability {
        if environment.toolchain.supports(BACKEND) {
            Capability::Available
        } else {
            Capability::unavailable(
                "this build of the Fe compiler has no EVM backend",
                "run `femp toolchain install`",
            )
        }
    }

    fn binding_kinds(&self) -> &[&'static str] {
        &[bindings::TYPESCRIPT]
    }

    fn check(&self, cx: &Context<'_>, module: &Module) -> Result<()> {
        module_ops::check(cx, BACKEND, module)
    }

    fn build(&self, cx: &Context<'_>, module: &Module, out_dir: &Path) -> Result<Vec<Artifact>> {
        build::build(cx, BACKEND, module, out_dir)
    }

    fn test(&self, cx: &Context<'_>, module: &Module, filter: Option<&str>) -> Result<TestRun> {
        module_ops::test(cx, BACKEND, &self.id, module, filter)
    }

    fn bindings(
        &self,
        _cx: &Context<'_>,
        _module: &Module,
        built: &[Artifact],
        kinds: &[String],
        out_dir: &Path,
    ) -> Result<Vec<Artifact>> {
        if kinds.iter().any(|kind| kind == bindings::TYPESCRIPT) {
            bindings::generate(built, out_dir)
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
}
