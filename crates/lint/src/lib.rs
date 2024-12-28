pub mod gas;
pub mod high;
pub mod info;
pub mod med;

use rayon::prelude::*;
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
    // TODO: remove later
    phantom: PhantomData<L>,
}

impl<L> ProjectLinter<L>
where
    L: Linter,
{
    // pub fn new() -> Self {
    //     Self {}
    // }

    /// Lints the project.
    pub fn lint<C: Compiler<CompilerContract = Contract>>(
        mut self,
        project: &Project<C>,
    ) -> Result<LinterOutput, LinterError> {
        if !project.paths.has_input_files() && self.files.is_empty() {
            sh_println!("Nothing to compile")?;
            // nothing to do here
            std::process::exit(0);
        }

        // Taking is fine since we don't need these in `compile_with`.
        let files = std::mem::take(&mut self.files);
        self.compile_with(|| {
            let sources = if !files.is_empty() {
                Source::read_all(files)?
            } else {
                project.paths.read_input_files()?
            };

            todo!()
        })
    }
}

/// The main linter abstraction trait
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait Linter: Send + Sync + Clone {
    // TODO: keep this
    /// Input type for the compiler. Contains settings and sources to be compiled.
    type Input: CompilerInput<Settings = Self::Settings, Language = Self::Language>;
    /// Compiler settings.
    type Settings: LinterSettings;
    type LinterError: Error;
    /// Enum of languages supported by the linter.
    type Language: Language;

    /// Main entrypoint for the linter.
    fn lint(&self, input: &Self::Input) -> Result<LinterOutput, Self::LinterError>;
}

// TODO: make Lint a generic
pub struct LinterOutput<L: Lint> {
    pub results: BTreeMap<L, Vec<SourceLocation>>,
}

pub trait Lint: Hash {}

/// Linter for languages supported by the Solc compiler
pub struct SolcLinter {}

impl Language for SolcLinter {
    const FILE_EXTENSIONS: &'static [&'static str] = SOLC_EXTENSIONS;
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    pub file: String,
    pub span: Span,
}

// pub struct Linter {
//     pub input: Vec<PathBuf>,
//     pub lints: Vec<Lint>,
//     pub description: bool,
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
