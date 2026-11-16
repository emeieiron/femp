//! The metadata of a contract, as `fe build --emit metadata` writes it: the
//! Solidity contract metadata format adapted to Fe, with every source the
//! contract was built from, so that the build can be reproduced. It follows
//! the Fe CLI (Apache-2.0) field for field.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use common::InputDb;
use common::config::{
    ArithmeticMode, DependencyArithmeticMode, resolve_dependency_arithmetic_mode,
};
use common::file::IngotFileKind;
use common::ingot::{IngotBaseUrl, IngotKind};
use driver::DriverDataBase;
use hir::hir_def::TopLevelMod;
use hir::lower::map_file_to_mod;
use serde_json::{Map, Value, json};
use tiny_keccak::{Hasher, Keccak};
use url::Url;

use super::abi::generate_contract_abi;
use crate::names::file_name;
use crate::{LINKED_FE_COMMIT, LINKED_FE_VERSION};

/// The metadata of each named contract the ingot declares, as text.
pub fn render(
    db: &DriverDataBase,
    ingot: hir::Ingot<'_>,
    names: &[String],
    optimization: &str,
) -> Result<BTreeMap<String, String>, String> {
    let profile = db.compilation_settings().profile(db);
    let profile = profile.as_str();

    let mut root_sources = BTreeMap::new();
    collect_sources(db, ingot, None, &mut root_sources);

    let closure = non_builtin_closure(db, ingot);
    let root_keys = root_sources.keys().cloned().collect();
    let namespaces = assign_namespaces(db, ingot, &closure, &root_keys);
    let mut all_sources = root_sources.clone();
    for &dependency in &closure {
        if dependency.base(db) == ingot.base(db) {
            continue;
        }
        let namespace = namespaces
            .get(dependency.base(db))
            .cloned()
            .unwrap_or_default();
        collect_sources(db, dependency, Some(&namespace), &mut all_sources);
    }

    let contracts = mir::ingot_contracts(db, ingot);
    let mut located = BTreeMap::new();
    for declared in contracts {
        let Some((path, _)) = root_sources
            .iter()
            .find(|(_, (top_mod, _))| *top_mod == declared.top_mod)
        else {
            continue;
        };
        located.insert(declared.name.as_str(), (path.as_str(), declared));
    }

    let mut sources = Map::new();
    for (key, (_, contents)) in &all_sources {
        sources.insert(
            key.clone(),
            json!({
                "keccak256": keccak256_hex(contents.as_bytes()),
                "content": contents,
            }),
        );
    }

    // Each ingot's arithmetic is recorded after `dependency-arithmetic` forced
    // it, so the dependency map needs no edge kinds to reproduce the build.
    let ingots: Vec<Value> = closure
        .iter()
        .map(|&member| {
            let namespace = namespaces.get(member.base(db)).cloned().unwrap_or_default();
            let dependency_arithmetic = resolve_dependency_arithmetic_mode(
                member.config(db).as_ref(),
                member.workspace_config(db).as_ref(),
                profile,
            );
            json!({
                "name": ingot_namespace(db, member),
                "version": member.version(db).as_ref().map(|version| version.to_string()),
                "namespace": namespace,
                "arithmetic": arithmetic(member.arithmetic_mode(db)),
                "dependencyArithmetic": dependency_arithmetic_name(dependency_arithmetic),
                "dependencies": Value::Object(dependencies(db, member, &namespaces)),
            })
        })
        .collect();
    let root_arithmetic = arithmetic(ingot.arithmetic_mode(db));
    let root_dependency_arithmetic =
        dependency_arithmetic_name(resolve_dependency_arithmetic_mode(
            ingot.config(db).as_ref(),
            ingot.workspace_config(db).as_ref(),
            profile,
        ));
    let compiler = json!({ "version": LINKED_FE_VERSION, "commit": LINKED_FE_COMMIT });

    let mut rendered = BTreeMap::new();
    for name in names {
        let Some(&(source_path, declared)) = located.get(name.as_str()) else {
            continue;
        };
        let abi = match declared.contract() {
            Some(contract) => {
                let result = generate_contract_abi(db, contract)
                    .map_err(|error| format!("cannot generate the ABI of {name}: {error}"))?;
                serde_json::from_str::<Value>(&result.json).unwrap_or_else(|_| json!([]))
            }
            None => json!([]),
        };
        let mut target = Map::new();
        target.insert(
            source_path.to_string(),
            Value::String(declared.path_in_file.clone()),
        );
        let value = json!({
            "version": 1,
            "language": "Fe",
            "compiler": compiler.clone(),
            "sources": Value::Object(sources.clone()),
            "settings": {
                "compilationTarget": Value::Object(target),
                "optimizer": { "level": optimization },
                "arithmetic": root_arithmetic,
                "dependencyArithmetic": root_dependency_arithmetic,
                "evmVersion": "osaka",
                "ingots": ingots.clone(),
            },
            "output": { "abi": abi },
        });
        let text = serde_json::to_string_pretty(&sorted(value))
            .map_err(|error| format!("cannot serialize the metadata of {name}: {error}"))?;
        rendered.insert(name.clone(), text);
    }
    Ok(rendered)
}

