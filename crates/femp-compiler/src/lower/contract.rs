//! A contract as a state machine over its storage, for the prover. Its
//! `init` and `recv` arms are lowered as handlers that return the storage
//! they leave, then wrapped into functions of machine shape: `init` from
//! the context and its arguments to the first storage, each arm from the
//! storage, the context and the message to the next storage, and each
//! `#[invariant]` from the storage to `bool`. The context fields any code
//! reads become arguments, so a counterexample names them.

use std::collections::BTreeSet;

use femp_core::property::{Param, Ty, Value};
use femp_vir::{Const, Cursor, Expr, FuncId, Function, Proj, Span, Type, TypeDef, TypeTable};
use hir::analysis::semantic::{
    NValueDefinition, SemanticInstance, normalize_semantic_body, root_semantic_instance_key,
};
use hir::analysis::ty::adt_def::AdtRef;
use hir::analysis::ty::ty_check::BodyOwner;
use hir::hir_def::{Contract, FuncParamName, Partial};

use super::Lowerer;
use super::body::Role;
use super::evm::{CONTEXT, VALUE};
use crate::error::Unsupported;
use crate::find::Root;
use crate::spans;

/// A contract claims name, with the `#[invariant]` functions that state
/// what its storage keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRoot {
    /// The module path of the contract inside its ingot.
    pub path: Vec<String>,
    pub name: String,
    pub invariants: Vec<Root>,
}

#[derive(Debug, Clone)]
pub struct LoweredContract {
    /// The storage: a tuple of the contract's fields.
    pub state: Type,
    pub fields: Vec<String>,
    /// From the context and its arguments to the storage of a new contract.
    pub init: Handler,
    /// From the storage, the context and a message to the next storage.
    pub arms: Vec<Handler>,
    /// Each invariant as a function of the storage.
    pub invariants: Vec<FuncId>,
}

#[derive(Debug, Clone)]
pub struct Handler {
    /// `init`, or the message variant an arm receives.
    pub name: String,
    pub func: FuncId,
    /// The context fields the handler reads, then its message's fields.
    pub params: Vec<Param>,
    /// How many of `params` are context fields.
    pub context: usize,
    pub payable: bool,
    /// The selector an arm is called with.
    pub selector: Option<u32>,
}

/// A handler lowered, before it is wrapped.
struct Pending {
    name: String,
    func: FuncId,
    params: Vec<Param>,
    payable: bool,
    selector: Option<u32>,
}

/// A contract whose handlers and invariants are lowered.
pub(super) struct Lowering {
    state: Type,
    fields: Vec<String>,
    init: Option<Pending>,
    arms: Vec<Pending>,
    /// Each invariant and the storage field of each of its parameters.
    invariants: Vec<(FuncId, Vec<usize>)>,
    name: String,
}

