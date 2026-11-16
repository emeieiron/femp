//! Keeps the functions the task's entry reaches and drops the rest.

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
        while next < order.len() {
            for callee in callees(&task.program, order[next]) {
                if !order.contains(&callee) {
                    order.push(callee);
                }
            }
            next += 1;
        }
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
        Ok(())
    }
}
