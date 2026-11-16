//! The transformations that turn a task into a straight-line procedure of
//! assumptions and assertions. Each pass sees the whole task and may refuse
//! code it cannot handle; `--dump-vir` prints the task after each one.

pub mod capture;
mod checks;
mod inline;
mod loops;
mod recursion;
mod scope;
mod simplify;
mod slots;
pub(crate) mod spec_shape;
mod summaries;

use femp_vir::{Expr, FuncId, Program, StmtKind};

use crate::task::{NotSupported, Task};

pub trait Pass {
    fn name(&self) -> &'static str;

    fn run(&self, task: &mut Task) -> Result<(), NotSupported>;
}

/// How many times a loop without an invariant is unrolled by default.
pub const DEFAULT_UNROLL: usize = 8;

/// The passes every task goes through, in order.
pub fn standard() -> Vec<Box<dyn Pass>> {
    with_unrolling(DEFAULT_UNROLL)
}

/// The standard passes, unrolling loops without an invariant `unroll` times.
pub fn with_unrolling(unroll: usize) -> Vec<Box<dyn Pass>> {
    vec![
        Box::new(scope::Scope),
        Box::new(recursion::Recursion),
        Box::new(spec_shape::SpecShape),
        Box::new(summaries::Summaries),
        Box::new(inline::Inline),
        Box::new(slots::Slots),
        Box::new(checks::Checks),
        Box::new(loops::Loops { unroll }),
        Box::new(simplify::Simplify),
    ]
}

/// The passes that reduce a function to call-free code with explicit
/// panics, before it is combined with another.
pub fn reduction() -> Vec<Box<dyn Pass>> {
    let mut passes = standard();
    passes.retain(|pass| pass.name() != "simplify");
    passes
}

/// The pass that keeps only what the task's entry reaches, which makes the
/// task's program comparable across targets.
pub fn scope() -> Box<dyn Pass> {
    Box::new(scope::Scope)
}

/// Runs `passes` in order, handing the task to `after` once each is done.
pub fn run(
    task: &mut Task,
    passes: &[Box<dyn Pass>],
    after: &mut dyn FnMut(&str, &Task),
) -> Result<(), NotSupported> {
    for pass in passes {
        pass.run(task)?;
        after(pass.name(), task);
    }
    Ok(())
}

/// The functions `func` calls directly, in order of first call.
pub(crate) fn callees(program: &Program, func: FuncId) -> Vec<FuncId> {
    let mut found = Vec::new();
    for block in &program.function(func).blocks {
        for stmt in &block.stmts {
            if let StmtKind::Def {
                expr: Expr::Call { func, .. },
                ..
            } = &stmt.kind
                && !found.contains(func)
            {
                found.push(*func);
            }
        }
    }
    found
}
