pub mod sol;

use core::fmt;
use std::{
    collections::BTreeMap,
    error::Error,
    fmt::Display,
    fs,
    hash::Hash,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use clap::ValueEnum;
use foundry_compilers::Language;
use serde::Serialize;
use sol::high;
use solar_ast::ast::Span;
use solar_interface::BytePos;
use yansi::{Paint, Painted};

// TODO: maybe add a way to specify the linter "profile" (ex. Default, OP Stack, etc.)
pub trait Linter: Send + Sync + Clone {
    /// Enum of languages supported by the linter.
    type Language: Language;
    type Lint: Lint + Ord;
    type LinterError: Error;

    /// Main entrypoint for the linter.
    fn lint(&self, input: &[PathBuf]) -> Result<LinterOutput<Self>, Self::LinterError>;
}

pub struct ProjectLinter<L>
where
    L: Linter,
{
    pub linter: L,
}

impl<L> ProjectLinter<L>
where
    L: Linter,
{
    pub fn new(linter: L) -> Self {
        Self { linter }
    }

    pub fn lint(self, input: &[PathBuf]) -> eyre::Result<LinterOutput<L>> {
        Ok(self.linter.lint(&input).expect("TODO: handle error"))
    }
}

// NOTE: add some way to specify linter profiles. For example having a profile adhering to the op
// stack, base, etc. This can probably also be accomplished via the foundry.toml or some functions.
// Maybe have generic profile/settings

pub struct LinterOutput<L: Linter>(pub BTreeMap<L::Lint, Vec<SourceLocation>>);

impl<L: Linter> LinterOutput<L> {
    // Optional: You can still provide a `new` method for convenience
    pub fn new() -> Self {
        LinterOutput(BTreeMap::new())
    }
}

impl<L: Linter> Deref for LinterOutput<L> {
    type Target = BTreeMap<L::Lint, Vec<SourceLocation>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<L: Linter> DerefMut for LinterOutput<L> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<L: Linter> Extend<(L::Lint, Vec<SourceLocation>)> for LinterOutput<L> {
    fn extend<T: IntoIterator<Item = (L::Lint, Vec<SourceLocation>)>>(&mut self, iter: T) {
        for (lint, findings) in iter {
            self.0.entry(lint).or_insert_with(Vec::new).extend(findings);
        }
    }
}

impl<L: Linter> fmt::Display for LinterOutput<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Add initial spacing before output
        writeln!(f, "")?;

        for (lint, locations) in &self.0 {
            let severity = lint.severity();
            let name = lint.name();
            let description = lint.description();

            for location in locations {
                if let Some(file_contents) = location.file_contents() {
                    if let Some(((start_line, start_column), (end_line, end_column))) =
                        location.location(&file_contents)
                    {
                        dbg!(start_line, start_column, end_line, end_column);
                        let max_line_number_width = end_line.to_string().len();

                        writeln!(f, "{severity}: {name}: {description}")?;

                        writeln!(
                            f,
                            "{}  {}:{}:{}",
                            Paint::blue(" -->").bold(),
                            location.file.display(),
                            start_line,
                            start_column
                        )?;

                        writeln!(
                            f,
                            "{:width$}{}",
                            "",
                            Paint::blue("|").bold(),
                            width = max_line_number_width + 1
                        )?;

                        let lines = file_contents.lines().collect::<Vec<&str>>();
                        let display_start_line =
                            if start_line > 1 { start_line - 1 } else { start_line };
                        let display_end_line =
                            if end_line < lines.len() { end_line + 1 } else { end_line };

                        for line_number in display_start_line..=display_end_line {
                            let line = lines.get(line_number - 1).unwrap_or(&"");

                            if line_number == start_line {
                                writeln!(
                                    f,
                                    "{:>width$} {} {}",
                                    line_number,
                                    Paint::blue("|").bold(),
                                    line,
                                    width = max_line_number_width
                                )?;

                                let caret = severity
                                    .color(&"^".repeat((end_column - start_column + 1) as usize));
                                writeln!(
                                    f,
                                    "{:width$}{} {}{}",
                                    "",
                                    Paint::blue("|").bold(),
                                    " ".repeat((start_column - 1) as usize),
                                    caret,
                                    width = max_line_number_width + 1
                                )?;
                            } else {
                                writeln!(
                                    f,
                                    "{:width$}{} {}",
                                    "",
                                    Paint::blue("|").bold(),
                                    line,
                                    width = max_line_number_width + 1
                                )?;
                            }
                        }

                        writeln!(
                            f,
                            "{:width$}{}",
                            "",
                            Paint::blue("|").bold(),
                            width = max_line_number_width + 1
                        )?;

                        writeln!(f, "")?;
                    }
                }
            }
        }

        Ok(())
    }
}

pub trait Lint: Hash {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn severity(&self) -> Severity;
}

// TODO: impl color for severity
#[derive(Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Severity {
    High,
    Med,
    Low,
    Info,
    Gas,
}

impl Severity {
    pub fn color(&self, message: &str) -> String {
        match self {
            Severity::High => Paint::red(message).bold().to_string(),
            Severity::Med => Paint::yellow(message).bold().to_string(),
            Severity::Low => Paint::green(message).bold().to_string(),
            Severity::Info => Paint::blue(message).bold().to_string(),
            Severity::Gas => Paint::green(message).bold().to_string(),
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let colored = match self {
            Severity::High => self.color("High"),
            Severity::Med => self.color("Med"),
            Severity::Low => self.color("Low"),
            Severity::Info => self.color("Info"),
            Severity::Gas => self.color("Gas"),
        };
        write!(f, "{}", colored)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    pub file: PathBuf,
    pub span: Span,
}

impl SourceLocation {
    pub fn new(file: PathBuf, span: Span) -> Self {
        Self { file, span }
    }

    pub fn file_contents(&self) -> Option<String> {
        fs::read_to_string(&self.file).ok()
    }

    /// Compute the line and column for the start and end of the span.
    pub fn location(&self, file_contents: &str) -> Option<((usize, usize), (usize, usize))> {
        let lo = self.span.lo().0 as usize;
        let hi = self.span.hi().0 as usize;

        if lo > file_contents.len() || hi > file_contents.len() {
            return None;
        }

        let mut offset = 0;
        let mut start_line = None;
        let mut start_column = None;

        for (line_number, line) in file_contents.lines().enumerate() {
            let line_length = line.len() + 1;

            // If start line and column is already found, look for end line and column
            if let Some(start) = start_line {
                if offset <= hi && hi < offset + line_length {
                    let end_line = line_number + 1;
                    let end_column = hi - offset + 1;
                    return Some(((start, start_column.unwrap()), (end_line, end_column)));
                }
            } else if offset <= lo && lo < offset + line_length {
                // Determine start line and column.
                start_line = Some(line_number + 1);
                start_column = Some(lo - offset + 1);
            }

            offset += line_length;
        }

        None
    }
}
