use femp_core::outcome::TestRun;
use femp_core::platform::Context;

use crate::plan::Job;

pub fn run(
    cx: &Context<'_>,
    jobs: &[Job<'_>],
    filter: Option<&str>,
    on_finished: &mut dyn FnMut(&TestRun),
) -> Vec<TestRun> {
    jobs.iter()
        .map(|job| {
            let run = job
                .platform
                .test(cx, job.module, filter)
                .unwrap_or_else(|error| TestRun {
                    module: job.module.name.clone(),
                    platform: job.platform.id().clone(),
                    cases: Vec::new(),
                    suite_error: Some(error.to_string()),
                    log: String::new(),
                });
            on_finished(&run);
            run
        })
        .collect()
}
