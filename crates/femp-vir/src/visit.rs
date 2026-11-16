//! Walks the values an instruction reads, so passes can rename or collect
//! them without matching every form.

use crate::program::{Expr, Function, Index, Proj, StmtKind, Terminator, ValueId};

fn path_values<'a>(path: &'a [Proj], out: &mut Vec<&'a ValueId>) {
    for projection in path {
        if let Proj::Index(Index::Value(value)) = projection {
            out.push(value);
        }
    }
}

fn path_values_mut<'a>(path: &'a mut [Proj], out: &mut Vec<&'a mut ValueId>) {
    for projection in path {
        if let Proj::Index(Index::Value(value)) = projection {
            out.push(value);
        }
    }
}

impl Expr {
    pub fn operands(&self) -> Vec<&ValueId> {
        let mut out = Vec::new();
        match self {
            Self::Use(value)
            | Self::Unary { value, .. }
            | Self::Cast { value, .. }
            | Self::IsVariant { value, .. }
            | Self::Tag { value }
            | Self::Repeat { value, .. } => out.push(value),
            Self::Binary { lhs, rhs, .. } => {
                out.push(lhs);
                out.push(rhs);
            }
            Self::Make { fields, .. } | Self::MakeEnum { fields, .. } => out.extend(fields),
            Self::Project { value, path } => {
                out.push(value);
                path_values(path, &mut out);
            }
            Self::Update { value, path, new } => {
                out.push(value);
                path_values(path, &mut out);
                out.push(new);
            }
            Self::Load { path, .. } => path_values(path, &mut out),
            Self::Call { args, .. } | Self::Intrinsic { args, .. } => out.extend(args),
            Self::Const(_) | Self::Havoc => {}
        }
        out
    }

    pub fn operands_mut(&mut self) -> Vec<&mut ValueId> {
        let mut out = Vec::new();
        match self {
            Self::Use(value)
            | Self::Unary { value, .. }
            | Self::Cast { value, .. }
            | Self::IsVariant { value, .. }
            | Self::Tag { value }
            | Self::Repeat { value, .. } => out.push(value),
            Self::Binary { lhs, rhs, .. } => {
                out.push(lhs);
                out.push(rhs);
            }
            Self::Make { fields, .. } | Self::MakeEnum { fields, .. } => out.extend(fields),
            Self::Project { value, path } => {
                out.push(value);
                path_values_mut(path, &mut out);
            }
            Self::Update { value, path, new } => {
                out.push(value);
                path_values_mut(path, &mut out);
                out.push(new);
            }
            Self::Load { path, .. } => path_values_mut(path, &mut out),
            Self::Call { args, .. } | Self::Intrinsic { args, .. } => out.extend(args),
            Self::Const(_) | Self::Havoc => {}
        }
        out
    }
}

impl StmtKind {
    pub fn operands(&self) -> Vec<&ValueId> {
        match self {
            Self::Def { expr, .. } => expr.operands(),
            Self::Store { path, value, .. } => {
                let mut out = Vec::new();
                path_values(path, &mut out);
                out.push(value);
                out
            }
            Self::CheckIndex { index, .. } => vec![index],
            Self::Spec { cond, .. } | Self::Assume(cond) | Self::Assert { cond, .. } => vec![cond],
        }
    }

    pub fn operands_mut(&mut self) -> Vec<&mut ValueId> {
        match self {
            Self::Def { expr, .. } => expr.operands_mut(),
            Self::Store { path, value, .. } => {
                let mut out = Vec::new();
                path_values_mut(path, &mut out);
                out.push(value);
                out
            }
            Self::CheckIndex { index, .. } => vec![index],
            Self::Spec { cond, .. } | Self::Assume(cond) | Self::Assert { cond, .. } => vec![cond],
        }
    }

    pub fn defined(&self) -> Option<ValueId> {
        match self {
            Self::Def { dst, .. } => Some(*dst),
            _ => None,
        }
    }
}

impl Terminator {
    pub fn operands(&self) -> Vec<&ValueId> {
        let mut out = Vec::new();
        match self {
            Self::Branch { cond, .. } => out.push(cond),
            Self::Switch { value, .. } | Self::Return(value) => out.push(value),
            Self::Goto(_) | Self::Fail(_) | Self::Unreachable | Self::Stop => {}
        }
        for succ in self.successors() {
            out.extend(&succ.args);
        }
        out
    }

    pub fn operands_mut(&mut self) -> Vec<&mut ValueId> {
        let mut out: Vec<&mut ValueId> = Vec::new();
        let (own, succs): (Option<&mut ValueId>, Vec<&mut crate::program::Succ>) = match self {
            Self::Branch {
                cond,
                then,
                otherwise,
            } => (Some(cond), vec![then, otherwise]),
            Self::Switch {
                value,
                cases,
                default,
            } => (
                Some(value),
                cases
                    .iter_mut()
                    .map(|(_, succ)| succ)
                    .chain(default.iter_mut())
                    .collect(),
            ),
            Self::Return(value) => (Some(value), Vec::new()),
            Self::Goto(succ) => (None, vec![succ]),
            Self::Fail(_) | Self::Unreachable | Self::Stop => (None, Vec::new()),
        };
        out.extend(own);
        for succ in succs {
            out.extend(succ.args.iter_mut());
        }
        out
    }
}

impl Function {
    /// Renames every value the function reads.
    pub fn rename_uses(&mut self, rename: &impl Fn(ValueId) -> ValueId) {
        for block in &mut self.blocks {
            for stmt in &mut block.stmts {
                for operand in stmt.kind.operands_mut() {
                    *operand = rename(*operand);
                }
            }
            for operand in block.term.operands_mut() {
                *operand = rename(*operand);
            }
        }
    }

    /// How many times each value is read.
    pub fn use_counts(&self) -> Vec<usize> {
        let mut counts = vec![0; self.values.len()];
        for block in &self.blocks {
            for stmt in &block.stmts {
                for operand in stmt.kind.operands() {
                    counts[operand.index()] += 1;
                }
            }
            for operand in block.term.operands() {
                counts[operand.index()] += 1;
            }
        }
        counts
    }
}
