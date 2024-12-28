pub mod sol;

use foundry_common::sh_println;
use foundry_compilers::{
    artifacts::{Contract, Source},
    Compiler, CompilerContract, CompilerInput, Language, Project,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    hash::{Hash, Hasher},
    marker::PhantomData,
    path::PathBuf,
};

use clap::ValueEnum;
use solar_ast::{
    ast::{self, SourceUnit, Span},
    interface::{ColorChoice, Session},
    visit::Visit,
};

#[derive(Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Json,
    Markdown,
}

#[derive(Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Severity {
    High,
    Med,
    Low,
    Info,
    Gas,
}

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
        mut self,
        project: &Project<C>,
    ) -> eyre::Result<LinterOutput<L>> {
        if !project.paths.has_input_files() && self.files.is_empty() {
            sh_println!("Nothing to compile")?;
            // nothing to do here
            std::process::exit(0);
        }

        let sources = if !self.files.is_empty() {
            Source::read_all(self.files.clone())?
        } else {
            project.paths.read_input_files()?
        };

        let input = sources.into_iter().map(|(path, _)| path).collect::<Vec<PathBuf>>();

        Ok(self.linter.lint(&input)?)
    }
}

// NOTE: add some way to specify linter profiles. For example having a profile adhering to the op stack, base, etc.
// This can probably also be accomplished via the foundry.toml or some functions. Maybe have generic profile/settings

/// The main linter abstraction trait
pub trait Linter: Send + Sync + Clone {
    // TODO: Add docs. This represents linter settings. (ex. Default, OP Stack, etc.
    // type Settings: LinterSettings<Self>;
    type Lint: Lint;
    type LinterError: Error;
    /// Enum of languages supported by the linter.
    type Language: Language;

    /// Main entrypoint for the linter.
    fn lint(&self, input: &[PathBuf]) -> Result<LinterOutput<Self>, Self::LinterError>;
}

// TODO: probably remove
pub trait LinterSettings<L: Linter> {
    fn lints() -> Vec<L::Lint>;
}

pub struct LinterOutput<L: Linter> {
    pub results: BTreeMap<L::Lint, Vec<SourceLocation>>,
}

pub trait Lint: Hash {
    fn results(&self) -> Vec<SourceLocation>;
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    pub file: String,
    pub span: Span,
}

// TODO: amend to display source location
// /// Tries to mimic Solidity's own error formatting.
// ///
// /// <https://github.com/ethereum/solidity/blob/a297a687261a1c634551b1dac0e36d4573c19afe/liblangutil/SourceReferenceFormatter.cpp#L105>
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

// impl Linter {
//     pub fn new(input: Vec<PathBuf>) -> Self {
//         Self { input, lints: Lint::all(), description: false }
//     }

//     pub fn with_severity(mut self, severity: Option<Vec<Severity>>) -> Self {
//         if let Some(severity) = severity {
//             self.lints.retain(|lint| severity.contains(&lint.severity()));
//         }
//         self
//     }

//     pub fn with_description(mut self, description: bool) -> Self {
//         self.description = description;
//         self
//     }

//     pub fn lint(self) {
//         let all_findings = self
//             .input
//             .par_iter()
//             .map(|file| {
//                 let lints = self.lints.clone();
//                 let mut local_findings = HashMap::new();

//                 // Create a new session for this file
//                 let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();
//                 let arena = ast::Arena::new();

//                 // Enter the session context for this thread
//                 let _ = sess.enter(|| -> solar_interface::Result<()> {
//                     let mut parser = solar_parse::Parser::from_file(&sess, &arena, file)?;

//                     let ast =
//                         parser.parse_file().map_err(|e| e.emit()).expect("Failed to parse file");

//                     // Run all lints on the parsed AST and collect findings
//                     for mut lint in lints {
//                         let results = lint.lint(&ast);
//                         local_findings.entry(lint).or_insert_with(Vec::new).extend(results);
//                     }

//                     Ok(())
//                 });

//                 local_findings
//             })
//             .collect::<Vec<HashMap<Lint, Vec<Span>>>>();

//         let mut aggregated_findings = HashMap::new();
//         for file_findings in all_findings {
//             for (lint, results) in file_findings {
//                 aggregated_findings.entry(lint).or_insert_with(Vec::new).extend(results);
//             }
//         }

