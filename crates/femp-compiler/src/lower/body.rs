//! One normalized body as a VIR function. Values keep their SSA identity;
//! local slots stay places until the prover's slots pass; views and shared
//! borrows become the value they read, since nothing can change it while
//! they live; a projection's yield returns what it grants.

use std::collections::HashMap;

use cranelift_entity::EntityRef;

use common::ingot::IngotKind;
use femp_core::property::PanicKind;
use femp_vir::{
    Arith, BinOp, BlockId, Const, Expr, Function, Index, Proj, SlotId, Span, Stmt, StmtKind, Succ,
    Terminator, Type, UnOp, ValueId,
};
use hir::analysis::semantic::{
    BinaryOp, FailReason, NBlockId, NDataProjection, NEffectArg, NEffectArgValue, NExpr, NIndex,
    NOperand, NPlace, NPlaceBase, NRootId, NRootKind, NStatement, NStatementKind, NSuccessor,
    NTerminatorKind, NValueDefinition, NValueId, NormalizedBody, Overflow, SemanticInstance,
    UnaryOp, normalize_semantic_body,
};
use hir::analysis::ty::ty_check::BodyOwner;
use hir::analysis::ty::ty_def::BorrowKind;
use hir::core::semantic::ProviderSource;
use hir::hir_def::{CompBinOp, Partial};

mod consts;

use super::Lowerer;
use super::evm::{self, is_evm};
use crate::error::Unsupported;
use crate::intrinsics::{self, Model};
use crate::spans;
use crate::spec_ops::{SPEC_INGOT, SpecCall, spec_call};

type Lowering<T> = Result<T, Unsupported>;

/// What a body is lowered as.
pub(super) enum Role {
    Function,
    /// A contract's `init` or a `recv` arm: it takes the message's fields,
    /// then the context and the storage, and returns the storage it leaves.
    Handler {
        params: Vec<Type>,
        state: Type,
        /// The pattern bindings of the message's fields, and the parameter
        /// each starts as.
        bindings: Vec<(hir::hir_def::PatId, usize)>,
    },
}

/// The parameters an ordinary function takes after its own, because its
/// body reaches what a contract provides: the context, the storage, and
/// the effects its callers pass.
#[derive(Debug, Default)]
pub(super) struct Hidden<'db> {
    env: bool,
    state: Option<hir::hir_def::Contract<'db>>,
    /// Whether the function may write the storage it is passed.
    writes_state: bool,
    /// The effects passed in, by their index among the function's effects.
    effects: Vec<u32>,
}

/// What an ordinary function returns besides its result: the final value of
/// each `mut` parameter, then the storage, when it writes it. Callers write
/// them back to the places they borrowed, which no one else can reach while
/// the borrow lives.
#[derive(Debug, Default)]
pub(super) struct Shape {
    /// The indexes of the `mut` parameters.
    mutable: Vec<usize>,
    writes_state: bool,
}

impl Shape {
    fn is_plain(&self) -> bool {
        self.mutable.is_empty() && !self.writes_state
    }
}

pub(super) fn shape<'db>(
    db: &'db driver::DriverDataBase,
    instance: SemanticInstance<'db>,
    body: &NormalizedBody<'db>,
) -> Shape {
    // A parameter is lent when the body receives it as a mutable borrow.
    let mut mutable: Vec<usize> = body
        .values
        .iter()
        .filter_map(|value| match value.definition {
            NValueDefinition::EntryParam { param } => {
                let ty = instance.normalized_ty(db, value.ty);
                mut_target(db, ty)
                    .is_some_and(|target| !is_evm(db, target))
                    .then_some(param as usize)
            }
            _ => None,
        })
        .collect();
    mutable.sort();
    mutable.dedup();
    Shape {
        mutable,
        writes_state: hidden(body).writes_state,
    }
}

pub(super) fn hidden<'db>(body: &NormalizedBody<'db>) -> Hidden<'db> {
    let mut hidden = Hidden::default();
    for root in &body.roots {
        let NRootKind::Provider { binding } = &root.kind else {
            continue;
        };
        match &binding.source {
            ProviderSource::RootProvider { .. } => hidden.env = true,
            ProviderSource::ContractField { field } => {
                hidden.state = Some(field.contract);
                hidden.writes_state |= binding.is_mut;
            }
            ProviderSource::UsesParam {
                requirement_idx, ..
            } => {
                if !hidden.effects.contains(requirement_idx) {
                    hidden.effects.push(*requirement_idx);
                }
            }
        }
    }
    hidden.effects.sort();
    hidden
}

/// What a mutable borrow of type `ty` borrows, if `ty` is one.
fn mut_target<'db>(
    db: &'db driver::DriverDataBase,
    ty: hir::analysis::ty::ty_def::TyId<'db>,
) -> Option<hir::analysis::ty::ty_def::TyId<'db>> {
    if let Some((hir::analysis::ty::ty_def::CapabilityKind::Mut, target)) = ty.as_capability(db) {
        return Some(target);
    }
    match ty.as_prim(db) {
        Some(hir::analysis::ty::ty_def::PrimTy::BorrowMut) => ty.generic_args(db).first().copied(),
        _ => None,
    }
}

