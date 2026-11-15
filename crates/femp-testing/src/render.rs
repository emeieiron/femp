use std::fmt::Write;

use crate::matrix::{Cell, ParityMatrix, Row};

const COLUMN_GAP: usize = 2;

pub fn table(matrix: &ParityMatrix) -> String {
    if matrix.rows.is_empty() {
        return String::new();
    }
    let labels: Vec<String> = matrix.rows.iter().map(label).collect();
    let label_width = labels.iter().map(String::len).max().unwrap_or_default();
    let column_width = matrix
        .platforms
        .iter()
        .map(|platform| platform.as_str().len())
        .chain(ALL_CELLS.iter().map(|cell| text(*cell).len()))
        .max()
        .unwrap_or_default();

    let mut out = String::new();
    let header: Vec<&str> = matrix.platforms.iter().map(|p| p.as_str()).collect();
    write_line(&mut out, "", label_width, &header, column_width);
    for (row, label) in matrix.rows.iter().zip(&labels) {
        let cells: Vec<&str> = row.cells.iter().map(|cell| text(*cell)).collect();
        write_line(&mut out, label, label_width, &cells, column_width);
    }
    out
}

pub fn divergence(matrix: &ParityMatrix, row: &Row) -> String {
    let on = |wanted: Cell| {
        matrix
            .platforms
            .iter()
            .zip(&row.cells)
            .filter(|(_, cell)| **cell == wanted)
            .map(|(platform, _)| platform.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{} passes on {} but fails on {}",
        label(row),
        on(Cell::Passed),
        on(Cell::Failed)
    )
}

const ALL_CELLS: [Cell; 4] = [Cell::Passed, Cell::Failed, Cell::Absent, Cell::SuiteError];

fn text(cell: Cell) -> &'static str {
    match cell {
        Cell::Passed => "pass",
        Cell::Failed => "FAIL",
        Cell::Absent => "-",
        Cell::SuiteError => "error",
    }
}

fn label(row: &Row) -> String {
    format!("{}::{}", row.module, row.test)
}

fn write_line(out: &mut String, label: &str, label_width: usize, cells: &[&str], width: usize) {
    let mut line = format!("{label:<label_width$}");
    for cell in cells {
        let _ = write!(line, "{:gap$}{cell:<width$}", "", gap = COLUMN_GAP);
    }
    out.push_str(line.trim_end());
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use femp_core::platform::PlatformId;

    use super::*;

    fn matrix() -> ParityMatrix {
        ParityMatrix {
            platforms: vec![PlatformId::new("evm"), PlatformId::new("native")],
            rows: vec![
                Row {
                    module: "core".to_string(),
                    test: "adds".to_string(),
                    cells: vec![Cell::Passed, Cell::Passed],
                },
                Row {
                    module: "core".to_string(),
                    test: "rounds_down".to_string(),
                    cells: vec![Cell::Passed, Cell::Failed],
                },
            ],
        }
    }

    #[test]
    fn aligns_columns() {
        assert_eq!(
            table(&matrix()),
            "                   evm     native\n\
             core::adds         pass    pass\n\
             core::rounds_down  pass    FAIL\n"
        );
    }

    #[test]
    fn describes_a_divergence() {
        let matrix = matrix();
        assert_eq!(
            divergence(&matrix, &matrix.rows[1]),
            "core::rounds_down passes on evm but fails on native"
        );
    }

    #[test]
    fn an_empty_matrix_renders_nothing() {
        assert_eq!(table(&ParityMatrix::default()), "");
    }
}
