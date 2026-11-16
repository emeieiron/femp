//! Lowers the functions claims name, and every function they reach, into
//! one VIR program. A function the prover has no model for fails every root
//! that reaches it, with the location of the construct.

mod body;
mod contract;
pub(crate) mod evm;

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

pub use contract::{ContractRoot, Handler, Invariant, LoweredContract};

/// The program for a set of roots, and for each root its function or why it
/// cannot be lowered; likewise for each contract.
pub struct Lowered {
    pub program: Program,
    pub roots: Vec<Result<FuncId, Unsupported>>,
    pub contracts: Vec<Result<LoweredContract, Unsupported>>,
}

impl Session {
    pub fn lower(&self, roots: &[Root]) -> Lowered {
        self.lower_with(roots, &[])
    }

    /// Lowers `roots`, and each of `contracts` with its invariants, into
    /// one program.
    pub fn lower_with(&self, roots: &[Root], contracts: &[ContractRoot]) -> Lowered {
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
            states: HashMap::new(),
            ingot: self.ingot(),
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
        // Every context field any handler reads is known only once all of
        // them are lowered.
        let pending: Vec<_> = contracts
            .iter()
            .map(|root| {
                let contract = self.find_contract(root).ok_or_else(|| {
                    Unsupported::new(
                        Span::default(),
                        format!("the contract `{}` was not found", root.name),
                    )
                })?;
                lowerer.contract(contract, root)
            })
            .collect();
        let contracts = pending
            .into_iter()
            .map(|pending| pending.map(|pending| pending.wrap(&mut lowerer)))
            .collect();
        lowerer.program.types = lowerer.types.table;
        Lowered {
            program: lowerer.program,
            roots: results,
            contracts,
        }
    }

    fn find_contract(&self, root: &ContractRoot) -> Option<hir::hir_def::Contract<'_>> {
        let db = &self.db;
        let tree = hir::lower::module_tree(db, self.ingot());
        tree.all_modules()
            .flat_map(|top| top.all_contracts(db).to_vec())
            .find(|contract| {
                let name = match contract.name(db) {
                    Partial::Present(name) => name.data(db).to_string(),
                    Partial::Absent => return false,
                };
                let path = contract
                    .scope()
                    .parent_module(db)
                    .and_then(|module| module.pretty_path(db))
                    .map(|path| path.split("::").skip(1).map(str::to_string).collect())
                    .unwrap_or_else(Vec::new);
                name == root.name && path == root.path
            })
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
    /// Each contract's storage, as a tuple of its fields.
    states: HashMap<hir::hir_def::Contract<'db>, Type>,
    ingot: hir::Ingot<'db>,
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
        match body::lower(self, instance, name, body::Role::Function) {
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

    /// A contract's storage: a tuple of its fields in declaration order.
    pub(crate) fn state_type(
        &mut self,
        contract: hir::hir_def::Contract<'db>,
        instance: SemanticInstance<'db>,
        span: &Span,
    ) -> Result<Type, Unsupported> {
        if let Some(found) = self.states.get(&contract) {
            return Ok(found.clone());
        }
        let mut fields: Vec<_> = contract.fields(self.db).values().cloned().collect();
        fields.sort_by_key(|field| field.index);
        let mut types = Vec::new();
        for field in fields {
            types.push(
                self.types
                    .lower(self.db, instance, field.declared)
                    .map_err(|message| Unsupported::new(span.clone(), message))?,
            );
        }
        let state = Type::Tuple(types);
        self.states.insert(contract, state.clone());
        Ok(state)
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
