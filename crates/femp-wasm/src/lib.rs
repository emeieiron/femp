mod bindings;
mod build;
mod harness;
mod manifest;

use std::path::Path;

use femp_compiler::{Target, TestOptions, WasmRunner};
use femp_core::artifact::{Artifact, copy_flat};
use femp_core::environment::Environment;
use femp_core::outcome::TestRun;
use femp_core::platform::{Capability, Context, Platform, PlatformId, Stability};
use femp_core::project::Module;
use femp_core::property::{Executor, HarnessRequest};
use femp_core::{Error, Result};
use femp_host::node::MIN_MAJOR as MIN_NODE_MAJOR;

pub use build::{MANIFEST, MODULE};

pub const ID: &str = "wasm";

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

    // Building needs nothing but the linked compiler; running tests and
    // properties needs Node.js, which those commands ask for.
    fn capability(&self, _environment: &Environment) -> Capability {
        Capability::Available
    }

    fn binding_kinds(&self) -> &[&'static str] {
        &[bindings::TYPESCRIPT]
    }

    fn check(&self, _cx: &Context<'_>, module: &Module) -> Result<()> {
        femp_compiler::load(module, Target::Wasm).map(drop)
    }

    fn build(&self, cx: &Context<'_>, module: &Module, out_dir: &Path) -> Result<Vec<Artifact>> {
        build::build(cx, module, out_dir)
    }

    fn test(&self, cx: &Context<'_>, module: &Module, filter: Option<&str>) -> Result<TestRun> {
        let node = require_node(cx)?;
        let options = TestOptions {
            optimize: cx.optimize,
            filter,
            wasm: Some(WasmRunner {
                node,
                host_script: bindings::HOST_SCRIPT,
            }),
        };
        femp_compiler::test(module, &self.id, Target::Wasm, &options)
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
        harness::start(cx, &self.id, request)
    }
}

/// The Node.js that runs WebAssembly code.
fn require_node<'a>(cx: &Context<'a>) -> Result<&'a Path> {
    let host = &cx.environment.host;
    match (&host.node, host.node_major) {
        (Some(node), Some(major)) if major >= MIN_NODE_MAJOR => Ok(node),
        _ => Err(Error::Unavailable {
            platform: ID.to_string(),
            reason: format!(
                "running WebAssembly code needs Node.js {MIN_NODE_MAJOR} or newer on PATH (the femp installer provides one)"
            ),
        }),
    }
}
