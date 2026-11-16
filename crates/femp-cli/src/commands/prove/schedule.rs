//! Proves every pending task, several at a time. Each task runs its own
//! solver process, so tasks share nothing but the work queue.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use femp_prove::{Config, Verdict};

use super::plan::{ClaimPlan, Pending};

pub fn run(plans: &mut [ClaimPlan], config: &Config, jobs: usize, on_done: &(dyn Fn(&str) + Sync)) {
    let mut work = Vec::new();
    for (claim, plan) in plans.iter_mut().enumerate() {
        for (slot, pending) in plan.results.iter_mut().enumerate() {
            if let Pending::Job(job) = pending {
                let label = format!(
                    "{}::{} [{}]",
                    plan.module,
                    plan.name,
                    job.targets.join(", ")
                );
                work.push((claim, slot, label, job.task.clone(), job.dumps.clone()));
            }
        }
    }
    let next = AtomicUsize::new(0);
    let verdicts: Vec<Mutex<Option<Verdict>>> = work.iter().map(|_| Mutex::new(None)).collect();
    thread::scope(|scope| {
        for _ in 0..jobs.max(1).min(work.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some((_, _, label, task, dumps)) = work.get(index) else {
                        break;
                    };
                    let verdict = femp_prove::prove(task.clone(), config, dumps);
                    on_done(label);
                    if let Ok(mut slot) = verdicts[index].lock() {
                        *slot = Some(verdict);
                    }
                }
            });
        }
    });
    for ((claim, slot, ..), verdict) in work.into_iter().zip(verdicts) {
        let verdict = verdict
            .into_inner()
            .ok()
            .flatten()
            .unwrap_or_else(|| Verdict::Unknown("the task did not finish".to_string()));
        let pending = &mut plans[claim].results[slot];
        if let Pending::Job(job) = pending {
            let targets = job.targets.clone();
            *pending = Pending::Done { targets, verdict };
        }
    }
}