impl<'db> Lowerer<'db> {
    pub(super) fn contract(
        &mut self,
        contract: Contract<'db>,
        root: &ContractRoot,
    ) -> Result<Lowering, Unsupported> {
        let db = self.db;
        let span = spans::of_contract(db, contract);
        let mut fields: Vec<_> = contract.fields(db).values().cloned().collect();
        fields.sort_by_key(|field| field.index);
        let names: Vec<String> = fields
            .iter()
            .map(|field| field.name.data(db).to_string())
            .collect();

        let init = match contract.init(db) {
            Some(init) => {
                let owner = BodyOwner::ContractInit { contract };
                let instance = instance(db, owner, &span)?;
                let state = self.state_type(contract, instance, &span)?;
                let params = self.init_params(instance, init, &span)?;
                let func =
                    self.handler(instance, format!("{}::init", root.name), &params, state)?;
                Some(Pending {
                    name: "init".to_string(),
                    func,
                    params,
                    payable: init.is_payable(db),
                    selector: None,
                })
            }
            None => None,
        };

        let sol = crate::evm::sol_abi_ty(db, contract).ok();
        let mut arms = Vec::new();
        for recv in contract.recv_views(db) {
            for arm in recv.arms(db) {
                let owner = BodyOwner::ContractRecvArm {
                    contract,
                    recv_idx: recv.recv_idx(db),
                    arm_idx: arm.arm_idx(db),
                };
                let instance = instance(db, owner, &span)?;
                let state = self.state_type(contract, instance, &span)?;
                let (name, params) = self.message(instance, arm, &span)?;
                let func =
                    self.handler(instance, format!("{}::{name}", root.name), &params, state)?;
                arms.push(Pending {
                    name,
                    func,
                    params,
                    payable: arm.arm(db).is_some_and(|arm| arm.is_payable(db)),
                    selector: sol.and_then(|sol| arm.abi_info(db, sol).selector_value),
                });
            }
        }

        let state = match init.as_ref().or(arms.first()) {
            Some(handler) => self.program.function(handler.func).result.clone(),
            None => Type::Tuple(Vec::new()),
        };
        let mut invariants = Vec::new();
        for invariant in &root.invariants {
            let func = self.find_root(invariant)?;
            let id = self.root(func)?;
            self.check_reachable(id)?;
            let mut indexes = Vec::new();
            let function = self.program.function(id);
            for (param, name) in function.params.iter().zip(param_names(db, func)) {
                let field = names
                    .iter()
                    .position(|field| *field == name)
                    .ok_or_else(|| {
                        Unsupported::new(
                            function.span.clone(),
                            format!(
                                "`{}` has a parameter `{name}`, but `{}` has no field of that name",
                                invariant.name, root.name
                            ),
                        )
                    })?;
                let Type::Tuple(types) = &state else {
                    unreachable!("the storage is a tuple");
                };
                if function.ty(*param) != &types[field] {
                    return Err(Unsupported::new(
                        function.span.clone(),
                        format!("the parameter `{name}` must have the type of the field `{name}`"),
                    ));
                }
                indexes.push(field);
            }
            if function.result != Type::Bool {
                return Err(Unsupported::new(
                    function.span.clone(),
                    "an `#[invariant]` must return `bool`",
                ));
            }
            invariants.push((id, indexes));
        }
        Ok(Lowering {
            state,
            fields: names,
            init,
            arms,
            invariants,
            name: root.name.clone(),
        })
    }

