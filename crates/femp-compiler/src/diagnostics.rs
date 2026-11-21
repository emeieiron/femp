//! What the compiler reports about an ingot before it generates code, in
//! the order and format `fe check` prints it: the ingot's own analysis, then
//! its dependencies', then the checks that run on the lowered program.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt::Write as _;

use common::InputDb;
use common::diagnostics::{CompleteDiagnostic, Severity};
use common::file::IngotFileKind;
use common::stdlib::is_embedded_builtin;
use driver::DriverDataBase;
use driver::db::DiagnosticsCollection;
use hir::Ingot;
use hir::analysis::semantic::collect_blocked_semantic_bodies;
use hir::hir_def::TopLevelMod;
use url::Url;

/// The formatted diagnostics of one ingot.
pub struct Report {
    pub text: String,
    pub has_errors: bool,
}

pub fn check(db: &DriverDataBase, ingot: Ingot<'_>) -> Report {
    let mut text = String::new();
    let hir = db.run_on_ingot(ingot);
    if !hir.is_empty() {
        text.push_str(&hir.format_diags(db));
    }
    if hir.has_errors(db) {
        return Report {
            text,
            has_errors: true,
        };
    }

    let dependencies = dependency_issues(db, ingot.base(db));
    if !dependencies.is_empty() {
        push_dependencies(db, &dependencies, &mut text);
        return Report {
            text,
            has_errors: true,
        };
    }

    let mir = db.mir_diagnostics_for_ingot(ingot);
    let modules: Vec<_> = hir::lower::module_tree(db, ingot).all_modules().collect();
    let builtins = reached_builtin_issues(db, &modules, &mir);
    if !builtins.is_empty() {
        push_dependencies(db, &builtins, &mut text);
        return Report {
            text,
            has_errors: true,
        };
    }
    if !mir.is_empty() {
        text.push_str(&db.format_complete_diagnostics(&mir));
    }
    Report {
        text,
        has_errors: mir.iter().any(|diag| diag.severity == Severity::Error),
    }
}

pub(crate) fn has_source_files(db: &DriverDataBase, ingot: Ingot<'_>) -> bool {
    ingot
        .files(db)
        .iter()
        .any(|(_, file)| matches!(file.kind(db), Some(IngotFileKind::Source)))
}

enum Issue<'db> {
    MissingSourceFiles(Url),
    Diagnostics {
        url: Url,
        hir: DiagnosticsCollection<'db>,
        mir: Vec<CompleteDiagnostic>,
    },
}

// The ingot's dependencies, breadth first. The embedded core and std are
// left to `reached_builtin_issues`: analyzing them whole on every run is far
// too slow.
fn dependency_issues<'db>(db: &'db DriverDataBase, root: &Url) -> Vec<Issue<'db>> {
    let Some(root) = db.workspace().containing_ingot(db, root.clone()) else {
        return Vec::new();
    };
    let mut seen = HashSet::from([root.base(db).clone()]);
    let mut pending: VecDeque<Url> = root
        .dependencies(db)
        .iter()
        .map(|(_, url)| url.clone())
        .collect();
    let mut analyzed = Vec::new();
    while let Some(url) = pending.pop_front() {
        if !seen.insert(url.clone()) {
            continue;
        }
        let Some(ingot) = db.workspace().containing_ingot(db, url.clone()) else {
            continue;
        };
        pending.extend(ingot.dependencies(db).iter().map(|(_, url)| url.clone()));
        if is_embedded_builtin(&url) {
            continue;
        }
        if !has_source_files(db, ingot) {
            analyzed.push(Err(url));
            continue;
        }
        let hir = db.run_on_ingot(ingot);
        analyzed.push(Ok((url, ingot, hir)));
    }

    let hir_has_errors = analyzed.iter().any(|dependency| match dependency {
        Ok((_, _, hir)) => hir.has_errors(db),
        Err(_) => true,
    });
    analyzed
        .into_iter()
        .filter_map(|dependency| match dependency {
            Err(url) => Some(Issue::MissingSourceFiles(url)),
            Ok((url, ingot, hir)) => {
                let mir = if hir_has_errors {
                    Vec::new()
                } else {
                    db.mir_diagnostics_for_ingot(ingot)
                };
                (!hir.is_empty() || !mir.is_empty()).then_some(Issue::Diagnostics { url, hir, mir })
            }
        })
        .collect()
}

// Errors in the bodies of the embedded libraries first show up while the
// program is lowered: as bodies the analysis blocked, or as errors located
// in a library. Only the library modules involved are analyzed.
fn reached_builtin_issues<'db>(
    db: &'db DriverDataBase,
    roots: &[TopLevelMod<'db>],
    mir: &[CompleteDiagnostic],
) -> Vec<Issue<'db>> {
    let blocked = roots
        .iter()
        .flat_map(|root| collect_blocked_semantic_bodies(db, *root))
        .map(|body| body.instance.key(db).owner(db).scope().top_mod(db));
    let located = mir
        .iter()
        .filter(|diag| diag.severity == Severity::Error)
        .filter_map(|diag| diag.primary_span())
        .map(|span| db.top_mod(span.file));
    let mut seen = HashSet::new();
    let mut by_ingot = BTreeMap::<Url, DiagnosticsCollection<'db>>::new();
    for module in blocked.chain(located) {
        let url = module.ingot(db).base(db);
        if !is_embedded_builtin(url) || !seen.insert(module) {
            continue;
        }
        let hir = db.run_on_top_mod(module);
        if !hir.has_errors(db) {
            continue;
        }
        match by_ingot.get_mut(url) {
            Some(existing) => existing.append(hir),
            None => {
                by_ingot.insert(url.clone(), hir);
            }
        }
    }
    by_ingot
        .into_iter()
        .map(|(url, hir)| Issue::Diagnostics {
            url,
            hir,
            mir: Vec::new(),
        })
        .collect()
}

fn push_dependencies(db: &DriverDataBase, issues: &[Issue<'_>], out: &mut String) {
    let heading = if issues.len() == 1 {
        "Errors in dependency"
    } else {
        "Errors in dependencies"
    };
    let _ = writeln!(out, "Error: {heading}");
    for issue in issues {
        let url = match issue {
            Issue::MissingSourceFiles(url) | Issue::Diagnostics { url, .. } => url,
        };
        let _ = writeln!(out, "\n{}\nURL: {url}\n", dependency_name(db, url));
        match issue {
            Issue::MissingSourceFiles(url) => {
                let _ = writeln!(out, "Error: Could not find source files for ingot {url}");
            }
            Issue::Diagnostics { hir, mir, .. } => {
                if !hir.is_empty() {
                    out.push_str(&hir.format_diags(db));
                }
                if !mir.is_empty() {
                    out.push_str(&db.format_complete_diagnostics(mir));
                }
            }
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
}

fn dependency_name(db: &DriverDataBase, url: &Url) -> String {
    let config = db
        .workspace()
        .containing_ingot(db, url.clone())
        .and_then(|ingot| ingot.config(db).clone());
    let Some(config) = config else {
        return "Dependency: <unknown>".to_string();
    };
    let name = config.metadata.name.as_deref().unwrap_or("unknown");
    match &config.metadata.version {
        Some(version) => format!("Dependency: {name} (version: {version})"),
        None => format!("Dependency: {name}"),
    }
}
