pub mod sol;

use core::fmt;
use std::{
    collections::BTreeMap,
    error::Error,
    fmt::Display,
    hash::Hash,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use clap::ValueEnum;
use foundry_compilers::Language;
use serde::Serialize;
use solar_ast::ast::Span;
use yansi::Paint;

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
        for (lint, locations) in &self.0 {
            // Get lint details
            let severity = lint.severity();
            let name = lint.name();
            let description = lint.description();

            // Write the main message
            writeln!(f, "{severity}: {name}: {description}")?;

            // Write the source locations
            for location in locations {
                // writeln!(
                //     f,
                //     " --> {}:{}:{}",
                //     location.file.display(),
                //     location.line(),
                //     location.column()
                // )?;
                // writeln!(f, "  |")?;
                // writeln!(f, "{} | {}", location.line(), "^".repeat(location.column() as usize))?;
                // writeln!(f, "  |")?;
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

#[derive(Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Json,
    Markdown,
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

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let colored = match self {
            Severity::High => Paint::red("High").bold(),
            Severity::Med => Paint::yellow("Med").bold(),
            Severity::Low => Paint::green("Low").bold(),
            Severity::Info => Paint::blue("Info").bold(),
            Severity::Gas => Paint::green("Gas").bold(),
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
}

// TODO:  Update to implement Display for LinterOutput, model after compiler error display
// impl fmt::Display for Error {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         let mut short_msg = self.message.trim();
//         let fmtd_msg = self.formatted_message.as_deref().unwrap_or("");

//         if short_msg.is_empty() {
//             // if the message is empty, try to extract the first line from the formatted message
//             if let Some(first_line) = fmtd_msg.lines().next() {
//                 // this is something like `ParserError: <short_message>`
//                 if let Some((_, s)) = first_line.split_once(':') {
//                     short_msg = s.trim_start();
//                 } else {
//                     short_msg = first_line;
//                 }
//             }
//         }

//         // Error (XXXX): Error Message
//         styled(f, self.severity.color().bold(), |f| self.fmt_severity(f))?;
//         fmt_msg(f, short_msg)?;

//         let mut lines = fmtd_msg.lines();

//         // skip the first line if it contains the same message as the one we just formatted,
//         // unless it also contains a source location, in which case the entire error message is
// an         // old style error message, like:
//         //     path/to/file:line:column: ErrorType: message
//         if lines
//             .clone()
//             .next()
//             .is_some_and(|l| l.contains(short_msg) && l.bytes().filter(|b| *b == b':').count() <
// 3) { let _ = lines.next(); }

//         // format the main source location
//         fmt_source_location(f, &mut lines)?;

//         // format remaining lines as secondary locations
//         while let Some(line) = lines.next() {
//             f.write_str("\n")?;

//             if let Some((note, msg)) = line.split_once(':') {
//                 styled(f, Self::secondary_style(), |f| f.write_str(note))?;
//                 fmt_msg(f, msg)?;
//             } else {
//                 f.write_str(line)?;
//             }

//             fmt_source_location(f, &mut lines)?;
//         }

//         Ok(())
//     }
// }
