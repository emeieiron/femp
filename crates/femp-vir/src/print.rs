//! A stable text form of programs, for `--dump-vir`, snapshot tests and
//! deciding whether two targets lowered to the same program.

use std::fmt::Write;

use crate::program::{
    Arith, BinOp, Check, Const, Expr, Function, Index, Intrinsic, Program, Proj, SpecOp, Stmt,
    StmtKind, Succ, Terminator, UnOp, ValueId,
};
use crate::types::{TypeDef, TypeTable};

pub fn program(program: &Program) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "target {}", program.target);
    out.push_str(&types(&program.types));
    for (index, function) in program.functions.iter().enumerate() {
        out.push('\n');
        let _ = write!(out, "f{index} ");
        out.push_str(&self::function(&program.types, function));
    }
    out
}

/// The program without its target line, which is the same text for two
/// targets exactly when they lowered alike.
pub fn body(program: &Program) -> String {
    let text = self::program(program);
    text.split_once('\n')
        .map_or(text.clone(), |(_, rest)| rest.to_string())
}

pub fn types(types: &TypeTable) -> String {
    let mut out = String::new();
    for (index, def) in types.defs.iter().enumerate() {
        let _ = write!(out, "type t{index} {} = ", def.name());
        match def {
            TypeDef::Struct { fields, .. } => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|field| format!("{}: {}", field.name, types.display(&field.ty)))
                    .collect();
                let _ = writeln!(out, "struct {{ {} }}", fields.join(", "));
            }
            TypeDef::Enum { variants, .. } => {
                let variants: Vec<String> = variants
                    .iter()
                    .map(|variant| {
                        if variant.fields.is_empty() {
                            variant.name.clone()
                        } else {
                            let fields: Vec<String> = variant
                                .fields
                                .iter()
                                .map(|ty| types.display(ty).to_string())
                                .collect();
                            format!("{}({})", variant.name, fields.join(", "))
                        }
                    })
                    .collect();
                let _ = writeln!(out, "enum {{ {} }}", variants.join(", "));
            }
        }
    }
    out
}

pub fn function(types: &TypeTable, function: &Function) -> String {
    let printer = Printer { types, function };
    let mut out = String::new();
    let params: Vec<String> = function
        .params
        .iter()
        .map(|param| printer.typed(*param))
        .collect();
    let _ = writeln!(
        out,
        "{}({}) -> {}{}",
        function.name,
        params.join(", "),
        types.display(&function.result),
        at(&function.span)
    );
    for (index, slot) in function.slots.iter().enumerate() {
        let name = slot
            .name
            .as_deref()
            .map(|n| format!(" {n}"))
            .unwrap_or_default();
        let _ = writeln!(out, "  slot s{index}{name}: {}", types.display(&slot.ty));
    }
    for id in function.block_ids() {
        let block = function.block(id);
        let params: Vec<String> = block.params.iter().map(|p| printer.typed(*p)).collect();
        if params.is_empty() {
            let _ = writeln!(out, "{id}:");
        } else {
            let _ = writeln!(out, "{id}({}):", params.join(", "));
        }
        for stmt in &block.stmts {
            let _ = writeln!(out, "  {}{}", printer.stmt(stmt), at(&stmt.span));
        }
        let _ = writeln!(out, "  {}{}", printer.term(&block.term), at(&block.span));
    }
    out
}

fn at(span: &crate::program::Span) -> String {
    if span.is_known() {
        format!("  @{span}")
    } else {
        String::new()
    }
}

struct Printer<'a> {
    types: &'a TypeTable,
    function: &'a Function,
}

