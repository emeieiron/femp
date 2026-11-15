use femp_core::platform::Platform;
use femp_core::project::{Module, Project};

pub struct Job<'a> {
    pub module: &'a Module,
    pub platform: &'a dyn Platform,
}

pub fn plan<'a>(project: &'a Project, platforms: &[&'a dyn Platform]) -> Vec<Job<'a>> {
    let shared = project.shared_modules().flat_map(|module| {
        platforms.iter().map(move |platform| Job {
            module,
            platform: *platform,
        })
    });
    let specific = platforms.iter().flat_map(|platform| {
        project.modules_for(platform.id()).map(move |module| Job {
            module,
            platform: *platform,
        })
    });
    shared.chain(specific).collect()
}
