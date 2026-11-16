//! Verification conditions for an acyclic task, by block reachability.
//! Every value is a constant equal to its definition. Each block has a
//! reachability literal, the disjunction of its incoming edges; inside a
//! block, an assumption narrows the literal and an assertion is checked
//! under it, then assumed for what follows. A block parameter equals the
//! argument of whichever edge was taken.

pub mod ints;
mod sorts;
mod terms;

use std::fmt::Write;

use femp_vir::{BlockId, Check, Function, Span, StmtKind, Terminator, Type, ValueId};

use crate::task::{NotSupported, Task};
use sorts::Sorts;
use terms::{Definition, Terms, name};

/// The encoded task: declarations and facts, then one query per assertion.
#[derive(Debug, Clone)]
pub struct Script {
    pub prelude: String,
    pub obligations: Vec<Obligation>,
}

/// An assertion as a satisfiability query: a model of `violation` is an
/// execution in which the assertion fails.
#[derive(Debug, Clone)]
pub struct Obligation {
    pub violation: String,
    pub check: Check,
    pub span: Span,
}

impl Script {
    /// The whole script as a file `z3 -smt2` can run, for `--keep-smt`.
    pub fn text(&self) -> String {
        let mut out = self.prelude.clone();
        for obligation in &self.obligations {
            let _ = writeln!(
                out,
                "; {} at {}\n(push 1)\n(assert {})\n(check-sat)\n(pop 1)",
                obligation.check, obligation.span, obligation.violation
            );
        }
        out
    }
}

pub fn encode(task: &Task) -> Result<Script, NotSupported> {
    let function = task.entry();
    if let Some(span) = cycle(function) {
        return Err(NotSupported::new(
            span,
            "a loop the prover could not cut or unroll",
        ));
    }
    let mut sorts = Sorts::new(&task.program.types);
    for info in &function.values {
        sorts.note(&info.ty);
    }
    let mut out = String::new();
    out.push_str("(set-option :produce-models true)\n");
    out.push_str(&sorts.declarations());
    let ints: std::collections::BTreeSet<(u16, bool)> = function
        .values
        .iter()
        .flat_map(|info| int_types(&info.ty))
        .chain(task.program.types.defs.iter().flat_map(def_int_types))
        .collect();
    out.push_str(&ints::prelude(&ints));
    let applied = declare_applied(function, &sorts, &mut out);
    let terms = Terms {
        sorts: &sorts,
        function,
        applied: &applied,
    };
    let mut encoder = Encoder {
        terms,
        out,
        obligations: Vec::new(),
        literals: 0,
        incoming: vec![Vec::new(); function.blocks.len()],
    };
    encoder.declare_values()?;
    encoder.blocks()?;
    Ok(Script {
        prelude: encoder.out,
        obligations: encoder.obligations,
    })
}

/// Declares an uninterpreted function for each name `Expr::Apply` uses.
fn declare_applied(
    function: &Function,
    sorts: &Sorts<'_>,
    out: &mut String,
) -> std::collections::BTreeMap<String, String> {
    let mut applied = std::collections::BTreeMap::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            let StmtKind::Def {
                dst,
                expr: femp_vir::Expr::Apply { name, args },
            } = &stmt.kind
            else {
                continue;
            };
            if applied.contains_key(name) {
                continue;
            }
            let symbol = format!("uf{}", applied.len());
            let params: Vec<String> = args
                .iter()
                .map(|arg| sorts.sort(function.ty(*arg)))
                .collect();
            let _ = writeln!(
                out,
                "(declare-fun {symbol} ({}) {})",
                params.join(" "),
                sorts.sort(function.ty(*dst))
            );
            applied.insert(name.clone(), symbol);
        }
    }
    applied
}

fn int_types(ty: &Type) -> Vec<(u16, bool)> {
    match ty {
        Type::Int { bits, signed } => vec![(*bits, *signed)],
        Type::Tuple(items) => items.iter().flat_map(int_types).collect(),
        Type::Array { elem, .. } => int_types(elem),
        Type::Map { key, value } => int_types(key).into_iter().chain(int_types(value)).collect(),
        Type::Bool | Type::Unit | Type::Adt(_) => Vec::new(),
    }
}

fn def_int_types(def: &femp_vir::TypeDef) -> Vec<(u16, bool)> {
    match def {
        femp_vir::TypeDef::Struct { fields, .. } => fields
            .iter()
            .flat_map(|field| int_types(&field.ty))
            .collect(),
        femp_vir::TypeDef::Enum { variants, .. } => variants
            .iter()
            .flat_map(|variant| &variant.fields)
            .flat_map(int_types)
            .collect(),
    }
}

/// The span of a block that a loop returns to, if the function has one.
fn cycle(function: &Function) -> Option<Span> {
    let order = function.reverse_postorder();
    let mut position = vec![usize::MAX; function.blocks.len()];
    for (index, block) in order.iter().enumerate() {
        position[block.index()] = index;
    }
    for block in &order {
        for succ in function.block(*block).term.successors() {
            if position[succ.block.index()] <= position[block.index()] {
                let target = function.block(succ.block);
                let span = target
                    .stmts
                    .first()
                    .map(|stmt| stmt.span.clone())
                    .unwrap_or_else(|| target.span.clone());
                return Some(span);
            }
        }
    }
    None
}

struct Encoder<'a> {
    terms: Terms<'a>,
    out: String,
    obligations: Vec<Obligation>,
    literals: usize,
    /// For each block, the conditions of the edges that enter it.
    incoming: Vec<Vec<String>>,
}

