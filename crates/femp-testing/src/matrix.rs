use std::collections::BTreeMap;

use femp_core::outcome::{TestRun, TestStatus};
use femp_core::platform::PlatformId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Passed,
    Failed,
    Absent,
    SuiteError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub module: String,
    pub test: String,
    pub cells: Vec<Cell>,
}

impl Row {
    pub fn is_divergent(&self) -> bool {
        self.cells.contains(&Cell::Passed) && self.cells.contains(&Cell::Failed)
    }

    pub fn is_partial(&self) -> bool {
        self.cells.contains(&Cell::Absent)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParityMatrix {
    pub platforms: Vec<PlatformId>,
    pub rows: Vec<Row>,
}

impl ParityMatrix {
    pub fn from_runs<'a>(runs: impl IntoIterator<Item = &'a TestRun>) -> Self {
        let runs: Vec<&TestRun> = runs.into_iter().collect();
        let mut platforms: Vec<PlatformId> = Vec::new();
        for run in &runs {
            if !platforms.contains(&run.platform) {
                platforms.push(run.platform.clone());
            }
        }

        let mut outcomes: BTreeMap<(&str, &str), Vec<Option<Cell>>> = BTreeMap::new();
        for run in &runs {
            let column = column_of(&platforms, &run.platform);
            for case in &run.cases {
                let cells = outcomes
                    .entry((&run.module, &case.name))
                    .or_insert_with(|| vec![None; platforms.len()]);
                cells[column] = Some(merge(cells[column], case.status));
            }
        }

        let rows = outcomes
            .into_iter()
            .map(|((module, test), cells)| Row {
                module: module.to_string(),
                test: test.to_string(),
                cells: cells
                    .into_iter()
                    .enumerate()
                    .map(|(column, cell)| {
                        cell.unwrap_or_else(|| missing(&runs, module, &platforms[column]))
                    })
                    .collect(),
            })
            .collect();
        Self { platforms, rows }
    }

    pub fn divergent(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|row| row.is_divergent())
    }

    pub fn partial(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|row| row.is_partial())
    }
}

fn column_of(platforms: &[PlatformId], platform: &PlatformId) -> usize {
    platforms
        .iter()
        .position(|candidate| candidate == platform)
        .unwrap_or_default()
}

// Tests in different modules of one ingot may share a name; they are
// reported under that one name, which fails if any of them failed.
fn merge(existing: Option<Cell>, status: TestStatus) -> Cell {
    match (existing, status) {
        (Some(Cell::Failed), _) | (_, TestStatus::Failed) => Cell::Failed,
        _ => Cell::Passed,
    }
}

fn missing(runs: &[&TestRun], module: &str, platform: &PlatformId) -> Cell {
    let suite_failed = runs
        .iter()
        .any(|run| run.module == module && &run.platform == platform && run.suite_error.is_some());
    if suite_failed {
        Cell::SuiteError
    } else {
        Cell::Absent
    }
}

#[cfg(test)]
mod tests {
    use femp_core::outcome::TestCase;

    use super::*;

    fn run(platform: &str, cases: &[(&str, TestStatus)]) -> TestRun {
        TestRun {
            module: "core".to_string(),
            platform: PlatformId::new(platform),
            cases: cases
                .iter()
                .map(|(name, status)| TestCase {
                    name: name.to_string(),
                    status: *status,
                })
                .collect(),
            suite_error: None,
            log: String::new(),
        }
    }

    fn row<'a>(matrix: &'a ParityMatrix, test: &str) -> &'a Row {
        matrix.rows.iter().find(|row| row.test == test).unwrap()
    }

    #[test]
    fn agreeing_platforms_are_not_divergent() {
        let runs = [
            run(
                "evm",
                &[("a", TestStatus::Passed), ("b", TestStatus::Failed)],
            ),
            run(
                "wasm",
                &[("a", TestStatus::Passed), ("b", TestStatus::Failed)],
            ),
        ];
        let matrix = ParityMatrix::from_runs(&runs);

        assert_eq!(
            matrix.platforms,
            [PlatformId::new("evm"), PlatformId::new("wasm")]
        );
        assert_eq!(row(&matrix, "a").cells, [Cell::Passed, Cell::Passed]);
        assert_eq!(matrix.divergent().count(), 0);
    }

    #[test]
    fn disagreeing_platforms_are_divergent() {
        let runs = [
            run("evm", &[("a", TestStatus::Passed)]),
            run("native", &[("a", TestStatus::Failed)]),
        ];
        let matrix = ParityMatrix::from_runs(&runs);

        let divergent: Vec<&str> = matrix.divergent().map(|row| row.test.as_str()).collect();
        assert_eq!(divergent, ["a"]);
    }

    #[test]
    fn tests_missing_on_a_platform_are_partial() {
        let runs = [
            run("evm", &[("a", TestStatus::Passed)]),
            run(
                "native",
                &[
                    ("a", TestStatus::Passed),
                    ("only_native", TestStatus::Passed),
                ],
            ),
        ];
        let matrix = ParityMatrix::from_runs(&runs);

        let partial = row(&matrix, "only_native");
        assert_eq!(partial.cells, [Cell::Absent, Cell::Passed]);
        assert!(partial.is_partial());
        assert!(!partial.is_divergent());
    }

    #[test]
    fn a_failed_suite_is_not_reported_as_absent() {
        let mut broken = run("wasm", &[]);
        broken.suite_error = Some("Compilation errors".to_string());
        let runs = [run("evm", &[("a", TestStatus::Passed)]), broken];
        let matrix = ParityMatrix::from_runs(&runs);

        assert_eq!(row(&matrix, "a").cells, [Cell::Passed, Cell::SuiteError]);
    }

    #[test]
    fn same_named_tests_fail_together() {
        let runs = [run(
            "evm",
            &[("a", TestStatus::Passed), ("a", TestStatus::Failed)],
        )];
        let matrix = ParityMatrix::from_runs(&runs);

        assert_eq!(row(&matrix, "a").cells, [Cell::Failed]);
    }
}