    fn find_root(&self, root: &Root) -> Result<hir::hir_def::Func<'db>, Unsupported> {
        let db = self.db;
        let tree = hir::lower::module_tree(db, self.ingot);
        tree.all_modules()
            .flat_map(|top| top.all_funcs(db).to_vec())
            .find(|func| {
                crate::find::place(db, *func).is_some_and(|place| {
                    place.path == root.path && place.owner == root.owner && place.name == root.name
                })
            })
            .ok_or_else(|| {
                Unsupported::new(Span::default(), format!("`{}` was not found", root.name))
            })
    }

    fn handler(
        &mut self,
        instance: SemanticInstance<'db>,
        name: String,
        params: &[Param],
        state: Type,
    ) -> Result<FuncId, Unsupported> {
        let owner = instance.key(self.db).owner(self.db);
        let id = self.program.push(Function::new(
            name.clone(),
            state.clone(),
            spans::of_owner(self.db, owner),
        ));
        self.functions.insert(instance, id);
        let role = Role::Handler {
            params: params.iter().map(|param| vir_type(param.ty)).collect(),
            state,
        };
        match super::body::lower(self, instance, name, role) {
            Ok(function) => *self.program.function_mut(id) = function,
            Err(error) => {
                self.failed.insert(id, error.clone());
                return Err(error);
            }
        }
        self.check_reachable(id)?;
        Ok(id)
    }

    fn init_params(
        &mut self,
        instance: SemanticInstance<'db>,
        init: hir::hir_def::ContractInit<'db>,
        span: &Span,
    ) -> Result<Vec<Param>, Unsupported> {
        let db = self.db;
        let names: Vec<String> = init
            .params(db)
            .data(db)
            .iter()
            .map(|param| match param.name {
                Partial::Present(FuncParamName::Ident(name)) => name.data(db).to_string(),
                _ => "_".to_string(),
            })
            .collect();
        let artifacts = normalize_semantic_body(db, instance).map_err(|failure| {
            Unsupported::new(
                span.clone(),
                format!("the compiler could not prepare `init` for proving ({failure:?})"),
            )
        })?;
        let mut params = Vec::new();
        for (index, name) in names.into_iter().enumerate() {
            let value = artifacts.body.values.iter().find(|value| {
                matches!(value.definition, NValueDefinition::EntryParam { param } if param as usize == index)
            });
            let Some(value) = value else {
                return Err(Unsupported::new(
                    span.clone(),
                    format!("the parameter `{name}` of `init` is never read"),
                ));
            };
            let ty = self
                .types
                .lower(db, instance, value.ty)
                .map_err(|message| Unsupported::new(span.clone(), message))?;
            params.push(param(name, &ty, span)?);
        }
        Ok(params)
    }

    /// The name of the message an arm receives, and its fields.
    fn message(
        &mut self,
        instance: SemanticInstance<'db>,
        arm: hir::semantic::RecvArmView<'db>,
        span: &Span,
    ) -> Result<(String, Vec<Param>), Unsupported> {
        let db = self.db;
        if arm.is_fallback(db) {
            return Ok(("fallback".to_string(), Vec::new()));
        }
        let variant = arm.variant_ty(db);
        let item = match variant.adt_def(db).map(|def| def.adt_ref(db)) {
            Some(AdtRef::Struct(item)) => item,
            _ => {
                return Err(Unsupported::new(
                    span.clone(),
                    format!("the message `{}` is not a struct", variant.pretty_print(db)),
                ));
            }
        };
        let name = match item.name(db) {
            Partial::Present(name) => name.data(db).to_string(),
            Partial::Absent => "message".to_string(),
        };
        let names: Vec<String> = item
            .hir_fields(db)
            .data(db)
            .iter()
            .map(|field| match field.name {
                Partial::Present(name) => name.data(db).to_string(),
                Partial::Absent => "_".to_string(),
            })
            .collect();
        let mut params = Vec::new();
        for (field, ty) in names
            .into_iter()
            .zip(instance.normalized_field_types(db, variant).iter())
        {
            let ty = self
                .types
                .lower(db, instance, *ty)
                .map_err(|message| Unsupported::new(span.clone(), message))?;
            params.push(param(field, &ty, span)?);
        }
        Ok((name, params))
    }
}

fn instance<'db>(
    db: &'db driver::DriverDataBase,
    owner: BodyOwner<'db>,
    span: &Span,
) -> Result<SemanticInstance<'db>, Unsupported> {
    let key = root_semantic_instance_key(db, owner).map_err(|_| {
        Unsupported::new(span.clone(), "the compiler has no instance of this handler")
    })?;
    Ok(SemanticInstance::new(db, key))
}

fn param_names<'db>(db: &'db driver::DriverDataBase, func: hir::hir_def::Func<'db>) -> Vec<String> {
    func.params(db)
        .map(|param| {
            param
                .name(db)
                .map_or_else(|| "_".to_string(), |name| name.data(db).to_string())
        })
        .collect()
}

fn param(name: String, ty: &Type, span: &Span) -> Result<Param, Unsupported> {
    let ty = match ty {
        Type::Bool => Ty::Bool,
        Type::Int {
            bits,
            signed: false,
        } => Ty::Uint(*bits),
        Type::Int { bits, signed: true } => Ty::Int(*bits),
        _ => {
            return Err(Unsupported::new(
                span.clone(),
                format!(
                    "`{name}` is not an integer or `bool`, which is all a contract's messages \
                     may carry for now"
                ),
            ));
        }
    };
    Ok(Param {
        name,
        label: None,
        ty,
        generator: "any".to_string(),
    })
}

