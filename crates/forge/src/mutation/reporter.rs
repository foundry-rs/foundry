use crate::mutation::{MutationsSummary, mutant::Mutant};
use comfy_table::{Cell, Color, Row, Table, modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL};
use yansi::Paint;

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

    pub fn report(&mut self, summary: &MutationsSummary) {
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

        let _ = sh_println!("\n{}", "═".repeat(60));
        let _ = sh_println!("{}", Paint::bold("MUTATION TESTING RESULTS"));
        let _ = sh_println!("{}", "═".repeat(60));

        let _ = sh_println!("\n{}\n", self.table);

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
            "Mutation Score: {} ({}/{} mutants killed)",
            score_colored,
            summary.total_dead(),
            summary.total_dead() + summary.total_survived()
        );

        // Survived mutants section - the most important for developers
        if !summary.get_survived().is_empty() {
            let _ = sh_println!("\n{}", "─".repeat(60));
            let _ = sh_println!("{}", Paint::red("⚠ SURVIVED MUTANTS (test suite gaps)").bold());
            let _ = sh_println!("{}", "─".repeat(60));
            let _ = sh_println!(
                "{}",
                Paint::dim(
                    "These mutations were NOT caught by your tests.\n\
                     Each represents a potential bug that your tests would miss.\n"
                )
            );

            for (i, mutant) in summary.get_survived().iter().enumerate() {
                self.print_survived_mutant(i + 1, mutant);
            }

            // Security implications
            let _ = sh_println!("\n{}", Paint::yellow("Security Implications:").bold());
            let _ = sh_println!(
                "  • Surviving mutations indicate untested code paths\n\
                 • Attackers could exploit logic bugs in these areas\n\
                 • Consider adding targeted tests for each surviving mutation"
            );

            // Suggestions
            let _ =
                sh_println!("\n{}", Paint::cyan("Suggestions to improve test coverage:").bold());
            self.print_suggestions(summary.get_survived());
        }

        // Killed mutants (collapsed by default, just count)
        if !summary.get_dead().is_empty() {
            let _ = sh_println!("\n{}", "─".repeat(60));
            let _ = sh_println!(
                "{} {} mutants killed (tests caught these mutations)",
                Paint::green("✓").bold(),
                summary.total_dead()
            );
        }

        // Invalid mutants (if any)
        if !summary.get_invalid().is_empty() {
            let _ = sh_println!("\n{}", "─".repeat(60));
            let _ = sh_println!(
                "{} {} invalid mutants (compilation failures - expected for some mutations)",
                Paint::dim("ℹ"),
                summary.total_invalid()
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

    fn print_suggestions(&self, survived: &[Mutant]) {
        // Group mutations by type and provide specific suggestions
        let mut has_arithmetic = false;
        let mut has_comparison = false;
        let mut has_increment = false;

        for mutant in survived {
            let mutation_str = mutant.mutation.to_string();
            if mutation_str.contains('+')
                || mutation_str.contains('-')
                || mutation_str.contains('*')
                || mutation_str.contains('/')
            {
                has_arithmetic = true;
            }
            if mutation_str.contains('<')
                || mutation_str.contains('>')
                || mutation_str.contains("==")
                || mutation_str.contains("!=")
            {
                has_comparison = true;
            }
            if mutation_str.contains("++") || mutation_str.contains("--") {
                has_increment = true;
            }
        }

        let _ = sh_println!("");

        if has_arithmetic {
            let _ = sh_println!(
                "  {} Test arithmetic edge cases:\n\
                 {}     - Zero values, max values (type(uint256).max)\n\
                 {}     - Overflow/underflow scenarios\n\
                 {}     - Sign changes for signed integers",
                Paint::cyan("→"),
                "",
                "",
                ""
            );
        }

        if has_comparison {
            let _ = sh_println!(
                "  {} Test boundary conditions:\n\
                 {}     - Values at exact boundaries (==, >, >=, <, <=)\n\
                 {}     - Off-by-one scenarios",
                Paint::cyan("→"),
                "",
                ""
            );
        }

        if has_increment {
            let _ = sh_println!(
                "  {} Test state transitions:\n\
                 {}     - Verify counter/index values after operations\n\
                 {}     - Check pre/post increment behavior",
                Paint::cyan("→"),
                "",
                ""
            );
        }

        if !has_arithmetic && !has_comparison && !has_increment {
            let _ = sh_println!(
                "  {} Add assertions that verify the exact behavior being mutated\n\
                 {}  {} Consider property-based testing (fuzz tests) for better coverage",
                Paint::cyan("→"),
                "",
                Paint::cyan("→"),
            );
        }
    }
}
