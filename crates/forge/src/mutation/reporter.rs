use comfy_table::{Cell, Color, Row, Table, modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL};
use std::time::Duration;
use yansi::Paint;

use crate::mutation::{MutationsSummary, mutant::Mutant};

pub struct MutationReporter {
    table: Table,
}

impl Default for MutationReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl MutationReporter {
    pub fn new() -> Self {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL);
        table.apply_modifier(UTF8_ROUND_CORNERS);

        table.set_header(vec![
            Cell::new("Status"),
            Cell::new("# Mutants"),
            Cell::new("% of Total"),
        ]);

        Self { table }
    }

    pub fn report(&mut self, summary: &MutationsSummary, duration: Duration) {
        let total = summary.total_mutants();
        if total == 0 {
            let _ = sh_println!("No mutants were generated.");
            return;
        }

        // Summary table
        self.add_row("Survived", summary.total_survived(), total, Color::Red);
        self.add_row("Killed", summary.total_dead(), total, Color::Green);
        self.add_row("Invalid", summary.total_invalid(), total, Color::DarkGrey);
        self.add_row("Skipped", summary.total_skipped(), total, Color::Yellow);
        if summary.total_timed_out() > 0 {
            self.add_row("Timed out", summary.total_timed_out(), total, Color::Magenta);
        }

        let _ = sh_println!("\n{}", "═".repeat(60));
        let _ = sh_println!("{}", Paint::bold("MUTATION TESTING RESULTS"));
        let _ = sh_println!("{}", "═".repeat(60));

        let _ = sh_println!("\n{}\n", self.table);

        // Legend: short, factual definitions of each status.
        let _ = sh_println!("{}", Paint::dim("Legend:"));
        let _ = sh_println!("  {} - tests did not catch the mutation", Paint::red("Survived"));
        let _ = sh_println!("  {} - tests caught the mutation", Paint::green("Killed"));
        let _ = sh_println!("  {} - mutation produced a compilation error", Paint::dim("Invalid"));
        let _ = sh_println!(
            "  {} - redundant mutation on the same expression",
            Paint::yellow("Skipped")
        );
        let _ = sh_println!(
            "  {} - compile/test exceeded the configured timeout\n",
            Paint::magenta("Timed out")
        );

        // Format duration similar to test output
        let duration_str = if duration.as_secs() >= 60 {
            format!("{}m {:.2}s", duration.as_secs() / 60, duration.as_secs_f64() % 60.0)
        } else {
            format!("{:.2}s", duration.as_secs_f64())
        };

        if summary.has_reliable_score() {
            // Mutation score with color
            let score = summary.mutation_score();
            let score_display = format!("{score:.1}%");
            let score_colored = if score >= 80.0 {
                Paint::green(&score_display).bold()
            } else if score >= 60.0 {
                Paint::yellow(&score_display).bold()
            } else {
                Paint::red(&score_display).bold()
            };

            let _ = sh_println!(
                "Mutation Score: {} ({}/{} mutants killed); finished in {}",
                score_colored,
                summary.total_dead(),
                summary.total_evaluated(),
                duration_str
            );
        } else {
            let _ = sh_println!(
                "Mutation Score: unavailable ({} timed out, {} evaluated); finished in {}",
                summary.total_timed_out(),
                summary.total_evaluated(),
                duration_str
            );
            let _ = sh_println!(
                "{}",
                Paint::yellow(
                    "Timed-out mutants dominate this run. Increase --mutation-timeout or use a \
                     faster mutation profile, for example lower optimizer_runs or disable via_ir."
                )
                .bold()
            );
        }

        // Survived mutants section - the most important for developers.
        if !summary.get_survived().is_empty() {
            let _ = sh_println!("\n{}", "─".repeat(60));
            let _ = sh_println!("{}", Paint::red("Survived mutants").bold());
            let _ = sh_println!("{}", "─".repeat(60));

            // Sort by (file, line, column, span, mutation text) so the
            // reported order is deterministic across runs / worker counts.
            // Workers complete in arbitrary order, so without this every run
            // can permute the report.
            let mut survived: Vec<&Mutant> = summary.get_survived().iter().collect();
            survived.sort_by(|a, b| {
                (
                    a.relative_path(),
                    a.line_number,
                    a.column_number,
                    a.span.lo().0,
                    a.span.hi().0,
                    a.mutation.to_string(),
                )
                    .cmp(&(
                        b.relative_path(),
                        b.line_number,
                        b.column_number,
                        b.span.lo().0,
                        b.span.hi().0,
                        b.mutation.to_string(),
                    ))
            });
            for (i, mutant) in survived.iter().enumerate() {
                self.print_survived_mutant(i + 1, mutant);
            }
        }

        // Killed mutants (collapsed: just count).
        if !summary.get_dead().is_empty() {
            let _ = sh_println!("\n{}", "─".repeat(60));
            let _ = sh_println!("{} mutants {}", summary.total_dead(), Paint::green("killed"));
        }

        // Invalid mutants (collapsed: just count).
        if !summary.get_invalid().is_empty() {
            let _ = sh_println!("\n{}", "─".repeat(60));
            let _ = sh_println!("{} mutants {}", summary.total_invalid(), Paint::dim("invalid"));
        }

        // Timed-out mutants (collapsed: just count).
        if !summary.get_timed_out().is_empty() {
            let _ = sh_println!("\n{}", "─".repeat(60));
            let _ = sh_println!(
                "{} mutants {}",
                summary.total_timed_out(),
                Paint::magenta("timed out")
            );
        }

        let _ = sh_println!("\n{}", "═".repeat(60));
    }

    fn add_row(&mut self, status: &str, count: usize, total: usize, color: Color) {
        let pct = if total > 0 { count as f64 / total as f64 * 100.0 } else { 0.0 };

        let mut row = Row::new();
        row.add_cell(Cell::new(status).fg(color))
            .add_cell(Cell::new(count.to_string()))
            .add_cell(Cell::new(format!("{pct:.1}%")));
        self.table.add_row(row);
    }

    fn print_survived_mutant(&self, index: usize, mutant: &Mutant) {
        // Show file:line
        let location = if mutant.line_number > 0 {
            format!("{}:{}", mutant.relative_path(), mutant.line_number)
        } else {
            mutant.relative_path()
        };

        let _ = sh_println!("\n  {}. {}", Paint::red(&index).bold(), Paint::bold(&location));

        // Show the source line context if available
        if !mutant.source_line.is_empty() {
            let _ = sh_println!("     {}", Paint::dim(&mutant.source_line));
        }

        // Show the diff
        let _ = sh_println!("     {}", Paint::dim("Mutation:"));
        let original = if mutant.original.is_empty() {
            "<unknown>".to_string()
        } else {
            mutant.original.clone()
        };
        let mutated = mutant.mutation.to_string();

        let _ = sh_println!("       {} {}", Paint::red("-"), Paint::red(original.trim()));
        let _ = sh_println!("       {} {}", Paint::green("+"), Paint::green(&mutated));
    }
}
