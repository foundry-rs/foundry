use crate::mutation::MutationsSummary;
use comfy_table::{Attribute, Cell, Color, Row, Table, modifiers::UTF8_ROUND_CORNERS};
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
            .add_cell(Cell::new(summary.total_survived().to_string()))
            .add_cell(Cell::new(format!(
                "{:.2}%",
                summary.total_survived() as f64 / summary.total_mutants() as f64 * 100.
            )));
        self.table.add_row(row);

        row = Row::new();
        row.add_cell(Cell::new("Dead").fg(Color::Green))
            .add_cell(Cell::new(summary.total_dead().to_string()))
            .add_cell(Cell::new(format!(
                "{:.2}%",
                summary.total_dead() as f64 / summary.total_mutants() as f64 * 100.
            )));
        self.table.add_row(row);

        row = Row::new();
        row.add_cell(Cell::new("Invalid").fg(Color::Green))
            .add_cell(Cell::new(summary.total_invalid().to_string()))
            .add_cell(Cell::new(format!(
                "{:.2}%",
                summary.total_invalid() as f64 / summary.total_mutants() as f64 * 100.
            )));
        self.table.add_row(row);

        sh_println!("Total number of mutants generated: {}", summary.total_mutants());
        sh_println!("\n{}\n", self.table);
        sh_println!("Dead mutants: {}\n", summary.dead());
        sh_println!("Survived mutants: {}\n", summary.survived());
    }
}