fn vir_type(ty: Ty) -> Type {
    match ty {
        Ty::Bool => Type::Bool,
        Ty::Uint(bits) => Type::uint(bits),
        Ty::Int(bits) => Type::int(bits),
    }
}

impl Lowering {
    /// Wraps the handlers once every context field any code reads is known.
    pub(super) fn wrap(self, lx: &mut Lowerer<'_>) -> LoweredContract {
        let evm = lx.types.evm();
        let table = lx.types.table.clone();
        let init = match &self.init {
            Some(pending) => wrap_handler(lx, &self, pending, &evm, &table, false),
            None => {
                let mut function = Function::new(
                    format!("{}::init", self.name),
                    self.state.clone(),
                    Span::default(),
                );
                let block = function.entry;
                let mut cursor = Cursor::new(&mut function, block);
                let state = zero(&mut cursor, &table, &self.state);
                cursor.terminate(femp_vir::Terminator::Return(state));
                Handler {
                    name: "init".to_string(),
                    func: lx.program.push(function),
                    params: Vec::new(),
                    context: 0,
                    payable: false,
                    selector: None,
                }
            }
        };
        let arms = self
            .arms
            .iter()
            .map(|pending| wrap_handler(lx, &self, pending, &evm, &table, true))
            .collect();
        let invariants = self
            .invariants
            .iter()
            .map(|(func, fields)| {
                let name = lx.program.function(*func).name.clone();
                let mut function = Function::new(
                    format!("{}::{name}", self.name),
                    Type::Bool,
                    lx.program.function(*func).span.clone(),
                );
                let storage = function.new_value(self.state.clone(), Some("storage".to_string()));
                function.params.push(storage);
                let block = function.entry;
                let mut cursor = Cursor::new(&mut function, block);
                let Type::Tuple(types) = &self.state else {
                    unreachable!("the storage is a tuple");
                };
                let args = fields
                    .iter()
                    .map(|field| {
                        cursor.def(
                            types[*field].clone(),
                            Expr::Project {
                                value: storage,
                                path: vec![Proj::Field(*field as u16)],
                            },
                        )
                    })
                    .collect();
                let holds = cursor.call(*func, args, Type::Bool);
                cursor.terminate(femp_vir::Terminator::Return(holds));
                lx.program.push(function)
            })
            .collect();
        LoweredContract {
            state: self.state,
            fields: self.fields,
            init,
            arms,
            invariants,
        }
    }
}