pub(super) fn lower<'db>(
    lx: &mut Lowerer<'db>,
    instance: SemanticInstance<'db>,
    name: String,
    role: Role,
) -> Lowering<Function> {
    let db = lx.db;
    let owner = instance.key(db).owner(db);
    let span = spans::of_owner(db, owner);
    let artifacts = normalize_semantic_body(db, instance).map_err(|failure| {
        Unsupported::new(
            span.clone(),
            format!("the compiler could not prepare this function for proving ({failure:?})"),
        )
    })?;
    let body = &artifacts.body;
    let declared = instance.normalized_result_ty(db);
    if mut_target(db, declared).is_some() {
        return Err(Unsupported::new(
            span.clone(),
            "functions that return a mutable borrow are not supported yet",
        ));
    }
    let shape = match &role {
        Role::Function => shape(db, instance, body),
        Role::Handler { .. } => Shape::default(),
    };
    let plain = match &role {
        Role::Function => lx
            .types
            .lower(db, instance, declared)
            .map_err(|message| Unsupported::new(span.clone(), message))?,
        Role::Handler { state, .. } => state.clone(),
    };
    let result = if shape.is_plain() {
        plain.clone()
    } else {
        let mut parts = vec![plain.clone()];
        for index in &shape.mutable {
            let value = body.values.iter().find(|value| {
                matches!(value.definition, NValueDefinition::EntryParam { param } if param as usize == *index)
            });
            if let Some(value) = value {
                parts.push(
                    lx.types
                        .lower(db, instance, value.ty)
                        .map_err(|message| Unsupported::new(span.clone(), message))?,
                );
            }
        }
        if shape.writes_state
            && let Some(contract) = hidden(body).state
        {
            parts.push(lx.state_type(contract, instance, &span)?);
        }
        Type::Tuple(parts)
    };
    let mut lowering = BodyLowering {
        lx,
        instance,
        owner,
        body,
        function: Function::new(name, result, span),
        values: HashMap::new(),
        blocks: HashMap::new(),
        slots: HashMap::new(),
        current: BlockId(0),
        role,
        env: None,
        state: None,
        storage: Vec::new(),
        effects: HashMap::new(),
        shape,
        mutable: Vec::new(),
        borrows: HashMap::new(),
    };
    lowering.run()?;
    Ok(lowering.function)
}

struct BodyLowering<'a, 'db> {
    lx: &'a mut Lowerer<'db>,
    instance: SemanticInstance<'db>,
    owner: BodyOwner<'db>,
    body: &'a NormalizedBody<'db>,
    function: Function,
    values: HashMap<NValueId, ValueId>,
    blocks: HashMap<NBlockId, BlockId>,
    slots: HashMap<NRootId, SlotId>,
    current: BlockId,
    role: Role,
    /// The EVM context the body reads.
    env: Option<ValueId>,
    /// The contract's storage as the body found it.
    state: Option<ValueId>,
    /// The slot holding each storage field, by field index.
    storage: Vec<SlotId>,
    /// The value of each effect the body was passed.
    effects: HashMap<NRootId, ValueId>,
    shape: Shape,
    /// The slot of each `mut` parameter, and the value it starts with.
    mutable: Vec<(SlotId, ValueId)>,
    /// What each mutable borrow the body holds points at.
    borrows: HashMap<NValueId, (SlotId, Vec<Proj>)>,
}

/// Where a place is: a part of a slot, or of a value standing for it.
enum Location {
    Slot(SlotId, Vec<Proj>),
    Value(ValueId, Vec<Proj>),
}

/// Where a place's base leads: a slot, or a value standing for the place.
enum Base {
    Slot(SlotId),
    Value(ValueId),
}

