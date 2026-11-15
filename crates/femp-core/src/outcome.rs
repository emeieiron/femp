use crate::platform::PlatformId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCase {
    pub name: String,
    pub status: TestStatus,
}

#[derive(Debug, Clone)]
pub struct TestRun {
    pub module: String,
    pub platform: PlatformId,
    pub cases: Vec<TestCase>,
    pub suite_error: Option<String>,
    pub log: String,
}

impl TestRun {
    pub fn succeeded(&self) -> bool {
        self.suite_error.is_none()
            && self
                .cases
                .iter()
                .all(|case| case.status == TestStatus::Passed)
    }
}