fn wrap_handler(
    lx: &mut Lowerer<'_>,
    contract: &Lowering,
    pending: &Pending,
    evm: &Type,
    table: &TypeTable,
    takes_storage: bool,
) -> Handler {
    let handler = lx.program.function(pending.func);
    let mut function = Function::new(
        handler.name.clone(),
        contract.state.clone(),
        handler.span.clone(),
    );
    let storage = takes_storage.then(|| {
        let value = function.new_value(contract.state.clone(), Some("storage".to_string()));
        function.params.push(value);
        value
    });
    let context: Vec<usize> = context_reads(&lx.program, pending.func, evm)
        .into_iter()
        .filter(|field| pending.payable || *field != VALUE)
        .collect();
    let mut params = Vec::new();
    let mut context_values = Vec::new();
    for field in &context {
        let name = CONTEXT[*field].1.to_string();
        let value = function.new_value(Type::uint(256), Some(name.clone()));
        function.params.push(value);
        context_values.push((*field, value));
        params.push(Param {
            name,
            label: None,
            ty: Ty::Uint(256),
            generator: "any".to_string(),
        });
    }
    let mut message = Vec::new();
    for param in &pending.params {
        let value = function.new_value(vir_type(param.ty), Some(param.name.clone()));
        function.params.push(value);
        message.push(value);
        params.push(param.clone());
    }
    let block = function.entry;
    let mut cursor = Cursor::new(&mut function, block);
    let fields = (0..CONTEXT.len())
        .map(
            |field| match context_values.iter().find(|(read, _)| *read == field) {
                Some((_, value)) => *value,
                None if field == VALUE => cursor.int(
                    &Type::uint(256),
                    Value::from_u128(Ty::Uint(256), 0).expect("zero is a u256"),
                ),
                None => cursor.havoc(Type::uint(256), None),
            },
        )
        .collect();
    let env = cursor.def(
        evm.clone(),
        Expr::Make {
            ty: evm.clone(),
            fields,
        },
    );
    let state = match storage {
        Some(storage) => storage,
        None => zero(&mut cursor, table, &contract.state),
    };
    let mut args = message;
    args.push(env);
    args.push(state);
    let next = cursor.call(pending.func, args, contract.state.clone());
    cursor.terminate(femp_vir::Terminator::Return(next));
    Handler {
        name: pending.name.clone(),
        func: lx.program.push(function),
        context: context.len(),
        params,
        payable: pending.payable,
        selector: pending.selector,
    }
}

/// The context fields the code `root` reaches reads.
fn context_reads(program: &femp_vir::Program, root: FuncId, evm: &Type) -> BTreeSet<usize> {
    let mut reads = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let function = program.function(id);
        for stmt in function.blocks.iter().flat_map(|block| &block.stmts) {
            let femp_vir::StmtKind::Def { expr, .. } = &stmt.kind else {
                continue;
            };
            match expr {
                Expr::Call { func, .. } => stack.push(*func),
                Expr::Project { value, path } if function.ty(*value) == evm => {
                    if let Some(Proj::Field(field)) = path.first() {
                        reads.insert(*field as usize);
                    }
                }
                _ => {}
            }
        }
    }
    reads
}

/// The value of storage nothing has written: zero, `false`, and the first
/// variant of an enum.
fn zero(cursor: &mut Cursor<'_>, table: &TypeTable, ty: &Type) -> femp_vir::ValueId {
    match ty {
        Type::Bool => cursor.boolean(false),
        Type::Int { bits, signed } => {
            let shape = if *signed {
                Ty::Int(*bits)
            } else {
                Ty::Uint(*bits)
            };
            cursor.int(
                ty,
                Value::from_u128(shape, 0).expect("zero fits every integer"),
            )
        }
        Type::Unit => cursor.def(Type::Unit, Expr::Const(Const::Unit)),
        Type::Tuple(items) => {
            let fields = items.iter().map(|item| zero(cursor, table, item)).collect();
            cursor.def(
                ty.clone(),
                Expr::Make {
                    ty: ty.clone(),
                    fields,
                },
            )
        }
        Type::Array { elem, len } => {
            let value = zero(cursor, table, elem);
            cursor.def(ty.clone(), Expr::Repeat { value, len: *len })
        }
        Type::Adt(id) => match table.get(*id) {
            TypeDef::Struct { fields, .. } => {
                let fields = fields
                    .clone()
                    .iter()
                    .map(|field| zero(cursor, table, &field.ty))
                    .collect();
                cursor.def(
                    ty.clone(),
                    Expr::Make {
                        ty: ty.clone(),
                        fields,
                    },
                )
            }
            TypeDef::Enum { variants, .. } => {
                let fields = variants
                    .first()
                    .map(|variant| variant.fields.clone())
                    .unwrap_or_default()
                    .iter()
                    .map(|field| zero(cursor, table, field))
                    .collect();
                cursor.def(
                    ty.clone(),
                    Expr::MakeEnum {
                        ty: ty.clone(),
                        variant: 0,
                        fields,
                    },
                )
            }
        },
    }
}
