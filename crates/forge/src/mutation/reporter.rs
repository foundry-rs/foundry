use crate::mutation::MutationsSummary;
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, Attribute, Cell, Color, Row, Table};
pub struct MutationReporter {
    table: Table,
}

impl MutationReporter {
    pub fn new() -> Self {
        let mut table = Table::new();

        table.apply_modifier(UTF8_ROUND_CORNERS);

        table.set_header(vec![
            Cell::new("Status"),
            Cell::new("# Mutants"),
            Cell::new("% of Total"),
        ]);

        Self { table }
    }

    pub fn report(&mut self, summary: &MutationsSummary) {
        let mut row = Row::new();
        row.add_cell(Cell::new("Survived").fg(Color::Red))
            .add_cell(Cell::new(summary.survived().to_string()))
            .add_cell(Cell::new(format!(
                "{:.2}%",
                summary.survived() as f64 / summary.total() as f64 * 100.
            )));
        self.table.add_row(row);

        row = Row::new();
        row.add_cell(Cell::new("Dead").fg(Color::Green))
            .add_cell(Cell::new(summary.dead().to_string()))
            .add_cell(Cell::new(format!(
                "{:.2}%",
                summary.dead() as f64 / summary.total() as f64 * 100.
            )));
        self.table.add_row(row);

        row = Row::new();
        row.add_cell(Cell::new("Invalid").fg(Color::Green))
            .add_cell(Cell::new(summary.invalid().to_string()))
            .add_cell(Cell::new(format!(
                "{:.2}%",
                summary.invalid() as f64 / summary.total() as f64 * 100.
            )));
        self.table.add_row(row);

        sh_println!("Total number of mutants generated: {}", summary.total());
        sh_println!("\n{}", self.table);
    }
}
