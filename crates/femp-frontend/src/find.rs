//! Finds functions by the names claims give them: a module path inside the
//! ingot, the type of an inherent `impl` for methods, and the function name.

use fe_hir::analysis::HirAnalysisDb;
use fe_hir::hir_def::{Func, ItemKind, Partial};

use crate::load::Loaded;

/// A function as claims name it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Root {
    pub path: Vec<String>,
    pub owner: Option<String>,
    pub name: String,
}

impl Root {
    pub fn function(path: &[String], name: &str) -> Self {
        Self {
            path: path.to_vec(),
            owner: None,
            name: name.to_string(),
        }
    }

    pub fn method(path: &[String], owner: &str, name: &str) -> Self {
        Self {
            path: path.to_vec(),
            owner: Some(owner.to_string()),
            name: name.to_string(),
        }
    }
}

/// Where a function sits: its module path, the type its `impl` block is
/// for, and its name. Trait methods have no place; claims never name them.
pub(crate) struct Place {
    pub path: Vec<String>,
    pub owner: Option<String>,
    pub name: String,
}

pub(crate) fn place<'db>(db: &'db dyn HirAnalysisDb, func: Func<'db>) -> Option<Place> {
    let name = match func.name(db) {
        Partial::Present(name) => name.data(db).to_string(),
        Partial::Absent => return None,
    };
    let scope = func.scope();
    let owner = match scope.parent_item(db) {
        Some(ItemKind::Impl(item)) => Some(base_name(item.ty(db).pretty_print(db))),
        Some(ItemKind::ImplTrait(_) | ItemKind::Trait(_)) => return None,
        _ => None,
    };
    let module = scope.parent_module(db)?;
    let mut path: Vec<String> = module
        .pretty_path(db)?
        .split("::")
        .map(str::to_string)
        .collect();
    // The first segment is the ingot's root module.
    path.remove(0);
    Some(Place { path, owner, name })
}

fn base_name(printed: &str) -> String {
    printed
        .split('<')
        .next()
        .unwrap_or(printed)
        .trim()
        .to_string()
}

impl Loaded {
    pub(crate) fn find(&self, root: &Root) -> Option<Func<'_>> {
        let db = &self.db;
        let ingot = self.ingot()?;
        let tree = fe_hir::lower::module_tree(db, ingot);
        tree.all_modules()
            .flat_map(|top| top.all_funcs(db).to_vec())
            .find(|func| {
                place(db, *func).is_some_and(|place| {
                    place.path == root.path && place.owner == root.owner && place.name == root.name
                })
            })
    }
}
