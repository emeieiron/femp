//! Lets specs stand in for their targets. A call to a function that has a
//! spec, proved or trusted, becomes a call to the spec's summary: a copy
//! that asserts what the spec requires of the caller's arguments, takes the
//! result as an unknown function of them, and assumes what the spec
//! ensures of it. The target's body
//! is not needed, so it need not be inlined or even supported.

use femp_vir::{Check, Expr, FuncId, SpecOp, StmtKind};

use super::Pass;
use super::spec_shape::{passes_parameters, shape};
use crate::task::{NotSupported, Task};

pub struct Summaries;

impl Pass for Summaries {
    fn name(&self) -> &'static str {
        "summaries"
    }

    fn run(&self, task: &mut Task) -> Result<(), NotSupported> {
        let mut done: Vec<FuncId> = Vec::new();
        for summary in task.summaries.clone() {
            if done.contains(&summary.target) {
                continue;
            }
            let spec = task.program.function(summary.spec);
            // A malformed spec is reported on its own claim; elsewhere the
            // target is inlined instead, which is always sound.
            let Ok(found) = shape(spec, summary.target, &summary.name) else {
                continue;
            };
            if !passes_parameters(spec, &found) {
                continue;
            }
            let mut copy = spec.clone();
            copy.name = format!("summary of {}", summary.name);
            let target_name = task.program.function(summary.target).name.clone();
            // The spec's proof showed that, given its requirements, its
            // statements cannot panic; trusted specs are taken at their word.
            for block in &mut copy.blocks {
                block.mode = block.mode.max(femp_vir::Mode::Trusted);
            }
            if let StmtKind::Def { expr, .. } =
                &mut copy.block_mut(found.block).stmts[found.index].kind
                && let Expr::Call { args, .. } = expr
            {
                // The code is pure, so the target's result is a function
                // of its arguments, even where the spec does not pin it.
                *expr = Expr::Apply {
                    name: target_name,
                    args: std::mem::take(args),
                };
            }
            for block in &mut copy.blocks {
                for stmt in &mut block.stmts {
                    if let StmtKind::Spec { op, cond } = stmt.kind {
                        stmt.kind = match op {
                            SpecOp::Requires => StmtKind::Assert {
                                cond,
                                check: Check::Precondition(summary.name.clone()),
                            },
                            SpecOp::Ensures | SpecOp::Invariant => StmtKind::Assume(cond),
                        };
                    }
                }
            }
            let replacement = task.program.push(copy);
            let mut used = false;
            let proving = task.proving.clone();
            for (index, function) in task.program.functions.iter_mut().enumerate() {
                let id = FuncId(index as u32);
                if id == replacement {
                    continue;
                }
                let own_target = proving
                    .as_ref()
                    .is_some_and(|p| p.spec == id && p.target == summary.target);
                if own_target {
                    continue;
                }
                for block in &mut function.blocks {
                    for stmt in &mut block.stmts {
                        if let StmtKind::Def {
                            expr: Expr::Call { func, .. },
                            ..
                        } = &mut stmt.kind
                            && *func == summary.target
                        {
                            *func = replacement;
                            used = true;
                        }
                    }
                }
            }
            if used {
                done.push(summary.target);
                let target = task.program.function(summary.target).name.clone();
                if summary.trusted {
                    let note = format!("the trusted spec `{}` stands for `{target}`", summary.name);
                    if !task.assumed.contains(&note) {
                        task.assumed.push(note);
                    }
                } else if !task.relies_on.contains(&summary.name) {
                    task.relies_on.push(summary.name.clone());
                }
            }
        }
        Ok(())
    }
}