impl<'db> BodyLowering<'_, 'db> {
    fn span(&self, origin: hir::analysis::semantic::SemOrigin<'db>) -> Span {
        spans::origin(self.lx.db, self.owner, origin)
    }

    fn unsupported(&self, span: &Span, message: impl Into<String>) -> Unsupported {
        let span = if span.is_known() {
            span.clone()
        } else {
            self.function.span.clone()
        };
        Unsupported::new(span, message)
    }

    fn ty(&mut self, ty: hir::analysis::ty::ty_def::TyId<'db>, span: &Span) -> Lowering<Type> {
        self.lx
            .types
            .lower(self.lx.db, self.instance, ty)
            .map_err(|message| self.unsupported(span, message))
    }

    fn run(&mut self) -> Lowering<()> {
        let prologue = self.function.entry;
        let order = self.body.reverse_postorder();
        self.params()?;
        self.hidden_params()?;
        for block in &order {
            let id = self.function.new_block();
            self.blocks.insert(*block, id);
        }
        for block in &order {
            let id = self.blocks[block];
            for param in self.body.blocks[block.index()].params.iter() {
                let value = self.value(*param)?;
                self.function.block_mut(id).params.push(value);
            }
        }
        self.current = prologue;
        self.slots_init()?;
        let entry = self.blocks[&self.body.entry];
        self.terminate(Terminator::Goto(Succ::to(entry)), Span::default());
        for block in order {
            self.current = self.blocks[&block];
            let data = &self.body.blocks[block.index()];
            for statement in &data.statements {
                self.statement(statement)?;
            }
            let span = self.span(data.terminator.origin);
            let term = self.terminator(&data.terminator.kind, &span)?;
            self.terminate(term, span);
        }
        Ok(())
    }

    fn params(&mut self) -> Lowering<()> {
        let mut params: Vec<(u32, NValueId)> = self
            .body
            .values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| match value.definition {
                NValueDefinition::EntryParam { param } => Some((param, NValueId::new(index))),
                _ => None,
            })
            .collect();
        params.sort();
        // A handler takes every field of its message, read or not.
        if let Role::Handler { params: types, .. } = &self.role {
            let types = types.clone();
            for (index, ty) in types.into_iter().enumerate() {
                let id = match params.iter().find(|(param, _)| *param as usize == index) {
                    Some((_, value)) => self.value(*value)?,
                    None => self.function.new_value(ty, None),
                };
                self.function.params.push(id);
            }
            return Ok(());
        }
        // Every declared parameter is one, read or not, so that calls keep
        // their arity.
        let declared = match self.owner {
            BodyOwner::Func(func, _) => func.arg_tys(self.lx.db).len(),
            _ => params.len(),
        };
        for index in 0..declared.max(params.len()) {
            let read = params.iter().find(|(param, _)| *param as usize == index);
            let fe_ty = match (read, self.owner) {
                (Some((_, value)), _) => self.body.values[value.index()].ty,
                (None, BodyOwner::Func(func, _)) => {
                    func.arg_tys(self.lx.db)[index].instantiate_identity()
                }
                (None, _) => {
                    return Err(self.unsupported(
                        &self.function.span.clone(),
                        "a parameter the body never reads is not supported",
                    ));
                }
            };
            let id = match read {
                Some((_, value)) => self.value(*value)?,
                None => {
                    let span = self.function.span.clone();
                    let ty = self.ty(fe_ty, &span)?;
                    self.function.new_value(ty, None)
                }
            };
            // A method of the context reads it from its receiver.
            let target = fe_ty
                .as_capability(self.lx.db)
                .map_or(fe_ty, |(_, target)| target);
            if self.env.is_none() && is_evm(self.lx.db, target) {
                self.env = Some(id);
            }
            // A `mut` parameter lives in a slot that the function returns.
            if self.shape.mutable.contains(&index) {
                let slot = self.function.new_slot(self.function.ty(id).clone(), None);
                self.mutable.push((slot, id));
                if let Some((_, value)) = read {
                    self.borrows.insert(*value, (slot, Vec::new()));
                }
            }
            self.function.params.push(id);
        }
        Ok(())
    }

    /// The context, storage and effects the body takes after its own
    /// parameters, in the order `Hidden` lists them.
    fn hidden_params(&mut self) -> Lowering<()> {
        let span = self.function.span.clone();
        let (env, state) = match &self.role {
            Role::Handler { state, .. } => (true, Some(state.clone())),
            Role::Function => {
                let hidden = hidden(self.body);
                let state = match hidden.state {
                    Some(contract) => Some(self.lx.state_type(contract, self.instance, &span)?),
                    None => None,
                };
                (hidden.env, state)
            }
        };
        if env {
            let ty = self.lx.types.evm();
            let param = self.function.new_value(ty, Some("ctx".to_string()));
            self.function.params.push(param);
            self.env = Some(param);
        }
        if let Some(ty) = state {
            let param = self.function.new_value(ty, Some("storage".to_string()));
            self.function.params.push(param);
            self.state = Some(param);
        }
        if matches!(self.role, Role::Function) {
            for requirement in hidden(self.body).effects {
                let mut param = None;
                for (index, root) in self.body.roots.iter().enumerate() {
                    let NRootKind::Provider { binding } = &root.kind else {
                        continue;
                    };
                    let ProviderSource::UsesParam {
                        requirement_idx, ..
                    } = binding.source
                    else {
                        continue;
                    };
                    if requirement_idx != requirement {
                        continue;
                    }
                    let value = match param {
                        Some(value) => value,
                        None => {
                            let ty = self.ty(binding.effective_target_ty(), &span)?;
                            let value = self.function.new_value(ty, None);
                            self.function.params.push(value);
                            param = Some(value);
                            value
                        }
                    };
                    self.effects.insert(NRootId::new(index), value);
                }
            }
        }
        Ok(())
    }

    /// The result, with the final value of each `mut` parameter and the
    /// storage when the function writes it.
    fn with_lent(&mut self, result: ValueId, span: &Span) -> Lowering<ValueId> {
        let mut parts = vec![result];
        for (slot, _) in self.mutable.clone() {
            let ty = self.function.slots[slot.0 as usize].ty.clone();
            parts.push(self.fresh(
                ty,
                Expr::Load {
                    slot,
                    path: Vec::new(),
                },
                span.clone(),
            ));
        }
        if self.shape.writes_state {
            parts.push(self.current_state(span)?);
        }
        let ty = self.function.result.clone();
        Ok(self.fresh(ty.clone(), Expr::Make { ty, fields: parts }, span.clone()))
    }

    /// Lets the contract a call reaches call back into this one: any of the
    /// contract's messages may run in the meantime, from the storage as it
    /// stands, so the storage afterwards is what those leave. The prover
    /// gives the contract's `reenter` that meaning.
    fn reenter(&mut self, name: &str, span: &Span) -> Lowering<()> {
        let writes = match self.role {
            Role::Handler { .. } => true,
            Role::Function => self.shape.writes_state,
        };
        if self.state.is_none() || !writes {
            return Err(self.unsupported(
                span,
                format!(
                    "`{name}` calls another contract from a function that cannot write the contract's storage, which the callee could change by calling back"
                ),
            ));
        }
        let before = self.current_state(span)?;
        let ty = self.function.ty(before).clone();
        let func = self.lx.reentry(&ty, span);
        let after = self.fresh(
            ty,
            Expr::Call {
                func,
                args: vec![before],
            },
            span.clone(),
        );
        self.store_state(after, span)
    }

    /// Reverts or goes on, as the callee chooses.
    fn may_revert(&mut self, span: &Span) {
        let reverted = self.fresh(Type::Bool, Expr::Havoc, span.clone());
        let fail = self.function.new_block();
        let next = self.function.new_block();
        self.terminate(
            Terminator::Branch {
                cond: reverted,
                then: Succ::to(fail),
                otherwise: Succ::to(next),
            },
            span.clone(),
        );
        self.current = fail;
        self.terminate(Terminator::Fail(PanicKind::Revert), span.clone());
        self.current = next;
    }

    /// Writes each field of `state` to the field's slot.
    fn store_state(&mut self, state: ValueId, span: &Span) -> Lowering<()> {
        let Type::Tuple(fields) = self.function.ty(state).clone() else {
            return Err(self.unsupported(span, "storage that is not a tuple of fields"));
        };
        for (index, (slot, ty)) in self.storage.clone().into_iter().zip(fields).enumerate() {
            let value = self.fresh(
                ty,
                Expr::Project {
                    value: state,
                    path: vec![Proj::Field(index as u16)],
                },
                span.clone(),
            );
            self.push(
                StmtKind::Store {
                    slot,
                    path: Vec::new(),
                    value,
                },
                span.clone(),
            );
        }
        Ok(())
    }

    /// The storage as it stands in the body's slots.
    fn current_state(&mut self, span: &Span) -> Lowering<ValueId> {
        let state = self
            .state
            .ok_or_else(|| self.unsupported(span, "a call that needs the contract's storage"))?;
        let ty = self.function.ty(state).clone();
        let Type::Tuple(fields) = &ty else {
            return Err(self.unsupported(span, "storage that is not a tuple of fields"));
        };
        let mut values = Vec::new();
        for (slot, field_ty) in self.storage.clone().into_iter().zip(fields.clone()) {
            let expr = Expr::Load {
                slot,
                path: Vec::new(),
            };
            values.push(self.fresh(field_ty, expr, span.clone()));
        }
        let make = Expr::Make {
            ty: ty.clone(),
            fields: values,
        };
        Ok(self.fresh(ty, make, span.clone()))
    }

    /// Slots for the body's local storage, and the stores that give
    /// parameter places their value.
    fn slots_init(&mut self) -> Lowering<()> {
        let span = self.function.span.clone();
        for (slot, value) in self.mutable.clone() {
            self.push(
                StmtKind::Store {
                    slot,
                    path: Vec::new(),
                    value,
                },
                span.clone(),
            );
        }
        // Every field of the storage lives in a slot, since a callee may
        // write any of them.
        if let Some(state) = self.state {
            let Type::Tuple(fields) = self.function.ty(state).clone() else {
                return Err(self.unsupported(&span, "storage that is not a tuple of fields"));
            };
            for (index, ty) in fields.into_iter().enumerate() {
                let slot = self.function.new_slot(ty.clone(), None);
                let value = self.fresh(
                    ty,
                    Expr::Project {
                        value: state,
                        path: vec![Proj::Field(index as u16)],
                    },
                    span.clone(),
                );
                self.push(
                    StmtKind::Store {
                        slot,
                        path: Vec::new(),
                        value,
                    },
                    span.clone(),
                );
                self.storage.push(slot);
            }
        }
        for (index, root) in self.body.roots.iter().enumerate() {
            let id = NRootId::new(index);
            match &root.kind {
                NRootKind::LocalSlot { binding } => {
                    let span = self.span(root.origin);
                    let ty = self.ty(root.ty, &span)?;
                    let slot = self.function.new_slot(ty, None);
                    self.slots.insert(id, slot);
                    // A field the arm's pattern binds starts as the message's.
                    if let (
                        Role::Handler { bindings, .. },
                        Some(hir::analysis::ty::ty_check::LocalBinding::Local { pat, .. }),
                    ) = (&self.role, binding)
                        && let Some((_, param)) = bindings.iter().find(|(bound, _)| bound == pat)
                        && let Some(value) = self.function.params.get(*param).copied()
                    {
                        self.push(
                            StmtKind::Store {
                                slot,
                                path: Vec::new(),
                                value,
                            },
                            span,
                        );
                    }
                }
                NRootKind::ParamPlace { param } => {
                    let span = self.span(root.origin);
                    let ty = self.ty(root.ty, &span)?;
                    let slot = self.function.new_slot(ty, None);
                    self.slots.insert(id, slot);
                    let Some(value) = self.function.params.get(*param as usize).copied() else {
                        return Err(self.unsupported(&span, "a parameter kept only in memory"));
                    };
                    self.push(
                        StmtKind::Store {
                            slot,
                            path: Vec::new(),
                            value,
                        },
                        span,
                    );
                }
                NRootKind::Temporary { .. } | NRootKind::CapabilityRepresentation { .. } => {}
                NRootKind::Provider { binding } => {
                    let ProviderSource::ContractField { field } = binding.source else {
                        continue;
                    };
                    let span = self.span(root.origin);
                    let slot =
                        self.storage
                            .get(field.index as usize)
                            .copied()
                            .ok_or_else(|| {
                                self.unsupported(&span, "contract storage outside the contract")
                            })?;
                    self.slots.insert(id, slot);
                }
            }
        }
        Ok(())
    }

    fn value(&mut self, value: NValueId) -> Lowering<ValueId> {
        if let Some(id) = self.values.get(&value) {
            return Ok(*id);
        }
        let data = &self.body.values[value.index()];
        let span = self.span(data.origin);
        let ty = self.ty(data.ty, &span)?;
        let id = self.function.new_value(ty, None);
        self.values.insert(value, id);
        Ok(id)
    }

    fn operand(&mut self, operand: &NOperand) -> Lowering<ValueId> {
        // A borrow reads what its place holds now, which writes through it
        // may have changed.
        if let Some((slot, path)) = self.borrows.get(&operand.value).cloned() {
            let id = self.value(operand.value)?;
            let ty = self.function.ty(id).clone();
            return Ok(self.fresh(ty, Expr::Load { slot, path }, Span::default()));
        }
        self.value(operand.value)
    }

    fn push(&mut self, kind: StmtKind, span: Span) {
        self.function
            .block_mut(self.current)
            .stmts
            .push(Stmt { kind, span });
    }

    fn define(&mut self, dst: ValueId, expr: Expr, span: Span) {
        self.push(StmtKind::Def { dst, expr }, span);
    }

    fn fresh(&mut self, ty: Type, expr: Expr, span: Span) -> ValueId {
        let dst = self.function.new_value(ty, None);
        self.define(dst, expr, span);
        dst
    }

    fn terminate(&mut self, term: Terminator, span: Span) {
        let block = self.function.block_mut(self.current);
        block.term = term;
        block.span = span;
    }

    fn statement(&mut self, statement: &NStatement<'db>) -> Lowering<()> {
        let span = self.span(statement.origin);
        match &statement.kind {
            NStatementKind::Define { result, expr } => self.define_expr(*result, expr, span),
            NStatementKind::Store { destination, value } => {
                let value = self.operand(value)?;
                match self.locate(destination, &span)? {
                    Location::Slot(slot, path) => {
                        self.push(StmtKind::Store { slot, path, value }, span);
                        Ok(())
                    }
                    Location::Value(..) => Err(self
                        .unsupported(&span, "writing through a borrow of a value without a place")),
                }
            }
            NStatementKind::CheckIndex { index, array } => {
                let len = array.array_len(self.lx.db).ok_or_else(|| {
                    self.unsupported(&span, "an index into an array of unknown length")
                })?;
                let index = match index {
                    NIndex::Value(value) => self.value(*value)?,
                    NIndex::Const(index) => {
                        let ty = Type::uint(64);
                        let value = femp_core::property::Value::from_u128(
                            femp_core::property::Ty::Uint(64),
                            *index as u128,
                        )
                        .ok_or_else(|| self.unsupported(&span, "an index beyond 64 bits"))?;
                        self.fresh(ty, Expr::Const(Const::Int(value)), span.clone())
                    }
                };
                self.push(StmtKind::CheckIndex { index, len }, span);
                Ok(())
            }
            NStatementKind::End { .. } | NStatementKind::Drop { .. } => Ok(()),
        }
    }

    fn base(&mut self, place: &NPlace<'db>, span: &Span) -> Lowering<Base> {
        match place.base {
            NPlaceBase::Root(root) => {
                if let Some(slot) = self.slots.get(&root) {
                    return Ok(Base::Slot(*slot));
                }
                match self.body.roots[root.index()].kind.clone() {
                    NRootKind::Temporary { value }
                    | NRootKind::CapabilityRepresentation { carrier: value } => {
                        Ok(Base::Value(self.value(value)?))
                    }
                    NRootKind::Provider { binding } => match binding.source {
                        ProviderSource::RootProvider { .. } => {
                            self.env.map(Base::Value).ok_or_else(|| {
                                self.unsupported(span, "the EVM context outside a contract")
                            })
                        }
                        ProviderSource::UsesParam { .. } => self
                            .effects
                            .get(&root)
                            .copied()
                            .map(Base::Value)
                            .ok_or_else(|| {
                                self.unsupported(span, "an effect the function was not passed")
                            }),
                        ProviderSource::ContractField { .. } => {
                            Err(self.unsupported(span, "contract storage outside the contract"))
                        }
                    },
                    NRootKind::LocalSlot { .. } | NRootKind::ParamPlace { .. } => {
                        Err(self.unsupported(span, "a local place without storage"))
                    }
                }
            }
            NPlaceBase::CapabilityTarget { carrier } => Ok(Base::Value(self.value(carrier)?)),
            NPlaceBase::Deref { .. } => {
                Err(self.unsupported(span, "raw pointers are not supported"))
            }
        }
    }

    fn path(&mut self, path: &[NDataProjection], _span: &Span) -> Lowering<Vec<Proj>> {
        path.iter()
            .map(|projection| {
                Ok(match projection {
                    NDataProjection::Field(field) => Proj::Field(field.0),
                    NDataProjection::VariantField { variant, field } => Proj::VariantField {
                        variant: variant.0,
                        field: field.0,
                    },
                    NDataProjection::Index(NIndex::Const(index)) => {
                        Proj::Index(Index::Const(*index))
                    }
                    NDataProjection::Index(NIndex::Value(value)) => {
                        Proj::Index(Index::Value(self.value(*value)?))
                    }
                    NDataProjection::Entry(key) => Proj::Index(Index::Value(self.value(*key)?)),
                })
            })
            .collect()
    }

    /// The value at a place, read now.
    fn read(&mut self, place: &NPlace<'db>, span: &Span) -> Lowering<Expr> {
        Ok(match self.locate(place, span)? {
            Location::Slot(slot, path) => Expr::Load { slot, path },
            Location::Value(value, path) if path.is_empty() => Expr::Use(value),
            Location::Value(value, path) => Expr::Project { value, path },
        })
    }

    /// Where a place is, through the borrows the body holds.
    fn locate(&mut self, place: &NPlace<'db>, span: &Span) -> Lowering<Location> {
        let path = self.path(place.path.as_slice(), span)?;
        if let NPlaceBase::CapabilityTarget { carrier } = place.base
            && let Some((slot, mut prefix)) = self.borrows.get(&carrier).cloned()
        {
            prefix.extend(path);
            return Ok(Location::Slot(slot, prefix));
        }
        Ok(match self.base(place, span)? {
            Base::Slot(slot) => Location::Slot(slot, path),
            Base::Value(value) => Location::Value(value, path),
        })
    }

    fn define_expr(&mut self, result: NValueId, expr: &NExpr<'db>, span: Span) -> Lowering<()> {
        let lowered = match expr {
            NExpr::Forward { src } if self.borrows.contains_key(&src.value) => {
                let target = self.borrows[&src.value].clone();
                self.borrows.insert(result, target);
                Expr::Use(self.operand(src)?)
            }
            NExpr::Forward { src } => Expr::Use(self.operand(src)?),
            NExpr::ProjectValue { value, path } => Expr::Project {
                value: self.operand(value)?,
                path: self.path(path.0.as_slice(), &span)?,
            },
            NExpr::Load { place, .. } | NExpr::MakeView { place, .. } => self.read(place, &span)?,
            NExpr::Borrow { place, kind, .. } => match kind {
                BorrowKind::Ref => self.read(place, &span)?,
                BorrowKind::Mut if is_evm(self.lx.db, place.ty) => self.read(place, &span)?,
                BorrowKind::Mut => match self.locate(place, &span)? {
                    Location::Slot(slot, path) => {
                        self.borrows.insert(result, (slot, path.clone()));
                        Expr::Load { slot, path }
                    }
                    Location::Value(..) => {
                        return Err(
                            self.unsupported(&span, "a mutable borrow of a value without a place")
                        );
                    }
                },
            },
            NExpr::Const(constant) => {
                let dst = self.value(result)?;
                let ty = self.function.ty(dst).clone();
                let value = self.constant(constant, &ty, &span)?;
                Expr::Use(value)
            }
            NExpr::Unary { op, value } => Expr::Unary {
                op: match op {
                    UnaryOp::Neg(overflow) => UnOp::Neg(arith(*overflow)),
                    UnaryOp::Not => UnOp::Not,
                    UnaryOp::BitNot => UnOp::BitNot,
                },
                value: self.operand(value)?,
            },
            NExpr::Binary { op, lhs, rhs } => Expr::Binary {
                op: binary(*op),
                lhs: self.operand(lhs)?,
                rhs: self.operand(rhs)?,
            },
            NExpr::ScalarCast { value, to } => Expr::Cast {
                value: self.operand(value)?,
                to: self.ty(*to, &span)?,
            },
            NExpr::ArrayRepeat { ty, value } => Expr::Repeat {
                value: self.operand(value)?,
                len: ty
                    .array_len(self.lx.db)
                    .ok_or_else(|| self.unsupported(&span, "an array of unknown length"))?,
            },
            NExpr::AggregateMake { ty, fields } => Expr::Make {
                ty: self.ty(*ty, &span)?,
                fields: fields
                    .iter()
                    .map(|field| self.operand(field))
                    .collect::<Lowering<_>>()?,
            },
            NExpr::EnumMake {
                enum_ty,
                variant,
                fields,
            } => Expr::MakeEnum {
                ty: self.ty(*enum_ty, &span)?,
                variant: variant.0,
                fields: fields
                    .iter()
                    .map(|field| self.operand(field))
                    .collect::<Lowering<_>>()?,
            },
            NExpr::GetEnumTag { value } => Expr::Tag {
                value: self.operand(value)?,
            },
            NExpr::IsEnumVariant { value, variant } => Expr::IsVariant {
                value: self.operand(value)?,
                variant: variant.0,
            },
            NExpr::Call {
                callee,
                args,
                effect_args,
                ..
            } => {
                let operands = args.clone();
                let args = args
                    .iter()
                    .map(|arg| self.operand(arg))
                    .collect::<Lowering<Vec<_>>>()?;
                let instance = SemanticInstance::new(self.lx.db, callee.key);
                return self.call(result, instance, args, &operands, effect_args, span);
            }
            NExpr::StructuralRepack { .. } => {
                return Err(self.unsupported(&span, "structural repacking is not supported yet"));
            }
            NExpr::PointerCast { .. } => {
                return Err(self.unsupported(&span, "raw pointers are not supported"));
            }
            NExpr::CodeRegionRef { .. }
            | NExpr::CodeRegionOffset { .. }
            | NExpr::CodeRegionLen { .. } => {
                return Err(self.unsupported(&span, "code regions are not supported"));
            }
        };
        let dst = self.value(result)?;
        self.define(dst, lowered, span);
        Ok(())
    }

    fn call(
        &mut self,
        result: NValueId,
        instance: SemanticInstance<'db>,
        mut args: Vec<ValueId>,
        operands: &[NOperand],
        effect_args: &[NEffectArg<'db>],
        span: Span,
    ) -> Lowering<()> {
        let db = self.lx.db;
        let BodyOwner::Func(func, _) = instance.key(db).owner(db) else {
            return Err(self.unsupported(&span, "calls to closures are not supported yet"));
        };
        let name = match func.name(db) {
            Partial::Present(name) => name.data(db).to_string(),
            Partial::Absent => String::new(),
        };
        let ingot = func.top_mod(db).ingot(db);
        let ingot_name = super::ingot_name(db, ingot);
        let dst = self.value(result)?;
        if let Some(model) = evm::model(db, func) {
            let expr = match model {
                evm::Model::Revert => {
                    self.terminate(Terminator::Fail(PanicKind::Revert), span.clone());
                    self.current = self.function.new_block();
                    Expr::Const(Const::Unit)
                }
                evm::Model::Emit => Expr::Const(Const::Unit),
                evm::Model::Context(field) => {
                    let env = self.env.ok_or_else(|| {
                        self.unsupported(
                            &span,
                            format!("`{name}` reads the EVM context outside it"),
                        )
                    })?;
                    Expr::Project {
                        value: env,
                        path: vec![Proj::Field(field as u16)],
                    }
                }
                evm::Model::Unconstrained => Expr::Havoc,
                evm::Model::Call { reenters, bubbles } => {
                    self.lx.calls_out = true;
                    if reenters {
                        self.reenter(&name, &span)?;
                    }
                    if bubbles {
                        self.may_revert(&span);
                    }
                    Expr::Havoc
                }
                evm::Model::Delegate => {
                    return Err(self.unsupported(
                        &span,
                        format!(
                            "`{name}` runs another contract's code on this contract's storage, which the prover does not model"
                        ),
                    ));
                }
            };
            self.define(dst, expr, span);
            return Ok(());
        }
        if ingot_name.as_deref() == Some(SPEC_INGOT)
            && let Some(call) = spec_call(&name)
        {
            match call {
                SpecCall::Statement(op) => {
                    let cond = arg(&args, 0, &span).map_err(|m| self.unsupported(&span, m))?;
                    self.push(StmtKind::Spec { op, cond }, span.clone());
                    self.define(dst, Expr::Const(Const::Unit), span);
                }
                SpecCall::Implies => self.define(
                    dst,
                    Expr::Binary {
                        op: BinOp::Implies,
                        lhs: arg(&args, 0, &span).map_err(|m| self.unsupported(&span, m))?,
                        rhs: arg(&args, 1, &span).map_err(|m| self.unsupported(&span, m))?,
                    },
                    span,
                ),
            }
            return Ok(());
        }
        if func.is_extern(db) {
            let library = matches!(ingot.kind(db), IngotKind::Core | IngotKind::Std);
            let Some(model) = intrinsics::model(&name).filter(|_| library) else {
                return Err(self.unsupported(
                    &span,
                    format!("`{name}` is an external function the prover has no model for"),
                ));
            };
            let expr = match model {
                Model::Binary(op) => Expr::Binary {
                    op,
                    lhs: arg(&args, 0, &span).map_err(|m| self.unsupported(&span, m))?,
                    rhs: arg(&args, 1, &span).map_err(|m| self.unsupported(&span, m))?,
                },
                Model::Unary(op) => Expr::Unary {
                    op,
                    value: arg(&args, 0, &span).map_err(|m| self.unsupported(&span, m))?,
                },
                Model::Intrinsic(op) => Expr::Intrinsic { op, args },
                Model::Identity => {
                    Expr::Use(arg(&args, 0, &span).map_err(|m| self.unsupported(&span, m))?)
                }
                Model::Panic => {
                    self.terminate(Terminator::Fail(PanicKind::Explicit), span.clone());
                    self.current = self.function.new_block();
                    Expr::Const(Const::Unit)
                }
            };
            self.define(dst, expr, span);
            return Ok(());
        }
        let callee = self.lx.function(instance)?;
        let mut callee_shape = Shape::default();
        if let Ok(artifacts) = normalize_semantic_body(db, instance) {
            callee_shape = shape(db, instance, &artifacts.body);
            let hidden = hidden(&artifacts.body);
            if hidden.env {
                let env = self.env.ok_or_else(|| {
                    self.unsupported(&span, format!("`{name}` needs the EVM context"))
                })?;
                args.push(env);
            }
            if hidden.state.is_some() {
                args.push(self.current_state(&span)?);
            }
            for requirement in hidden.effects {
                let passed = effect_args
                    .iter()
                    .find(|arg| arg.binding_idx == requirement)
                    .ok_or_else(|| {
                        self.unsupported(&span, format!("a call to `{name}` without its effects"))
                    })?;
                let value = match &passed.arg {
                    NEffectArgValue::Value(operand) => self.operand(operand)?,
                    NEffectArgValue::Place(place) => {
                        let ty = self.ty(place.ty, &span)?;
                        let expr = self.read(place, &span)?;
                        self.fresh(ty, expr, span.clone())
                    }
                };
                args.push(value);
            }
        }
        if callee_shape.is_plain() {
            self.define(dst, Expr::Call { func: callee, args }, span);
            return Ok(());
        }
        // The callee returns its result, then what it leaves in each place
        // it was lent, which goes back where it came from.
        let returned = self.lx.program.function(callee).result.clone();
        let Type::Tuple(parts) = &returned else {
            return Err(self.unsupported(&span, format!("`{name}` returns what it borrowed oddly")));
        };
        let parts = parts.clone();
        let tuple = self.fresh(returned, Expr::Call { func: callee, args }, span.clone());
        self.define(
            dst,
            Expr::Project {
                value: tuple,
                path: vec![Proj::Field(0)],
            },
            span.clone(),
        );
        for (position, index) in callee_shape.mutable.iter().enumerate() {
            let lent = operands
                .get(*index)
                .and_then(|operand| self.borrows.get(&operand.value).cloned())
                .ok_or_else(|| {
                    self.unsupported(
                        &span,
                        format!(
                            "a mutable borrow passed to `{name}` from where it cannot be traced"
                        ),
                    )
                })?;
            let back = self.fresh(
                parts[position + 1].clone(),
                Expr::Project {
                    value: tuple,
                    path: vec![Proj::Field(position as u16 + 1)],
                },
                span.clone(),
            );
            self.push(
                StmtKind::Store {
                    slot: lent.0,
                    path: lent.1,
                    value: back,
                },
                span.clone(),
            );
        }
        if callee_shape.writes_state {
            let last = parts.len() - 1;
            let state = self.fresh(
                parts[last].clone(),
                Expr::Project {
                    value: tuple,
                    path: vec![Proj::Field(last as u16)],
                },
                span.clone(),
            );
            self.store_state(state, &span)?;
        }
        Ok(())
    }

    fn succ(&mut self, succ: &NSuccessor) -> Lowering<Succ> {
        Ok(Succ {
            block: self.blocks[&succ.block],
            args: succ
                .args
                .iter()
                .map(|arg| self.operand(arg))
                .collect::<Lowering<_>>()?,
        })
    }

    fn terminator(&mut self, term: &NTerminatorKind<'db>, span: &Span) -> Lowering<Terminator> {
        Ok(match term {
            NTerminatorKind::Goto(succ) => Terminator::Goto(self.succ(succ)?),
            NTerminatorKind::Branch {
                cond,
                then_target,
                else_target,
            } => Terminator::Branch {
                cond: self.operand(cond)?,
                then: self.succ(then_target)?,
                otherwise: self.succ(else_target)?,
            },
            NTerminatorKind::MatchEnum {
                value,
                cases,
                default,
                ..
            } => Terminator::Switch {
                value: self.operand(value)?,
                cases: cases
                    .iter()
                    .map(|(variant, succ)| Ok((variant.0, self.succ(succ)?)))
                    .collect::<Lowering<_>>()?,
                default: default.as_ref().map(|succ| self.succ(succ)).transpose()?,
            },
            NTerminatorKind::Fail(FailReason::Assert { message }) => {
                Terminator::Fail(match message {
                    Some(text) => PanicKind::Message(text.data(self.lx.db).to_string()),
                    None => PanicKind::Assertion,
                })
            }
            NTerminatorKind::Unreachable => Terminator::Unreachable,
            NTerminatorKind::Return(_) if matches!(self.role, Role::Handler { .. }) => {
                Terminator::Return(self.current_state(span)?)
            }
            NTerminatorKind::Return(value) if !self.shape.is_plain() => {
                let value = match value {
                    Some(value) => self.operand(value)?,
                    None => self.fresh(Type::Unit, Expr::Const(Const::Unit), span.clone()),
                };
                Terminator::Return(self.with_lent(value, span)?)
            }
            NTerminatorKind::Yield { value, .. } if !self.shape.is_plain() => {
                let value = self.operand(value)?;
                Terminator::Return(self.with_lent(value, span)?)
            }
            NTerminatorKind::Return(Some(value)) | NTerminatorKind::Yield { value, .. } => {
                Terminator::Return(self.operand(value)?)
            }
            NTerminatorKind::Return(None) => {
                let unit = self.fresh(Type::Unit, Expr::Const(Const::Unit), span.clone());
                Terminator::Return(unit)
            }
        })
    }
}

