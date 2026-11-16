use std::fmt;

use femp_core::property::{PanicKind, Value};

use crate::types::{Type, TypeTable};

macro_rules! id {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);

        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

id!(FuncId, "f");
id!(BlockId, "bb");
id!(ValueId, "v");
id!(SlotId, "s");

/// A source position: a file relative to its ingot, and a line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    pub file: String,
    pub line: u32,
}

impl Span {
    pub fn new(file: impl Into<String>, line: u32) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }

    pub fn is_known(&self) -> bool {
        !self.file.is_empty()
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_known() {
            write!(f, "{}:{}", self.file, self.line)
        } else {
            f.write_str("?")
        }
    }
}

/// Every function a set of claims reaches, lowered for one target.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Program {
    pub target: String,
    pub types: TypeTable,
    pub functions: Vec<Function>,
}

impl Program {
    pub fn function(&self, id: FuncId) -> &Function {
        &self.functions[id.index()]
    }

    pub fn function_mut(&mut self, id: FuncId) -> &mut Function {
        &mut self.functions[id.index()]
    }

    pub fn push(&mut self, function: Function) -> FuncId {
        self.functions.push(function);
        FuncId(self.functions.len() as u32 - 1)
    }

    pub fn find(&self, name: &str) -> Option<FuncId> {
        self.functions
            .iter()
            .position(|function| function.name == name)
            .map(|index| FuncId(index as u32))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValueInfo {
    pub ty: Type,
    /// The source binding the value came from, for messages.
    pub name: Option<String>,
}

/// Storage for a local variable, which loads and stores reach until the
/// slots pass turns them into values.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SlotInfo {
    pub ty: Type,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Function {
    /// The path of the function in its ingot, such as `schedule::Schedule::vested_at`.
    pub name: String,
    pub params: Vec<ValueId>,
    pub result: Type,
    pub values: Vec<ValueInfo>,
    pub slots: Vec<SlotInfo>,
    pub blocks: Vec<Block>,
    pub entry: BlockId,
    pub span: Span,
}

impl Function {
    pub fn new(name: impl Into<String>, result: Type, span: Span) -> Self {
        let mut function = Self {
            name: name.into(),
            params: Vec::new(),
            result,
            values: Vec::new(),
            slots: Vec::new(),
            blocks: Vec::new(),
            entry: BlockId(0),
            span,
        };
        function.entry = function.new_block();
        function
    }

    pub fn value(&self, id: ValueId) -> &ValueInfo {
        &self.values[id.index()]
    }

    pub fn ty(&self, id: ValueId) -> &Type {
        &self.values[id.index()].ty
    }

    pub fn block(&self, id: BlockId) -> &Block {
        &self.blocks[id.index()]
    }

    pub fn block_mut(&mut self, id: BlockId) -> &mut Block {
        &mut self.blocks[id.index()]
    }

    pub fn new_value(&mut self, ty: Type, name: Option<String>) -> ValueId {
        self.values.push(ValueInfo { ty, name });
        ValueId(self.values.len() as u32 - 1)
    }

    pub fn new_param(&mut self, ty: Type, name: Option<String>) -> ValueId {
        let value = self.new_value(ty, name);
        self.params.push(value);
        value
    }

    pub fn new_slot(&mut self, ty: Type, name: Option<String>) -> SlotId {
        self.slots.push(SlotInfo { ty, name });
        SlotId(self.slots.len() as u32 - 1)
    }

    pub fn new_block(&mut self) -> BlockId {
        self.blocks.push(Block::default());
        BlockId(self.blocks.len() as u32 - 1)
    }

    pub fn block_ids(&self) -> impl Iterator<Item = BlockId> {
        (0..self.blocks.len() as u32).map(BlockId)
    }

    /// The blocks reachable from the entry, each before its successors
    /// unless a loop leads back to it.
    pub fn reverse_postorder(&self) -> Vec<BlockId> {
        let mut visited = vec![false; self.blocks.len()];
        let mut postorder = Vec::new();
        let mut stack = vec![(self.entry, 0usize)];
        visited[self.entry.index()] = true;
        while let Some((block, next)) = stack.last_mut() {
            let successors = self.block(*block).term.successors();
            if let Some(successor) = successors.get(*next) {
                *next += 1;
                let target = successor.block;
                if !visited[target.index()] {
                    visited[target.index()] = true;
                    stack.push((target, 0));
                }
            } else {
                postorder.push(*block);
                stack.pop();
            }
        }
        postorder.reverse();
        postorder
    }

    /// The predecessors of every block, in block order, with repeats for
    /// several edges from one block.
    pub fn predecessors(&self) -> Vec<Vec<BlockId>> {
        let mut preds = vec![Vec::new(); self.blocks.len()];
        for id in self.block_ids() {
            for successor in self.block(id).term.successors() {
                preds[successor.block.index()].push(id);
            }
        }
        preds
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Block {
    pub params: Vec<ValueId>,
    pub stmts: Vec<Stmt>,
    pub term: Terminator,
    pub span: Span,
    /// What this block's checks mean; code inlined here inherits the
    /// stronger of its own mode and this one.
    pub mode: Mode,
}

/// How a block's checks are treated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    /// Every check is an obligation.
    #[default]
    Checked,
    /// Panics are assumed not to happen, as in a proved spec's summary,
    /// whose proof already ruled them out; other checks stay obligations.
    Trusted,
    /// Every check is assumed, as when invariants are assumed of an
    /// arbitrary state.
    Assumed,
}

impl Default for Block {
    fn default() -> Self {
        Self {
            params: Vec::new(),
            stmts: Vec::new(),
            term: Terminator::Unreachable,
            span: Span::default(),
            mode: Mode::Checked,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StmtKind {
    Def {
        dst: ValueId,
        expr: Expr,
    },
    Store {
        slot: SlotId,
        path: Vec<Proj>,
        value: ValueId,
    },
    /// Fails unless `index` is below `len`, before an access at that index.
    CheckIndex {
        index: ValueId,
        len: usize,
    },
    /// A statement of the specification library, before the passes that
    /// give it meaning.
    Spec {
        op: SpecOp,
        cond: ValueId,
    },
    /// Executions where `cond` is false are not considered.
    Assume(ValueId),
    /// Executions where `cond` is false fail the check, then stop.
    Assert {
        cond: ValueId,
        check: Check,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpecOp {
    Requires,
    Ensures,
    Invariant,
}

/// What a failed assertion means.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Check {
    /// The program panics.
    Panic(PanicKind),
    /// A claim the task states, described for the report.
    Claim(String),
    /// What a spec `ensures` of its target's result.
    Ensures,
    /// What the named spec `requires` of a call to its target.
    Precondition(String),
    /// A loop may run more often than it was unrolled.
    Unwind,
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panic(kind) => write!(f, "panic: {kind}"),
            Self::Claim(text) => f.write_str(text),
            Self::Ensures => f.write_str("an `ensures` does not hold"),
            Self::Precondition(spec) => write!(f, "the precondition of `{spec}` may not hold"),
            Self::Unwind => f.write_str("loop exceeds its unrolling"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Const {
    Bool(bool),
    Int(Value),
    Unit,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Index {
    Const(usize),
    Value(ValueId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Proj {
    Field(u16),
    VariantField { variant: u16, field: u16 },
    Index(Index),
}

/// What an arithmetic operation does with a result outside its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arith {
    /// Panics; division and remainder also panic on a zero divisor.
    Checked,
    /// Keeps the low bits.
    Wrapping,
    /// Known to stay in range: the checks were made explicit.
    Exact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnOp {
    Neg(Arith),
    Not,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add(Arith),
    Sub(Arith),
    Mul(Arith),
    /// Rounds toward zero.
    Div(Arith),
    /// Has the sign of the dividend.
    Rem(Arith),
    Pow(Arith),
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Implies,
}

impl BinOp {
    pub fn arith(self) -> Option<Arith> {
        match self {
            Self::Add(arith)
            | Self::Sub(arith)
            | Self::Mul(arith)
            | Self::Div(arith)
            | Self::Rem(arith)
            | Self::Pow(arith) => Some(arith),
            _ => None,
        }
    }

    pub fn with_arith(self, arith: Arith) -> Self {
        match self {
            Self::Add(_) => Self::Add(arith),
            Self::Sub(_) => Self::Sub(arith),
            Self::Mul(_) => Self::Mul(arith),
            Self::Div(_) => Self::Div(arith),
            Self::Rem(_) => Self::Rem(arith),
            Self::Pow(_) => Self::Pow(arith),
            other => other,
        }
    }

    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            Self::Eq | Self::Ne | Self::Lt | Self::Le | Self::Gt | Self::Ge
        )
    }

    pub fn is_logical(self) -> bool {
        matches!(self, Self::And | Self::Or | Self::Implies)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intrinsic {
    /// Reinterprets the bits of an integer as another integer type,
    /// truncating or sign-extending.
    Bitcast,
    SaturatingAdd,
    SaturatingSub,
    SaturatingMul,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expr {
    Use(ValueId),
    Const(Const),
    Unary {
        op: UnOp,
        value: ValueId,
    },
    Binary {
        op: BinOp,
        lhs: ValueId,
        rhs: ValueId,
    },
    /// A lossless conversion between integer types, or a `bool` as 0 or 1.
    Cast {
        value: ValueId,
        to: Type,
    },
    /// A struct, tuple or array from its fields or elements.
    Make {
        ty: Type,
        fields: Vec<ValueId>,
    },
    MakeEnum {
        ty: Type,
        variant: u16,
        fields: Vec<ValueId>,
    },
    Project {
        value: ValueId,
        path: Vec<Proj>,
    },
    /// `value` with the part at `path` replaced by `new`.
    Update {
        value: ValueId,
        path: Vec<Proj>,
        new: ValueId,
    },
    Load {
        slot: SlotId,
        path: Vec<Proj>,
    },
    IsVariant {
        value: ValueId,
        variant: u16,
    },
    /// The variant index of an enum value, as the result type.
    Tag {
        value: ValueId,
    },
    Repeat {
        value: ValueId,
        len: usize,
    },
    /// A map whose every entry is `value`.
    Fill {
        value: ValueId,
    },
    Call {
        func: FuncId,
        args: Vec<ValueId>,
    },
    Intrinsic {
        op: Intrinsic,
        args: Vec<ValueId>,
    },
    /// Any well-formed value of the result's type.
    Havoc,
    /// The result of a function known only by name, as a spec leaves it:
    /// any well-formed value, but the same one for the same arguments.
    Apply {
        name: String,
        args: Vec<ValueId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Succ {
    pub block: BlockId,
    pub args: Vec<ValueId>,
}

impl Succ {
    pub fn to(block: BlockId) -> Self {
        Self {
            block,
            args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Terminator {
    Goto(Succ),
    Branch {
        cond: ValueId,
        then: Succ,
        otherwise: Succ,
    },
    /// Continues at the case of the enum value's variant.
    Switch {
        value: ValueId,
        cases: Vec<(u16, Succ)>,
        default: Option<Succ>,
    },
    Return(ValueId),
    /// The program panics.
    Fail(PanicKind),
    /// Control never gets here.
    Unreachable,
    /// The execution is not considered further: an assumption cut it off.
    Stop,
}

impl Terminator {
    pub fn successors(&self) -> Vec<&Succ> {
        match self {
            Self::Goto(succ) => vec![succ],
            Self::Branch {
                then, otherwise, ..
            } => vec![then, otherwise],
            Self::Switch { cases, default, .. } => cases
                .iter()
                .map(|(_, succ)| succ)
                .chain(default.iter())
                .collect(),
            Self::Return(_) | Self::Fail(_) | Self::Unreachable | Self::Stop => Vec::new(),
        }
    }

    pub fn successors_mut(&mut self) -> Vec<&mut Succ> {
        match self {
            Self::Goto(succ) => vec![succ],
            Self::Branch {
                then, otherwise, ..
            } => vec![then, otherwise],
            Self::Switch { cases, default, .. } => cases
                .iter_mut()
                .map(|(_, succ)| succ)
                .chain(default.iter_mut())
                .collect(),
            Self::Return(_) | Self::Fail(_) | Self::Unreachable | Self::Stop => Vec::new(),
        }
    }
}

/// The names and types of every value and slot a function uses, for
/// printing and for building new functions alongside existing ones.
pub fn describe_value(function: &Function, types: &TypeTable, value: ValueId) -> String {
    let info = function.value(value);
    match &info.name {
        Some(name) => format!("{value} {name}: {}", types.display(&info.ty)),
        None => format!("{value}: {}", types.display(&info.ty)),
    }
}
