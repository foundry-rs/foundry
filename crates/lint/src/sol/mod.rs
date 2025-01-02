pub mod gas;
pub mod high;
pub mod info;
pub mod med;

use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
};

use foundry_compilers::solc::SolcLanguage;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use solar_ast::{
    ast::{Arena, SourceUnit},
    visit::Visit,
};
use solar_interface::{ColorChoice, Session, Span};
use thiserror::Error;

use crate::{Lint, Linter, LinterOutput, Severity, SourceLocation};

#[derive(Debug, Clone, Default)]
pub struct SolidityLinter {
    pub severity: Option<Vec<Severity>>,
    pub description: bool,
}

impl SolidityLinter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_description(mut self, description: bool) -> Self {
        self.description = description;
        self
    }

    pub fn with_severity(mut self, severity: Option<Vec<Severity>>) -> Self {
        self.severity = severity;
        self
    }
}

impl Linter for SolidityLinter {
    type Language = SolcLanguage;
    type Lint = SolLint;
    type LinterError = SolLintError;

    fn lint(&self, input: &[PathBuf]) -> Result<LinterOutput<Self>, Self::LinterError> {
        let all_findings = input
            .into_par_iter()
            .map(|file| {
                let mut lints = if let Some(severity) = &self.severity {
                    SolLint::with_severity(severity.to_owned())
                } else {
                    SolLint::all()
                };

                // Initialize session and parsing environment
                let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();
                let arena = Arena::new();

                // Enter the session context for this thread
                let _ = sess.enter(|| -> solar_interface::Result<()> {
                    let mut parser = solar_parse::Parser::from_file(&sess, &arena, file)?;
                    let ast =
                        parser.parse_file().map_err(|e| e.emit()).expect("Failed to parse file");

                    // Run all lints on the parsed AST and collect findings
                    for lint in lints.iter_mut() {
                        lint.lint(&ast);
                    }

                    Ok(())
                });

                (file.to_owned(), lints)
            })
            .collect::<Vec<(PathBuf, Vec<SolLint>)>>();

        let mut output = LinterOutput::new();
        for (file, lints) in all_findings {
            for lint in lints {
                let source_locations = lint
                    .results()
                    .iter()
                    .map(|span| SourceLocation::new(file.clone(), *span))
                    .collect::<Vec<_>>();

                if source_locations.is_empty() {
                    continue;
                }

                output.insert(lint, source_locations);
            }
        }

        Ok(output)
    }
}

#[derive(Error, Debug)]
pub enum SolLintError {}

macro_rules! declare_sol_lints {
    ($(($name:ident, $severity:expr, $lint_name:expr, $description:expr, $url:expr)),* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
        pub enum SolLint {
            $(
                $name($name),
            )*
        }

        impl SolLint {
            pub fn all() -> Vec<Self> {
                vec![
                    $(
                        SolLint::$name($name::new()),
                    )*
                ]
            }

            pub fn results(&self) -> &[Span] {
                match self {
                    $(
                        SolLint::$name(lint) => &lint.results,
                    )*
                }
            }

            pub fn with_severity(severity: Vec<Severity>) -> Vec<Self> {
                Self::all()
                .into_iter()
                .filter(|lint| severity.contains(&lint.severity()))
                .collect()
            }

            pub fn lint(&mut self, source_unit: &SourceUnit<'_>) -> Vec<Span> {
                match self {
                    $(
                        SolLint::$name(lint) => {
                            lint.visit_source_unit(source_unit);
                            lint.results.clone()
                        },
                    )*
                }
            }
        }

        impl<'ast> Visit<'ast> for SolLint {
            fn visit_source_unit(&mut self, source_unit: &SourceUnit<'ast>) {
                match self {
                    $(
                        SolLint::$name(lint) => lint.visit_source_unit(source_unit),
                    )*
                }
            }
        }

        impl Hash for SolLint {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.name().hash(state);
            }
        }

        impl Lint for SolLint {
            fn name(&self) -> &'static str {
                match self {
                    $(
                        SolLint::$name(_) => $lint_name,
                    )*
                }
            }

            fn description(&self) -> &'static str {
                match self {
                    $(
                        SolLint::$name(_) => $description,
                    )*
                }
            }

            fn severity(&self) -> Severity {
                match self {
                    $(
                        SolLint::$name(_) => $severity,
                    )*
                }
            }

            fn url(&self) -> Option<&'static str> {
                match self {
                    $(
                        SolLint::$name(_) => {
                            if !$url.is_empty() {
                                Some($url)
                            } else {
                                None
                            }
                        },
                    )*
                }
            }
        }

        $(
            #[derive(Debug, Default, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
            pub struct $name {
                pub results: Vec<Span>,
            }

            impl $name {
                pub fn new() -> Self {
                    Self { results: Vec::new() }
                }
            }
        )*
    };
}

declare_sol_lints!(
    //High
    (IncorrectShift, Severity::High, "incorrect-shift", "The order of args in a shift operation is incorrect.", ""),
    // Med
    (DivideBeforeMultiply, Severity::Med, "divide-before-multiply", "Multiplication should occur before division to avoid loss of precision.", ""),
    // Low
    // Info
    (VariableMixedCase, Severity::Info, "variable-mixed-case", "Variables should follow `camelCase` naming conventions unless they are constants or immutables.", ""),
    (ScreamingSnakeCase, Severity::Info, "screaming-snake-case", "Constants and immutables should be named with all capital letters with underscores separating words.", "https://docs.soliditylang.org/en/latest/style-guide.html#contract-and-library-names"),
    (StructPascalCase, Severity::Info, "struct-pascal-case", "Structs should be named using PascalCase. Examples: MyCoin, Position", "https://docs.soliditylang.org/en/latest/style-guide.html#struct-names"),
    (FunctionMixedCase, Severity::Info, "function-mixed-case", "Constants should be named with all capital letters with underscores separating words.", "https://docs.soliditylang.org/en/latest/style-guide.html#function-names"),
    // Gas Optimizations
    (AsmKeccak256, Severity::Gas, "asm-keccak256", "Hashing via keccak256 can be done with inline assembly to save gas.", "https://placeholder.xyz"),
    // TODO: PackStorageVariables
    // TODO: PackStructs
    // TODO: UseConstantVariable
    // TODO: UseImmutableVariable
    // TODO: UseCalldataInsteadOfMemory
);
