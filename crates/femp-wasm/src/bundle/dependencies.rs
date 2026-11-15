use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use femp_core::ingot::{DependencySource, GitDependency, IngotDependency, IngotManifest};
use femp_core::project::{Module, Project};
use femp_core::workspace::{Workspace, is_workspace, read_config};
use femp_core::{Error, Result};

pub struct Ingot {
    pub name: String,
    pub edges: BTreeMap<String, PathBuf>,
}

pub fn graph(project: &Project, module: &Module) -> Result<BTreeMap<PathBuf, Ingot>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![module.dir.clone()];
    while let Some(dir) = pending.pop() {
        if found.contains_key(&dir) {
            continue;
        }
        let ingot = IngotManifest::read(&dir)?;
        let workspace = Workspace::discover(&dir)?;
        let mut edges = BTreeMap::new();
        for dependency in &ingot.dependencies {
            let path = resolve(project, &dir, workspace.as_ref(), dependency)?;
            pending.push(path.clone());
            edges.insert(dependency.alias.clone(), path);
        }
        found.insert(
            dir,
            Ingot {
                name: ingot.name,
                edges,
            },
        );
    }
    Ok(found)
}

fn resolve(
    project: &Project,
    dir: &Path,
    workspace: Option<&Workspace>,
    dependency: &IngotDependency,
) -> Result<PathBuf> {
    let missing = || {
        Error::Artifact(format!(
            "cannot resolve dependency {} of {} for WASM",
            dependency.alias,
            dir.display()
        ))
    };
    let (base, source, name) = match (&dependency.source, workspace) {
        // Fe gives an inherited workspace dependency precedence over member lookup.
        (DependencySource::Name(_), Some(workspace)) => {
            match workspace
                .dependencies
                .iter()
                .find(|item| item.alias == dependency.alias)
            {
                Some(inherited) => (
                    workspace.root.as_path(),
                    &inherited.source,
                    dependency.name.as_deref().or(inherited.name.as_deref()),
                ),
                None => (dir, &dependency.source, dependency.name.as_deref()),
            }
        }
        _ => (dir, &dependency.source, dependency.name.as_deref()),
    };
    match source {
        DependencySource::Path(path) => select(&base.join(path), name),
        DependencySource::Remote(git) => select(&checkout(git)?, name),
        DependencySource::Name(name) => {
            if let Some(path) = workspace.and_then(|workspace| workspace.members.get(name)) {
                return Ok(path.clone());
            }
            project
                .modules
                .iter()
                .find(|module| &module.ingot.name == name)
                .map(|module| module.dir.clone())
                .ok_or_else(missing)
        }
    }
}

fn select(path: &Path, name: Option<&str>) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .map_err(|error| Error::io("resolve dependency", path, error))?;
    let document = read_config(&path)?;
    if is_workspace(&document) {
        let workspace = Workspace::load(path, document)?;
        if let Some(path) = name.and_then(|name| workspace.members.get(name)) {
            return Ok(path.clone());
        }
        return Err(Error::Artifact(format!(
            "cannot select ingot {} in dependency workspace {}",
            name.unwrap_or("(missing name)"),
            workspace.root.display()
        )));
    }
    Ok(path)
}

/// The original ingot has already been checked by Fe. Read the resolved
/// checkout instead of doing a second Git fetch or selecting a different rev.
fn checkout(git: &GitDependency) -> Result<PathBuf> {
    use femp_core::artifact::sha256;
    let source =
        url::Url::parse(&git.source).map_err(|error| Error::Artifact(error.to_string()))?;
    let key = sha256(format!("{}@{}", source.as_str(), git.rev).as_bytes());
    let root = env::var_os("FE_REMOTE_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("FE_CACHE_DIR").map(PathBuf::from).map(|path| {
                if path.ends_with("git") {
                    path
                } else {
                    path.join("git")
                }
            })
        })
        .or_else(|| env::var_os("XDG_CACHE_HOME").map(|path| PathBuf::from(path).join("fe/git")))
        .or_else(default_git_cache)
        .unwrap_or_else(|| env::temp_dir().join("fe/git"));
    let path = root.join(key);
    Ok(git
        .path
        .as_ref()
        .map_or_else(|| path.clone(), |subdir| path.join(subdir)))
}

fn default_git_cache() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        env::var_os("HOME").map(|path| PathBuf::from(path).join(".cache/fe/git"))
    }
    #[cfg(windows)]
    {
        env::var_os("LOCALAPPDATA")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(|path| PathBuf::from(path).join("Fe/cache/git"))
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}
