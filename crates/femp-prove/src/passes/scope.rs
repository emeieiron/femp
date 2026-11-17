//! Keeps the functions the task's entry reaches, the specs that may stand
//! in for any of them and the invariants a reached call back needs, and
//! drops the rest.

use std::collections::BTreeMap;

use femp_vir::{Expr, FuncId, StmtKind};

use super::{Pass, callees};
use crate::task::{NotSupported, Task};

pub struct Scope;

impl Pass for Scope {
    fn name(&self) -> &'static str {
        "scope"
    }

    fn run(&self, task: &mut Task) -> Result<(), NotSupported> {
        let mut order = vec![task.entry];
        let mut next = 0;
        loop {
            while next < order.len() {
                for callee in callees(&task.program, order[next]) {
                    if !order.contains(&callee) {
                        order.push(callee);
                    }
                }
                next += 1;
            }
            let mut wanted: Vec<FuncId> = task
                .summaries
                .iter()
                .filter(|summary| order.contains(&summary.target) && !order.contains(&summary.spec))
                .map(|summary| summary.spec)
                .collect();
            if let Some(reentry) = &task.reentry
                && order.contains(&reentry.func)
            {
                let invariants = reentry.checked.iter().map(|(func, _, _)| *func);
                for func in invariants.chain(reentry.assumed.iter().map(|(func, _)| *func)) {
                    if !order.contains(&func) && !wanted.contains(&func) {
                        wanted.push(func);
                    }
                }
            }
            if wanted.is_empty() {
                break;
            }
            order.extend(wanted);
        }
        task.summaries
            .retain(|summary| order.contains(&summary.target));
        let renumbered: BTreeMap<FuncId, FuncId> = order
            .iter()
            .enumerate()
            .map(|(index, old)| (*old, FuncId(index as u32)))
            .collect();
        let mut kept = Vec::new();
        for old in &order {
            let mut function = task.program.function(*old).clone();
            for block in &mut function.blocks {
                for stmt in &mut block.stmts {
                    if let StmtKind::Def {
                        expr: Expr::Call { func, .. },
                        ..
                    } = &mut stmt.kind
                    {
                        *func = renumbered[func];
                    }
                }
            }
            kept.push(function);
        }
        task.program.functions = kept;
        task.entry = FuncId(0);
        for summary in task.summaries.iter_mut().chain(task.proving.as_mut()) {
            summary.spec = renumbered[&summary.spec];
            summary.target = renumbered[&summary.target];
        }
        // A task whose code never lets a callee call back needs no reentry.
        task.reentry = task.reentry.take().and_then(|mut reentry| {
            reentry.func = *renumbered.get(&reentry.func)?;
            for (func, _, _) in &mut reentry.checked {
                *func = renumbered[func];
            }
            for (func, _) in &mut reentry.assumed {
                *func = renumbered[func];
            }
            Some(reentry)
        });
        Ok(())
    }
}
