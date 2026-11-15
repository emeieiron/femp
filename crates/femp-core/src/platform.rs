use std::fmt;
use std::path::Path;

use crate::artifact::Artifact;
use crate::environment::{Environment, OptLevel};
use crate::error::{Error, Result};
use crate::outcome::TestRun;
use crate::project::{Module, Project};
use crate::property::{Executor, HarnessRequest};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlatformId(String);

impl PlatformId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PlatformId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stability {
    Stable,
    Experimental,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    Available,
    Unavailable { reason: String, fix: Option<String> },
}

impl Capability {
    pub fn unavailable(reason: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
            fix: Some(fix.into()),
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

pub struct Context<'a> {
    pub project: &'a Project,
    pub environment: &'a Environment,
    pub optimize: OptLevel,
}

pub trait Platform {
    fn id(&self) -> &PlatformId;

    fn fe_target(&self) -> &str;

    fn stability(&self) -> Stability;

    fn capability(&self, environment: &Environment) -> Capability;

    fn binding_kinds(&self) -> &[&'static str];

    fn check(&self, cx: &Context<'_>, module: &Module) -> Result<()>;

    fn build(&self, cx: &Context<'_>, module: &Module, out_dir: &Path) -> Result<Vec<Artifact>>;

    fn test(&self, cx: &Context<'_>, module: &Module, filter: Option<&str>) -> Result<TestRun>;

    fn bindings(
        &self,
        cx: &Context<'_>,
        module: &Module,
        built: &[Artifact],
        kinds: &[String],
        out_dir: &Path,
    ) -> Result<Vec<Artifact>>;

    fn package(
        &self,
        cx: &Context<'_>,
        module: &Module,
        built: &[Artifact],
        dist_dir: &Path,
    ) -> Result<Vec<Artifact>>;

    /// Compiles a property harness for the request's suite and starts an
    /// executor for it. Platforms that cannot run properties keep the default.
    fn property_harness(
        &self,
        _cx: &Context<'_>,
        _request: &HarnessRequest<'_>,
    ) -> Result<Box<dyn Executor>> {
        Err(Error::Unavailable {
            platform: self.id().to_string(),
            reason: "this platform cannot run property tests".to_string(),
        })
    }
}

#[derive(Default)]
pub struct PlatformRegistry {
    platforms: Vec<Box<dyn Platform>>,
}

impl PlatformRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, platform: Box<dyn Platform>) {
        self.platforms.push(platform);
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Platform> {
        self.platforms.iter().map(Box::as_ref)
    }

    pub fn find(&self, id: &str) -> Option<&dyn Platform> {
        self.iter().find(|platform| platform.id().as_str() == id)
    }

    pub fn get(&self, id: &str) -> Result<&dyn Platform> {
        self.find(id).ok_or_else(|| Error::UnknownPlatform {
            name: id.to_string(),
            known: self.known_ids(),
        })
    }

    pub fn known_ids(&self) -> String {
        self.iter()
            .map(|platform| platform.id().as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
