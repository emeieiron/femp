//! Lowers the functions claims name, and every function they reach, into
//! one VIR program. A function the prover has no model for fails every root
//! that reaches it, with the location of the construct.

mod body;

use std::collections::{BTreeSet, HashMap};

use common::ingot::IngotKind;
use driver::DriverDataBase;
use femp_vir::{Expr, FuncId, Function, Program, Span, StmtKind, Type};
use hir::analysis::semantic::{SemanticInstance, root_semantic_instance_key};
use hir::analysis::ty::ty_check::BodyOwner;
use hir::hir_def::{Func, Partial};

use crate::error::Unsupported;
use crate::find::{Root, place};
use crate::session::Session;
use crate::spans;
use crate::types::Types;

/// The program for a set of roots, and for each root its function or why it
/// cannot be lowered.
pub struct Lowered {
    pub program: Program,
    pub roots: Vec<Result<FuncId, Unsupported>>,
}

impl Session {
    pub fn lower(&self, roots: &[Root]) -> Lowered {
        let mut lowerer = Lowerer {
            db: &self.db,
            root_ingot: ingot_name(&self.db, self.ingot()).unwrap_or_default(),
            types: Types::new(self.target.word_bits()),
            program: Program {
                target: self.target.name().to_string(),
                ..Program::default()
            },
            functions: HashMap::new(),
            failed: HashMap::new(),
            names: BTreeSet::new(),
        };
        let results = roots
            .iter()
            .map(|root| {
                let func = self.find(root).ok_or_else(|| {
                    Unsupported::new(Span::default(), format!("`{}` was not found", label(root)))
                })?;
                let id = lowerer.root(func)?;
                lowerer.check_reachable(id)?;
                Ok(id)
            })
            .collect();
        lowerer.program.types = lowerer.types.table;
        Lowered {
            program: lowerer.program,
            roots: results,
        }
    }
}

/// The name an ingot's `fe.toml` gives it.
pub(crate) fn ingot_name(db: &DriverDataBase, ingot: hir::Ingot<'_>) -> Option<String> {
    ingot
        .config(db)
        .as_ref()
        .and_then(|config| config.metadata.name.as_ref())
        .map(|name| name.to_string())
}

fn label(root: &Root) -> String {
    root.path
        .iter()
        .map(String::as_str)
        .chain(root.owner.as_deref())
        .chain(std::iter::once(root.name.as_str()))
        .collect::<Vec<_>>()
        .join("::")
}

pub(crate) struct Lowerer<'db> {
    db: &'db DriverDataBase,
    root_ingot: String,
    types: Types<'db>,
    program: Program,
    functions: HashMap<SemanticInstance<'db>, FuncId>,
    failed: HashMap<FuncId, Unsupported>,
    names: BTreeSet<String>,
}

impl<'db> Lowerer<'db> {
    fn root(&mut self, func: Func<'db>) -> Result<FuncId, Unsupported> {
        let owner = BodyOwner::Func(func, None);
        let key = root_semantic_instance_key(self.db, owner).map_err(|_| {
            Unsupported::new(
                spans::of_owner(self.db, owner),
                "the function is generic or declares effects, which the prover does not support",
            )
        })?;
        self.function(SemanticInstance::new(self.db, key))
    }

    /// The function of an instance, lowered on first use. An instance being
    /// lowered already has its id, so recursion terminates here and is
    /// rejected by the prover later.
    pub(crate) fn function(
        &mut self,
        instance: SemanticInstance<'db>,
    ) -> Result<FuncId, Unsupported> {
        if let Some(id) = self.functions.get(&instance) {
            return match self.failed.get(id) {
                Some(error) => Err(error.clone()),
                None => Ok(*id),
            };
        }
        let owner = instance.key(self.db).owner(self.db);
        let name = self.unique_name(owner);
        let id = self.program.push(Function::new(
            name.clone(),
            Type::Unit,
            spans::of_owner(self.db, owner),
        ));
        self.functions.insert(instance, id);
        match body::lower(self, instance, name) {
            Ok(function) => {
                *self.program.function_mut(id) = function;
                Ok(id)
            }
            Err(error) => {
                self.failed.insert(id, error.clone());
                Err(error)
            }
        }
    }

    fn unique_name(&mut self, owner: BodyOwner<'db>) -> String {
        let base = match owner {
            BodyOwner::Func(func, _) => self.func_name(func),
            _ => "<body>".to_string(),
        };
        let mut name = base.clone();
        let mut counter = 1;
        while !self.names.insert(name.clone()) {
            counter += 1;
            name = format!("{base}#{counter}");
        }
        name
    }

    fn func_name(&self, func: Func<'db>) -> String {
        let db = self.db;
        let ingot = func.top_mod(db).ingot(db);
        let ingot_name = ingot_name(db, ingot).unwrap_or_else(|| match ingot.kind(db) {
            IngotKind::Core => "core".to_string(),
            IngotKind::Std => "std".to_string(),
            _ => "ingot".to_string(),
        });
        let place = place(db, func);
        let mut segments: Vec<String> = Vec::new();
        if ingot_name != self.root_ingot {
            segments.push(ingot_name);
        }
        match place {
            Some(place) => {
                segments.extend(place.path);
                segments.extend(place.owner);
                segments.push(place.name);
            }
            None => segments.push(match func.name(db) {
                Partial::Present(name) => name.data(db).to_string(),
                Partial::Absent => "<fn>".to_string(),
            }),
        }
        segments.join("::")
    }

    /// Fails a root that reaches a function which could not be lowered.
    fn check_reachable(&self, root: FuncId) -> Result<(), Unsupported> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(error) = self.failed.get(&id) {
                return Err(error.clone());
            }
            for block in &self.program.function(id).blocks {
                for stmt in &block.stmts {
                    if let StmtKind::Def {
                        expr: Expr::Call { func, .. },
                        ..
                    } = &stmt.kind
                    {
                        stack.push(*func);
                    }
                }
            }
        }
        Ok(())
    }
}
