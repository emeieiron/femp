mod build;
mod harness;

use std::path::Path;

use femp_core::Result;
use femp_core::artifact::{Artifact, copy_flat};
use femp_core::environment::{Environment, Host};
use femp_core::outcome::TestRun;
use femp_core::platform::{Capability, Context, Platform, PlatformId, Stability};
use femp_core::project::Module;
use femp_core::property::{Executor, HarnessRequest};
use femp_toolchain::module_ops;

pub use build::EXECUTABLE;

pub const ID: &str = "native";
const BACKEND: &str = "native";
const SUPPORTED_HOSTS: &[(&str, &str)] = &[("linux", "x86_64"), ("macos", "aarch64")];

pub struct NativePlatform {
    id: PlatformId,
}

impl NativePlatform {
    pub fn new() -> Self {
        Self {
            id: PlatformId::new(ID),
        }
    }
}

impl Default for NativePlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for NativePlatform {
    fn id(&self) -> &PlatformId {
        &self.id
    }

    fn fe_target(&self) -> &str {
        "native"
    }

    fn stability(&self) -> Stability {
        Stability::Experimental
    }

    fn capability(&self, environment: &Environment) -> Capability {
        if !environment.toolchain.supports(BACKEND) {
            return Capability::unavailable(
                "this build of the Fe compiler has no native backend",
                "run `femp toolchain install`",
            );
        }
        if !host_is_supported(&environment.host) {
            return Capability::Unavailable {
                reason: format!(
                    "native executables can only be built on x86-64 Linux and AArch64 macOS; \
                     this host is {} {}",
                    environment.host.arch, environment.host.os
                ),
                fix: None,
            };
        }
        if !environment.host.has_c_compiler {
            return Capability::unavailable(
                "no C compiler (`cc`) was found to link executables",
                "install a C toolchain",
            );
        }
        Capability::Available
    }

    fn binding_kinds(&self) -> &[&'static str] {
        &[]
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
        _built: &[Artifact],
        _kinds: &[String],
        _out_dir: &Path,
    ) -> Result<Vec<Artifact>> {
        Ok(Vec::new())
    }

    fn package(
        &self,
        cx: &Context<'_>,
        _module: &Module,
        built: &[Artifact],
        dist_dir: &Path,
    ) -> Result<Vec<Artifact>> {
        let host = &cx.environment.host;
        copy_flat(built, &dist_dir.join(format!("{}-{}", host.arch, host.os)))
    }

    fn property_harness(
        &self,
        cx: &Context<'_>,
        request: &HarnessRequest<'_>,
    ) -> Result<Box<dyn Executor>> {
        harness::start(cx, &self.id, BACKEND, request)
    }
}

fn host_is_supported(host: &Host) -> bool {
    SUPPORTED_HOSTS
        .iter()
        .any(|(os, arch)| host.os == *os && host.arch == *arch)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use femp_core::environment::Toolchain;

    use super::*;

    fn environment(backends: &[&str], os: &str, arch: &str, has_c_compiler: bool) -> Environment {
        Environment {
            toolchain: Toolchain {
                fe: "fe".into(),
                version: "26.6.0".to_string(),
                backends: backends
                    .iter()
                    .map(|b| b.to_string())
                    .collect::<BTreeSet<_>>(),
            },
            host: Host {
                os: os.to_string(),
                arch: arch.to_string(),
                node: None,
                node_major: None,
                z3: None,
                z3_version: None,
                has_c_compiler,
            },
        }
    }

    #[test]
    fn available_on_supported_hosts() {
        let platform = NativePlatform::new();
        assert!(
            platform
                .capability(&environment(&["native"], "macos", "aarch64", true))
                .is_available()
        );
        assert!(
            platform
                .capability(&environment(&["native"], "linux", "x86_64", true))
                .is_available()
        );
    }

    #[test]
    fn unavailable_without_backend_host_or_linker() {
        let platform = NativePlatform::new();
        for environment in [
            environment(&["sonatina"], "macos", "aarch64", true),
            environment(&["native"], "windows", "x86_64", true),
            environment(&["native"], "macos", "aarch64", false),
        ] {
            assert!(!platform.capability(&environment).is_available());
        }
    }
}