// The compiler writes objects with their keys in order.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// keccak256 as `0x` and lowercase hex, as Solidity records sources.
fn keccak256_hex(bytes: &[u8]) -> String {
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    format!("0x{}", hex::encode(out))
}

fn arithmetic(mode: Option<ArithmeticMode>) -> &'static str {
    match mode {
        Some(ArithmeticMode::Unchecked) => "unchecked",
        Some(ArithmeticMode::Checked) | None => "checked",
    }
}

fn dependency_arithmetic_name(mode: DependencyArithmeticMode) -> &'static str {
    match mode {
        DependencyArithmeticMode::Defer => "defer",
        DependencyArithmeticMode::Checked => "checked",
        DependencyArithmeticMode::Unchecked => "unchecked",
    }
}

/// The ingot's declared name, or the last segment of its URL.
fn ingot_namespace(db: &DriverDataBase, ingot: hir::Ingot<'_>) -> String {
    if let Some(name) = ingot
        .config(db)
        .as_ref()
        .and_then(|config| config.metadata.name.as_ref())
    {
        return name.to_string();
    }
    ingot
        .base(db)
        .path()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or("dep")
        .to_string()
}

type Sources<'db> = BTreeMap<String, (TopLevelMod<'db>, String)>;

/// An ingot's source files by their path in the ingot, under `prefix`.
fn collect_sources<'db>(
    db: &'db DriverDataBase,
    ingot: hir::Ingot<'db>,
    prefix: Option<&str>,
    out: &mut Sources<'db>,
) {
    let standalone = ingot.standalone_file(db).is_some();
    for (url, file) in ingot.files(db).iter() {
        if file.kind(db) != Some(IngotFileKind::Source) {
            continue;
        }
        let relative = if standalone {
            url.path()
                .rsplit('/')
                .next()
                .filter(|segment| !segment.is_empty())
                .unwrap_or("input.fe")
                .to_string()
        } else {
            match file.path(db).clone() {
                Some(path) if !path.as_str().is_empty() => path.into_string(),
                _ => continue,
            }
        };
        let key = match prefix {
            Some(namespace) => format!("{namespace}/{relative}"),
            None => relative,
        };
        out.insert(key, (map_file_to_mod(db, file), file.text(db).clone()));
    }
}

/// The root ingot and its dependencies, without the bundled core and std.
fn non_builtin_closure<'db>(
    db: &'db DriverDataBase,
    root: hir::Ingot<'db>,
) -> Vec<hir::Ingot<'db>> {
    let mut closure = vec![root];
    for url in db.dependency_graph().dependency_urls(db, root.base(db)) {
        let Some(dependency) = url.ingot(db) else {
            continue;
        };
        if matches!(dependency.kind(db), IngotKind::Core | IngotKind::Std) {
            continue;
        }
        closure.push(dependency);
    }
    closure
}

// A unique namespace per ingot: empty for the root, the name for the
// others, qualified by version (then a counter) when two ingots share a name,
// and never a prefix of a root source path.
fn assign_namespaces(
    db: &DriverDataBase,
    root: hir::Ingot<'_>,
    closure: &[hir::Ingot<'_>],
    root_keys: &BTreeSet<String>,
) -> HashMap<Url, String> {
    let root_base = root.base(db);
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for &member in closure {
        if member.base(db) != root_base {
            *name_counts.entry(ingot_namespace(db, member)).or_default() += 1;
        }
    }
    let mut map = HashMap::from([(root_base.clone(), String::new())]);
    let mut used: HashSet<String> =
        HashSet::from([String::new(), "std".to_string(), "core".to_string()]);
    let mut others: Vec<hir::Ingot<'_>> = closure
        .iter()
        .copied()
        .filter(|member| member.base(db) != root_base)
        .collect();
    others.sort_by_key(|member| member.base(db).to_string());
    for member in others {
        let name = ingot_namespace(db, member);
        let mut candidate = if name_counts.get(&name).copied().unwrap_or(0) > 1 {
            match member.version(db) {
                Some(version) => format!("{name}-{}", file_name(&version.to_string())),
                None => name.clone(),
            }
        } else {
            name.clone()
        };
        let stem = candidate.clone();
        let mut counter = 2;
        while used.contains(&candidate)
            || root_keys.iter().any(|key| {
                key.strip_prefix(candidate.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            })
        {
            candidate = format!("{stem}-{counter}");
            counter += 1;
        }
        used.insert(candidate.clone());
        map.insert(member.base(db).clone(), candidate);
    }
    map
}

/// `{ alias: namespace }` for an ingot's direct dependencies.
fn dependencies(
    db: &DriverDataBase,
    ingot: hir::Ingot<'_>,
    namespaces: &HashMap<Url, String>,
) -> Map<String, Value> {
    let mut map = Map::new();
    for (alias, url) in ingot.dependencies(db) {
        let value = match url.ingot(db) {
            Some(dependency) if matches!(dependency.kind(db), IngotKind::Std) => "std".to_string(),
            Some(dependency) if matches!(dependency.kind(db), IngotKind::Core) => {
                "core".to_string()
            }
            Some(dependency) => namespaces
                .get(dependency.base(db))
                .cloned()
                .unwrap_or_else(|| ingot_namespace(db, dependency)),
            None => alias.to_string(),
        };
        map.insert(alias.to_string(), Value::String(value));
    }
    map
}