//         // TODO: make the output nicer
//         for finding in aggregated_findings {
//             let (lint, results) = finding;
//             let _description = if self.description { lint.description() } else { "" };

//             for _result in results {
//                 // TODO: display the finding
//             }
//         }
//     }
// }

// macro_rules! declare_lints {
//     ($(($name:ident, $severity:expr, $lint_name:expr, $description:expr)),* $(,)?) => {
//         #[derive(Debug, Clone, PartialEq, Eq)]
//         pub enum Lint {
//             $(
//                 $name($name),
//             )*
//         }

//         impl Lint {
//             pub fn all() -> Vec<Self> {
//                 vec![
//                     $(
//                         Lint::$name($name::new()),
//                     )*
//                 ]
//             }

//             pub fn severity(&self) -> Severity {
//                 match self {
//                     $(
//                         Lint::$name(_) => $severity,
//                     )*
//                 }
//             }

//             pub fn name(&self) -> &'static str {
//                 match self {
//                     $(
//                         Lint::$name(_) => $lint_name,
//                     )*
//                 }
//             }

//             pub fn description(&self) -> &'static str {
//                 match self {
//                     $(
//                         Lint::$name(_) => $description,
//                     )*
//                 }
//             }

//             /// Lint a source unit and return the findings
//             pub fn lint(&mut self, source_unit: &SourceUnit<'_>) -> Vec<Span> {
//                 match self {
//                     $(
//                         Lint::$name(lint) => {
//                             lint.visit_source_unit(source_unit);
//                             lint.items.clone()
//                         },
//                     )*
//                 }
//             }
//         }

//         impl<'ast> Visit<'ast> for Lint {
//             fn visit_source_unit(&mut self, source_unit: &SourceUnit<'ast>) {
//                 match self {
//                     $(
//                         Lint::$name(lint) => lint.visit_source_unit(source_unit),
//                     )*
//                 }
//             }
//         }

//         impl std::hash::Hash for Lint {
//             fn hash<H: Hasher>(&self, state: &mut H) {
//                 self.name().hash(state);
//             }
//         }

//         $(
//             #[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
//             pub struct $name {
//                 pub items: Vec<Span>,
//             }

//             impl $name {
//                 pub fn new() -> Self {
//                     Self { items: Vec::new() }
//                 }

//                 /// Returns the severity of the lint
//                 pub fn severity() -> Severity {
//                     $severity
//                 }

//                 /// Returns the name of the lint
//                 pub fn name() -> &'static str {
//                     $lint_name
//                 }

//                 /// Returns the description of the lint
//                 pub fn description() -> &'static str {
//                     $description
//                 }
//             }
//         )*
//     };
// }

// declare_lints!(
//     //High
//     (IncorrectShift, Severity::High, "incorrect-shift", "TODO: description"),
//     (ArbitraryTransferFrom, Severity::High, "arbitrary-transfer-from", "TODO: description"),
//     // Med
//     (DivideBeforeMultiply, Severity::Med, "divide-before-multiply", "TODO: description"),
//     // Low
//     // Info
//     (VariableCamelCase, Severity::Info, "variable-camel-case", "TODO: description"),
//     (VariableCapsCase, Severity::Info, "variable-caps-case", "TODO: description"),
//     (StructPascalCase, Severity::Info, "struct-pascal-case", "TODO: description"),
//     (FunctionCamelCase, Severity::Info, "function-camel-case", "TODO: description"),
//     // Gas Optimizations
//     (AsmKeccak256, Severity::Gas, "asm-keccak256", "TODO: description"),
//     (PackStorageVariables, Severity::Gas, "pack-storage-variables", "TODO: description"),
//     (PackStructs, Severity::Gas, "pack-structs", "TODO: description"),
//     (UseConstantVariable, Severity::Gas, "use-constant-var", "TODO: description"),
//     (UseImmutableVariable, Severity::Gas, "use-immutable-var", "TODO: description"),
//     (UseExternalVisibility, Severity::Gas, "use-external-visibility", "TODO: description"),
//     (
//         AvoidUsingThis,
//         Severity::Gas,
//         "avoid-using-this",
//         "Avoid using `this` to read public variables. This incurs an unncessary STATICCALL."
//     ),
// );
