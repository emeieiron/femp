//! Proves every pending job, several at a time. Each task runs its own
//! solver process, so jobs share nothing but the work queue.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use femp_prove::{Config, Dumps, Task, Verdict};

use super::plan::{ClaimPlan, Done, Pending};

struct Work {
    claim: usize,
    slot: usize,
    label: String,
    tasks: Vec<Task>,
    dumps: Vec<Dumps>,
}

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
                work.push(Work {
                    claim,
                    slot,
                    label,
                    tasks: job.tasks.clone(),
                    dumps: job.dumps.clone(),
                });
            }
        }
    }
    let next = AtomicUsize::new(0);
    let verdicts: Vec<Mutex<Option<(usize, Verdict)>>> =
        work.iter().map(|_| Mutex::new(None)).collect();
    thread::scope(|scope| {
        for _ in 0..jobs.max(1).min(work.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(item) = work.get(index) else {
                        break;
                    };
                    let decided = femp_prove::prove_first(&item.tasks, config, &item.dumps);
                    on_done(&item.label);
                    if let Ok(mut slot) = verdicts[index].lock() {
                        *slot = Some(decided);
                    }
                }
            });
        }
    });
    for (item, verdict) in work.into_iter().zip(verdicts) {
        let (index, verdict) = verdict
            .into_inner()
            .ok()
            .flatten()
            .unwrap_or_else(|| (0, Verdict::Unknown("the task did not finish".to_string())));
        let pending = &mut plans[item.claim].results[item.slot];
        if let Pending::Job(job) = pending {
            *pending = Pending::Done(Done {
                role: job.role,
                targets: job.targets.clone(),
                index,
                verdict,
            });
        }
    }
}
