//! Rejects recursion: inlining a recursive call would not terminate, and
//! the prover has no other way to reason about it yet.

use femp_vir::FuncId;

use super::{Pass, callees};
use crate::task::{NotSupported, Task};

pub struct Recursion;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Unvisited,
    Active,
    Done,
}

impl Pass for Recursion {
    fn name(&self) -> &'static str {
        "recursion"
    }

    fn run(&self, task: &mut Task) -> Result<(), NotSupported> {
        let mut marks = vec![Mark::Unvisited; task.program.functions.len()];
        let mut stack: Vec<(FuncId, Vec<FuncId>)> =
            vec![(task.entry, callees(&task.program, task.entry))];
        marks[task.entry.index()] = Mark::Active;
        while let Some((func, pending)) = stack.last_mut() {
            let Some(next) = pending.pop() else {
                marks[func.index()] = Mark::Done;
                stack.pop();
                continue;
            };
            match marks[next.index()] {
                Mark::Active => {
                    let function = task.program.function(next);
                    return Err(NotSupported::new(
                        function.span.clone(),
                        format!(
                            "`{}` is recursive, which the prover does not support",
                            function.name
                        ),
                    ));
                }
                Mark::Done => {}
                Mark::Unvisited => {
                    marks[next.index()] = Mark::Active;
                    let calls = callees(&task.program, next);
                    stack.push((next, calls));
                }
            }
        }
        Ok(())
    }
}
