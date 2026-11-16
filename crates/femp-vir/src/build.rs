//! Appends instructions to a function, for the front end and for tasks that
//! write procedures of their own.

use femp_core::property::{Ty, Value};

use crate::program::{
    BinOp, BlockId, Check, Const, Expr, FuncId, Function, Span, Stmt, StmtKind, Terminator, UnOp,
    ValueId,
};
use crate::types::Type;

pub struct Cursor<'a> {
    pub function: &'a mut Function,
    pub block: BlockId,
    pub span: Span,
}

impl<'a> Cursor<'a> {
    pub fn new(function: &'a mut Function, block: BlockId) -> Self {
        let span = function.span.clone();
        Self {
            function,
            block,
            span,
        }
    }

    pub fn at(&mut self, span: Span) -> &mut Self {
        self.span = span;
        self
    }

    pub fn stmt(&mut self, kind: StmtKind) {
        let span = self.span.clone();
        self.function
            .block_mut(self.block)
            .stmts
            .push(Stmt { kind, span });
    }

    pub fn def(&mut self, ty: Type, expr: Expr) -> ValueId {
        let dst = self.function.new_value(ty, None);
        self.stmt(StmtKind::Def { dst, expr });
        dst
    }

    pub fn boolean(&mut self, value: bool) -> ValueId {
        self.def(Type::Bool, Expr::Const(Const::Bool(value)))
    }

    /// An integer constant of an integer type, which must hold it.
    pub fn int(&mut self, ty: &Type, value: Value) -> ValueId {
        self.def(ty.clone(), Expr::Const(Const::Int(value)))
    }

    pub fn unary(&mut self, op: UnOp, value: ValueId) -> ValueId {
        let ty = self.function.ty(value).clone();
        self.def(ty, Expr::Unary { op, value })
    }

    pub fn binary(&mut self, op: BinOp, lhs: ValueId, rhs: ValueId) -> ValueId {
        let ty = if op.is_comparison() || op.is_logical() {
            Type::Bool
        } else {
            self.function.ty(lhs).clone()
        };
        self.def(ty, Expr::Binary { op, lhs, rhs })
    }

    pub fn not(&mut self, value: ValueId) -> ValueId {
        self.unary(UnOp::Not, value)
    }

    pub fn all(&mut self, conditions: &[ValueId]) -> ValueId {
        let mut iter = conditions.iter().copied();
        let Some(first) = iter.next() else {
            return self.boolean(true);
        };
        iter.fold(first, |acc, next| self.binary(BinOp::And, acc, next))
    }

    pub fn any(&mut self, conditions: &[ValueId]) -> ValueId {
        let mut iter = conditions.iter().copied();
        let Some(first) = iter.next() else {
            return self.boolean(false);
        };
        iter.fold(first, |acc, next| self.binary(BinOp::Or, acc, next))
    }

    pub fn call(&mut self, func: FuncId, args: Vec<ValueId>, result: Type) -> ValueId {
        self.def(result, Expr::Call { func, args })
    }

    pub fn havoc(&mut self, ty: Type, name: Option<String>) -> ValueId {
        let dst = self.function.new_value(ty, name);
        self.stmt(StmtKind::Def {
            dst,
            expr: Expr::Havoc,
        });
        dst
    }

    pub fn assume(&mut self, cond: ValueId) {
        self.stmt(StmtKind::Assume(cond));
    }

    pub fn assert(&mut self, cond: ValueId, check: Check) {
        self.stmt(StmtKind::Assert { cond, check });
    }

    pub fn terminate(&mut self, term: Terminator) {
        let span = self.span.clone();
        let block = self.function.block_mut(self.block);
        block.term = term;
        block.span = span;
    }

    pub fn new_block(&mut self) -> BlockId {
        self.function.new_block()
    }

    pub fn switch_to(&mut self, block: BlockId) {
        self.block = block;
    }

    /// Whether `value` lies between two ordinals of `ty`, inclusive.
    pub fn within(&mut self, value: ValueId, ty: Ty, lo: Value, hi: Value) -> ValueId {
        let vir = Type::Int {
            bits: ty.bits(),
            signed: ty.is_signed(),
        };
        let low = self.int(&vir, lo);
        let high = self.int(&vir, hi);
        let above = self.binary(BinOp::Ge, value, low);
        let below = self.binary(BinOp::Le, value, high);
        self.binary(BinOp::And, above, below)
    }
}
