pub mod sol;

use foundry_common::sh_println;
use foundry_compilers::{
    artifacts::{Contract, Source},
    Compiler, CompilerContract, CompilerInput, Language, Project,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sol::SolidityLinter;
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    hash::{Hash, Hasher},
    marker::PhantomData,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use clap::ValueEnum;
use solar_ast::ast::{self, SourceUnit, Span};

pub struct ProjectLinter<L>
where
    L: Linter,
{
    pub linter: L,
    /// Extra files to include, that are not necessarily in the project's source dir.
    pub files: Vec<PathBuf>,
    pub severity: Option<Vec<Severity>>,
    pub description: bool,
}

impl<L> ProjectLinter<L>
where
    L: Linter,
{
    pub fn new(linter: L) -> Self {
        Self { linter, files: Vec::new(), severity: None, description: false }
    }

    pub fn with_description(mut self, description: bool) -> Self {
        self.description = description;
        self
    }

    pub fn with_severity(mut self, severity: Option<Vec<Severity>>) -> Self {
        self.severity = severity;
        self
    }

    /// Lints the project.
    pub fn lint<C: Compiler<CompilerContract = Contract>>(
        self,
        mut project: &Project<C>,
    ) -> eyre::Result<LinterOutput<L>> {
        // TODO: infer linter from project

        // // Expand ignore globs and canonicalize paths
        // let mut ignored = expand_globs(&root, config.fmt.ignore.iter())?
        //     .iter()
        //     .flat_map(foundry_common::fs::canonicalize_path)
        //     .collect::<HashSet<_>>();

        // // Add explicitly excluded paths to the ignored set
        // if let Some(exclude_paths) = &self.exclude {
        //     ignored.extend(exclude_paths.iter().flat_map(foundry_common::fs::canonicalize_path));
        // }

        // let mut input: Vec<PathBuf> = if let Some(include_paths) = &self.include {
        //     include_paths.iter().filter(|path| path.exists()).cloned().collect()
        // } else {
        //     source_files_iter(&root, SOLC_EXTENSIONS)
        //         .filter(|p| !(ignored.contains(p) || ignored.contains(&root.join(p))))
        //         .collect()
        // };

        // input.retain(|path| !ignored.contains(path));

        // if input.is_empty() {
        //     bail!("No source files found in path");
        // }

        if !project.paths.has_input_files() && self.files.is_empty() {
            sh_println!("Nothing to lint")?;
            std::process::exit(0);
        }

        let sources = if !self.files.is_empty() {
            Source::read_all(self.files.clone())?
        } else {
            project.paths.read_input_files()?
        };

        let input = sources.into_iter().map(|(path, _)| path).collect::<Vec<PathBuf>>();

        Ok(self.linter.lint(&input).expect("TODO: handle error"))
    }
}

// NOTE: add some way to specify linter profiles. For example having a profile adhering to the op stack, base, etc.
// This can probably also be accomplished via the foundry.toml or some functions. Maybe have generic profile/settings

/// The main linter abstraction trait
pub trait Linter: Send + Sync + Clone {
    /// Enum of languages supported by the linter.
    type Language: Language;
    // TODO: Add docs. This represents linter settings. (ex. Default, OP Stack, etc.
    // type Settings: LinterSettings<Self>;
    type Lint: Lint + Ord;
    type LinterError: Error;

    /// Main entrypoint for the linter.
    fn lint(&self, input: &[PathBuf]) -> Result<LinterOutput<Self>, Self::LinterError>;
}

// TODO: probably remove
pub trait LinterSettings<L: Linter> {
    fn lints() -> Vec<L::Lint>;
}

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
//         // unless it also contains a source location, in which case the entire error message is an
//         // old style error message, like:
//         //     path/to/file:line:column: ErrorType: message
//         if lines
//             .clone()
//             .next()
//             .is_some_and(|l| l.contains(short_msg) && l.bytes().filter(|b| *b == b':').count() < 3)
//         {
//             let _ = lines.next();
//         }

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
