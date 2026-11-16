//! One normalized body as a VIR function. Values keep their SSA identity;
//! local slots stay places until the prover's slots pass; views and shared
//! borrows become the value they read, since nothing can change it while
//! they live; a projection's yield returns what it grants.

use std::collections::HashMap;

use cranelift_entity::EntityRef;

use fe_common::ingot::IngotKind;
use fe_hir::analysis::semantic::{
    BinaryOp, FailReason, NBlockId, NDataProjection, NExpr, NIndex, NOperand, NPlace, NPlaceBase,
    NRootId, NRootKind, NStatement, NStatementKind, NSuccessor, NTerminatorKind, NValueDefinition,
    NValueId, NormalizedBody, Overflow, SemanticInstance, UnaryOp, normalize_semantic_body,
};
use fe_hir::analysis::ty::ty_check::BodyOwner;
use fe_hir::analysis::ty::ty_def::BorrowKind;
use fe_hir::hir_def::{CompBinOp, Partial};
use femp_core::property::PanicKind;
use femp_vir::{
    Arith, BinOp, BlockId, Const, Expr, Function, Index, Proj, SlotId, Span, Stmt, StmtKind, Succ,
    Terminator, Type, UnOp, ValueId,
};

mod consts;

use super::Lowerer;
use crate::error::Unsupported;
use crate::intrinsics::{self, Model};
use crate::spans;
use crate::spec_ops::{SPEC_INGOT, SpecCall, spec_call};

type Lowering<T> = Result<T, Unsupported>;

pub(super) fn lower<'db>(
    lx: &mut Lowerer<'db>,
    instance: SemanticInstance<'db>,
    name: String,
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
    let result = lx
        .types
        .lower(db, instance, instance.normalized_result_ty(db))
        .map_err(|message| Unsupported::new(span.clone(), message))?;
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
}

/// Where a place's base leads: a slot, or a value standing for the place.
enum Base {
    Slot(SlotId),
    Value(ValueId),
}

impl<'db> BodyLowering<'_, 'db> {
    fn span(&self, origin: fe_hir::analysis::semantic::SemOrigin<'db>) -> Span {
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

    fn ty(&mut self, ty: fe_hir::analysis::ty::ty_def::TyId<'db>, span: &Span) -> Lowering<Type> {
        self.lx
            .types
            .lower(self.lx.db, self.instance, ty)
            .map_err(|message| self.unsupported(span, message))
    }

    fn run(&mut self) -> Lowering<()> {
        let prologue = self.function.entry;
        let order = self.body.reverse_postorder();
        self.params()?;
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
        for (expected, (param, value)) in params.into_iter().enumerate() {
            if param as usize != expected {
                return Err(self.unsupported(
                    &self.function.span.clone(),
                    "a parameter the body never reads is not supported",
                ));
            }
            let id = self.value(value)?;
            self.function.params.push(id);
        }
        Ok(())
    }

    /// Slots for the body's local storage, and the stores that give
    /// parameter places their value.
    fn slots_init(&mut self) -> Lowering<()> {
        for (index, root) in self.body.roots.iter().enumerate() {
            let id = NRootId::new(index);
            match &root.kind {
                NRootKind::LocalSlot { .. } => {
                    let span = self.span(root.origin);
                    let ty = self.ty(root.ty, &span)?;
                    let slot = self.function.new_slot(ty, None);
                    self.slots.insert(id, slot);
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
                NRootKind::Provider { .. } => {}
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
                match self.base(destination, &span)? {
                    Base::Slot(slot) => {
                        let path = self.path(destination.path.as_slice(), &span)?;
                        self.push(StmtKind::Store { slot, path, value }, span);
                        Ok(())
                    }
                    Base::Value(_) => {
                        Err(self
                            .unsupported(&span, "writing through a borrow is not supported yet"))
                    }
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
                    NRootKind::Provider { .. } => {
                        Err(self.unsupported(span, "effect providers are not supported yet"))
                    }
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

    fn path(&mut self, path: &[NDataProjection], span: &Span) -> Lowering<Vec<Proj>> {
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
                    NDataProjection::Entry(_) => {
                        return Err(self.unsupported(span, "keyed collections are not supported"));
                    }
                })
            })
            .collect()
    }

    /// The value at a place, read now.
    fn read(&mut self, place: &NPlace<'db>, span: &Span) -> Lowering<Expr> {
        let path = self.path(place.path.as_slice(), span)?;
        Ok(match self.base(place, span)? {
            Base::Slot(slot) => Expr::Load { slot, path },
            Base::Value(value) if path.is_empty() => Expr::Use(value),
            Base::Value(value) => Expr::Project { value, path },
        })
    }

    fn define_expr(&mut self, result: NValueId, expr: &NExpr<'db>, span: Span) -> Lowering<()> {
        let lowered = match expr {
            NExpr::Forward { src } => Expr::Use(self.operand(src)?),
            NExpr::ProjectValue { value, path } => Expr::Project {
                value: self.operand(value)?,
                path: self.path(path.0.as_slice(), &span)?,
            },
            NExpr::Load { place, .. } | NExpr::MakeView { place, .. } => self.read(place, &span)?,
            NExpr::Borrow { place, kind, .. } => match kind {
                BorrowKind::Ref => self.read(place, &span)?,
                BorrowKind::Mut => {
                    return Err(self.unsupported(&span, "mutable borrows are not supported yet"));
                }
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
                if !effect_args.is_empty() {
                    return Err(self.unsupported(&span, "calls with effects are not supported yet"));
                }
                let args = args
                    .iter()
                    .map(|arg| self.operand(arg))
                    .collect::<Lowering<Vec<_>>>()?;
                let instance = SemanticInstance::new(self.lx.db, callee.key);
                return self.call(result, instance, args, span);
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
        args: Vec<ValueId>,
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
        self.define(dst, Expr::Call { func: callee, args }, span);
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
