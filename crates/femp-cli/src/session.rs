use std::env;

use anyhow::{Context as _, Result, bail};
use femp_core::environment::{Environment, OptLevel};
use femp_core::layering;
use femp_core::platform::{Capability, Context, Platform, PlatformRegistry};
use femp_core::project::Project;
use femp_host::probe;

use crate::console;
use crate::generation::{self, StagedProject};
use crate::registry;

const ALL_TARGETS: &str = "all";

pub struct Session {
    pub project: Project,
    pub registry: PlatformRegistry,
    pub environment: Environment,
}

pub struct BuildInputs<'a> {
    session: &'a Session,
    staged: Option<StagedProject>,
}

impl BuildInputs<'_> {
    pub fn context(&self, optimize: OptLevel) -> Context<'_> {
        self.context_with(optimize, None)
    }

    pub fn context_with(&self, optimize: OptLevel, wasm_stack_size: Option<u32>) -> Context<'_> {
        Context {
            project: self
                .staged
                .as_ref()
                .map_or(&self.session.project, |staged| &staged.project),
            environment: &self.session.environment,
            optimize,
            wasm_stack_size,
        }
    }
}

impl Session {
    pub fn open() -> Result<Self> {
        let current_dir = env::current_dir().context("cannot read the current directory")?;
        let project = Project::discover(&current_dir)?;
        let registry = registry::platforms();
        layering::validate(&project, &registry)?;
        femp_compiler::check_pin(&project.manifest.toolchain.fe)?;
        let environment = Environment {
            toolchain: femp_compiler::toolchain(),
            host: probe::host(),
        };
        Ok(Self {
            project,
            registry,
            environment,
        })
    }

    pub fn prepare(
        &self,
        platforms: &[&dyn Platform],
        include_shared: bool,
    ) -> Result<BuildInputs<'_>> {
        let staged = generation::stage(&self.project, platforms, include_shared)?;
        Ok(BuildInputs {
            session: self,
            staged,
        })
    }

    /// Resolves `--target` values to platforms. Naming a platform demands
    /// it: an unavailable one is an error. Without names, every platform the
    /// project targets is selected and unavailable ones are skipped with a
    /// warning, so one missing backend does not block the others.
    pub fn select(&self, requested: &[String]) -> Result<Vec<&dyn Platform>> {
        let explicit = !requested.is_empty() && requested.iter().all(|name| name != ALL_TARGETS);
        let names: Vec<String> = if explicit {
            requested.to_vec()
        } else {
            self.project
                .platform_ids()
                .iter()
                .map(|id| id.as_str().to_string())
                .collect()
        };

        let mut selected = Vec::new();
        for name in names {
            let platform = self.registry.get(&name)?;
            match platform.capability(&self.environment) {
                Capability::Available => selected.push(platform),
                Capability::Unavailable { reason, fix } => {
                    let hint = fix.map(|fix| format!(" ({fix})")).unwrap_or_default();
                    if explicit {
                        bail!("platform `{name}` is unavailable: {reason}{hint}");
                    }
                    console::warning(&format!("skipping `{name}`: {reason}{hint}"));
                }
            }
        }
        if selected.is_empty() {
            bail!("no platform is available to build for; run `femp doctor` for details");
        }
        Ok(selected)
    }
}