impl Encoder<'_> {
    fn line(&mut self, text: impl AsRef<str>) {
        self.out.push_str(text.as_ref());
        self.out.push('\n');
    }

    fn fresh_literal(&mut self, definition: String) -> String {
        let literal = format!("r{}", self.literals);
        self.literals += 1;
        self.line(format!("(declare-const {literal} Bool)"));
        self.line(format!("(assert (= {literal} {definition}))"));
        literal
    }

    fn declare_values(&mut self) -> Result<(), NotSupported> {
        let function = self.terms.function;
        for (index, info) in function.values.iter().enumerate() {
            let value = ValueId(index as u32);
            let sort = self.terms.sorts.sort(&info.ty);
            self.line(format!("(declare-const {} {sort})", name(value)));
        }
        for block in &function.blocks {
            for stmt in &block.stmts {
                let StmtKind::Def { dst, expr } = &stmt.kind else {
                    continue;
                };
                let definition = self.terms.define(*dst, expr).map_err(|error| {
                    NotSupported::new(
                        if error.span.is_known() {
                            error.span.clone()
                        } else {
                            stmt.span.clone()
                        },
                        error.message,
                    )
                })?;
                match definition {
                    Definition::Equals(term) => {
                        self.line(format!("(assert (= {} {term}))", name(*dst)));
                    }
                    Definition::Facts(facts) => {
                        for fact in facts {
                            self.line(format!("(assert {fact})"));
                        }
                    }
                    Definition::Arbitrary => {
                        let ty = function.ty(*dst);
                        if let Some(wf) = self.terms.sorts.wf(ty, &name(*dst)) {
                            self.line(format!("(assert {wf})"));
                        }
                    }
                    Definition::Unknown(term) => {
                        self.line(format!("(assert (= {} {term}))", name(*dst)));
                        let ty = function.ty(*dst);
                        if let Some(wf) = self.terms.sorts.wf(ty, &name(*dst)) {
                            self.line(format!("(assert {wf})"));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn blocks(&mut self) -> Result<(), NotSupported> {
        let function = self.terms.function;
        let order = function.reverse_postorder();
        for block in &order {
            let literal = format!("b{}", block.0);
            self.line(format!("(declare-const {literal} Bool)"));
        }
        for block in order {
            let reach = format!("b{}", block.0);
            if block == function.entry {
                self.line(format!("(assert {reach})"));
            } else {
                let incoming = std::mem::take(&mut self.incoming[block.index()]);
                let definition = match incoming.len() {
                    0 => "false".to_string(),
                    1 => incoming[0].clone(),
                    _ => format!("(or {})", incoming.join(" ")),
                };
                self.line(format!("(assert (= {reach} {definition}))"));
            }
            let mut current = reach;
            for stmt in &function.block(block).stmts {
                match &stmt.kind {
                    StmtKind::Assume(cond) => {
                        current = self.fresh_literal(format!("(and {current} {})", name(*cond)));
                    }
                    StmtKind::Assert { cond, check } => {
                        self.obligations.push(Obligation {
                            violation: format!("(and {current} (not {}))", name(*cond)),
                            check: check.clone(),
                            span: stmt.span.clone(),
                        });
                        current = self.fresh_literal(format!("(and {current} {})", name(*cond)));
                    }
                    StmtKind::Def { .. } => {}
                    StmtKind::Store { .. }
                    | StmtKind::CheckIndex { .. }
                    | StmtKind::Spec { .. } => {
                        return Err(NotSupported::new(
                            stmt.span.clone(),
                            "internal: an unlowered statement reached the encoder",
                        ));
                    }
                }
            }
            self.edges(block, &current)?;
        }
        Ok(())
    }

    fn edges(&mut self, block: BlockId, current: &str) -> Result<(), NotSupported> {
        let function = self.terms.function;
        let term = &function.block(block).term;
        let edges: Vec<(String, &femp_vir::Succ)> = match term {
            Terminator::Goto(succ) => vec![(current.to_string(), succ)],
            Terminator::Branch {
                cond,
                then,
                otherwise,
            } => vec![
                (format!("(and {current} {})", name(*cond)), then),
                (format!("(and {current} (not {}))", name(*cond)), otherwise),
            ],
            Terminator::Switch {
                value,
                cases,
                default,
            } => {
                let ty = function.ty(*value);
                let tests: Vec<String> = cases
                    .iter()
                    .map(|(variant, _)| {
                        format!(
                            "({} {})",
                            self.terms.sorts.tester(ty, *variant),
                            name(*value)
                        )
                    })
                    .collect();
                let mut edges: Vec<(String, &femp_vir::Succ)> = cases
                    .iter()
                    .zip(&tests)
                    .map(|((_, succ), test)| (format!("(and {current} {test})"), succ))
                    .collect();
                if let Some(default) = default {
                    let none = if tests.is_empty() {
                        "true".to_string()
                    } else {
                        format!("(not (or {}))", tests.join(" "))
                    };
                    edges.push((format!("(and {current} {none})"), default));
                }
                edges
            }
            Terminator::Return(_) | Terminator::Unreachable | Terminator::Stop => Vec::new(),
            Terminator::Fail(_) => {
                return Err(NotSupported::new(
                    function.block(block).span.clone(),
                    "internal: a failure reached the encoder",
                ));
            }
        };
        for (condition, succ) in edges {
            let edge = self.fresh_literal(condition);
            let params = function.block(succ.block).params.clone();
            for (param, arg) in params.iter().zip(&succ.args) {
                self.line(format!(
                    "(assert (=> {edge} (= {} {})))",
                    name(*param),
                    name(*arg)
                ));
            }
            self.incoming[succ.block.index()].push(edge);
        }
        Ok(())
    }
}