fn arg(args: &[ValueId], index: usize, _span: &Span) -> Result<ValueId, String> {
    args.get(index)
        .copied()
        .ok_or_else(|| format!("a call with too few arguments (expected argument {index})"))
}

fn arith(overflow: Overflow) -> Arith {
    match overflow {
        Overflow::Checked => Arith::Checked,
        Overflow::Wrapping => Arith::Wrapping,
    }
}

fn binary(op: BinaryOp) -> BinOp {
    match op {
        BinaryOp::Add(overflow) => BinOp::Add(arith(overflow)),
        BinaryOp::Sub(overflow) => BinOp::Sub(arith(overflow)),
        BinaryOp::Mul(overflow) => BinOp::Mul(arith(overflow)),
        BinaryOp::Div(overflow) => BinOp::Div(arith(overflow)),
        BinaryOp::Rem => BinOp::Rem(Arith::Checked),
        BinaryOp::Pow(overflow) => BinOp::Pow(arith(overflow)),
        BinaryOp::Shl => BinOp::Shl,
        BinaryOp::Shr => BinOp::Shr,
        BinaryOp::BitAnd => BinOp::BitAnd,
        BinaryOp::BitOr => BinOp::BitOr,
        BinaryOp::BitXor => BinOp::BitXor,
        BinaryOp::Comp(op) => match op {
            CompBinOp::Eq => BinOp::Eq,
            CompBinOp::NotEq => BinOp::Ne,
            CompBinOp::Lt => BinOp::Lt,
            CompBinOp::LtEq => BinOp::Le,
            CompBinOp::Gt => BinOp::Gt,
            CompBinOp::GtEq => BinOp::Ge,
        },
    }
}