impl Printer<'_> {
    fn typed(&self, value: ValueId) -> String {
        let info = self.function.value(value);
        let name = info
            .name
            .as_deref()
            .map(|n| format!(" {n}"))
            .unwrap_or_default();
        format!("{value}{name}: {}", self.types.display(&info.ty))
    }

    fn stmt(&self, stmt: &Stmt) -> String {
        match &stmt.kind {
            StmtKind::Def { dst, expr } => format!("{} = {}", self.typed(*dst), self.expr(expr)),
            StmtKind::Store { slot, path, value } => {
                format!("store {slot}{} = {value}", self.path(path))
            }
            StmtKind::CheckIndex { index, len } => format!("check_index {index} < {len}"),
            StmtKind::Spec { op, cond } => {
                let op = match op {
                    SpecOp::Requires => "requires",
                    SpecOp::Ensures => "ensures",
                    SpecOp::Invariant => "invariant",
                };
                format!("{op} {cond}")
            }
            StmtKind::Assume(cond) => format!("assume {cond}"),
            StmtKind::Assert { cond, check } => format!("assert {cond} [{}]", check_text(check)),
        }
    }

    fn path(&self, path: &[Proj]) -> String {
        path.iter()
            .map(|projection| match projection {
                Proj::Field(field) => format!(".{field}"),
                Proj::VariantField { variant, field } => format!(".#{variant}.{field}"),
                Proj::Index(Index::Const(index)) => format!("[{index}]"),
                Proj::Index(Index::Value(value)) => format!("[{value}]"),
            })
            .collect()
    }

    fn list(values: &[ValueId]) -> String {
        values
            .iter()
            .map(ValueId::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn expr(&self, expr: &Expr) -> String {
        match expr {
            Expr::Use(value) => format!("{value}"),
            Expr::Const(constant) => match constant {
                Const::Bool(value) => format!("{value}"),
                Const::Int(value) => format!("{value}"),
                Const::Unit => "()".to_string(),
            },
            Expr::Unary { op, value } => {
                let op = match op {
                    UnOp::Neg(arith) => format!("neg{}", arith_suffix(*arith)),
                    UnOp::Not => "not".to_string(),
                    UnOp::BitNot => "bitnot".to_string(),
                };
                format!("{op} {value}")
            }
            Expr::Binary { op, lhs, rhs } => format!("{} {lhs}, {rhs}", bin_name(*op)),
            Expr::Cast { value, to } => format!("cast {value} to {}", self.types.display(to)),
            Expr::Make { ty, fields } => {
                format!("make {} {{{}}}", self.types.display(ty), Self::list(fields))
            }
            Expr::MakeEnum {
                ty,
                variant,
                fields,
            } => format!(
                "make {}#{variant} {{{}}}",
                self.types.display(ty),
                Self::list(fields)
            ),
            Expr::Project { value, path } => format!("{value}{}", self.path(path)),
            Expr::Update { value, path, new } => {
                format!("update {value}{} = {new}", self.path(path))
            }
            Expr::Load { slot, path } => format!("load {slot}{}", self.path(path)),
            Expr::IsVariant { value, variant } => format!("is {value} #{variant}"),
            Expr::Tag { value } => format!("tag {value}"),
            Expr::Repeat { value, len } => format!("repeat {value}; {len}"),
            Expr::Call { func, args } => format!("call {func}({})", Self::list(args)),
            Expr::Intrinsic { op, args } => {
                let op = match op {
                    Intrinsic::Bitcast => "bitcast",
                    Intrinsic::SaturatingAdd => "saturating_add",
                    Intrinsic::SaturatingSub => "saturating_sub",
                    Intrinsic::SaturatingMul => "saturating_mul",
                };
                format!("{op}({})", Self::list(args))
            }
            Expr::Havoc => "havoc".to_string(),
        }
    }

    fn succ(succ: &Succ) -> String {
        format!("{}({})", succ.block, Self::list(&succ.args))
    }

    fn term(&self, term: &Terminator) -> String {
        match term {
            Terminator::Goto(succ) => format!("goto {}", Self::succ(succ)),
            Terminator::Branch {
                cond,
                then,
                otherwise,
            } => format!(
                "branch {cond} -> {}, {}",
                Self::succ(then),
                Self::succ(otherwise)
            ),
            Terminator::Switch {
                value,
                cases,
                default,
            } => {
                let mut arms: Vec<String> = cases
                    .iter()
                    .map(|(variant, succ)| format!("#{variant} {}", Self::succ(succ)))
                    .collect();
                if let Some(default) = default {
                    arms.push(format!("_ {}", Self::succ(default)));
                }
                format!("switch {value} [{}]", arms.join(", "))
            }
            Terminator::Return(value) => format!("return {value}"),
            Terminator::Fail(kind) => format!("fail ({kind})"),
            Terminator::Unreachable => "unreachable".to_string(),
            Terminator::Stop => "stop".to_string(),
        }
    }
}

fn check_text(check: &Check) -> String {
    check.to_string()
}

fn arith_suffix(arith: Arith) -> &'static str {
    match arith {
        Arith::Checked => ".checked",
        Arith::Wrapping => ".wrapping",
        Arith::Exact => "",
    }
}

fn bin_name(op: BinOp) -> String {
    let (name, arith) = match op {
        BinOp::Add(arith) => ("add", Some(arith)),
        BinOp::Sub(arith) => ("sub", Some(arith)),
        BinOp::Mul(arith) => ("mul", Some(arith)),
        BinOp::Div(arith) => ("div", Some(arith)),
        BinOp::Rem(arith) => ("rem", Some(arith)),
        BinOp::Pow(arith) => ("pow", Some(arith)),
        BinOp::Shl => ("shl", None),
        BinOp::Shr => ("shr", None),
        BinOp::BitAnd => ("bitand", None),
        BinOp::BitOr => ("bitor", None),
        BinOp::BitXor => ("bitxor", None),
        BinOp::Eq => ("eq", None),
        BinOp::Ne => ("ne", None),
        BinOp::Lt => ("lt", None),
        BinOp::Le => ("le", None),
        BinOp::Gt => ("gt", None),
        BinOp::Ge => ("ge", None),
        BinOp::And => ("and", None),
        BinOp::Or => ("or", None),
        BinOp::Implies => ("implies", None),
    };
    format!("{name}{}", arith.map(arith_suffix).unwrap_or_default())
}
