pub mod gas;
pub mod high;
pub mod info;
pub mod med;

use std::{
    hash::{Hash, Hasher},
    ops::ControlFlow,
    path::PathBuf,
};

use foundry_compilers::solc::SolcLanguage;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use solar_ast::{visit::Visit, Arena, SourceUnit};
use solar_interface::{diagnostics::Level, Session, Span};
use thiserror::Error;

use crate::linter::{Lint, Linter, Severity};

/// Linter implementation to analyze Solidity source code responsible for identifying
/// vulnerabilities gas optimizations, and best practices.
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

    fn lint(&self, input: &[PathBuf]) -> Result<(), Self::LinterError> {
        let _ = input
            .into_par_iter()
            .map(|file| {
                let mut lints = if let Some(severity) = &self.severity {
                    SolLint::with_severity(severity.to_owned())
                } else {
                    SolLint::all()
                };

                let mut sess = Session::builder().with_stderr_emitter().build();
                sess.dcx = sess.dcx.set_flags(|flags| flags.track_diagnostics = false);

                let arena = Arena::new();

                let _ = sess.enter(|| -> solar_interface::Result<()> {
                    let mut parser = solar_parse::Parser::from_file(&sess, &arena, file)?;
                    let ast =
                        parser.parse_file().map_err(|e| e.emit()).expect("Failed to parse file");

                    // Run all lints on the parsed AST
                    for lint in lints.iter_mut() {
                        for span in lint.lint(&ast) {
                            sess.dcx
                                .diag::<()>(
                                    Level::Warning,
                                    format!("{}: {}", lint.severity(), lint.description()),
                                )
                                .span(span)
                                .help(lint.help().unwrap_or_default())
                                .emit()
                        }
                    }
                    Ok(())
                });
            })
            .collect::<Vec<_>>();

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum SolLintError {}

/// Macro for defining lints and relevant metadata for the Solidity linter.
///
/// This macro generates the [`SolLint`] enum with each lint along with utility methods and
/// corresponding structs for each lint specified.
///
/// # Parameters
///
/// Each lint is defined as a tuple with the following fields:
/// - `$name`: Identitifier used for the struct and enum variant created for the lint.
/// - `$severity`: The [`Severity`] of the lint (e.g. `High`, `Med`, `Low`, `Info`, `Gas`).
/// - `$lint_name`: A string identifier for the lint used during reporting.
/// - `$description`: A short description of the lint.
/// - `$url`: URL providing additional information about the lint or best practices.
macro_rules! declare_sol_lints {
    ($(($name:ident, $severity:expr, $lint_name:expr, $description:expr, $help:expr)),* $(,)?) => {
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
            type BreakValue = ();
            fn visit_source_unit(&mut self, source_unit: &SourceUnit<'ast>) -> ControlFlow<Self::BreakValue> {
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

            fn help(&self) -> Option<&'static str> {
                match self {
                    $(
                        SolLint::$name(_) => {
                            if !$help.is_empty() {
                                Some($help)
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
    // TODO: FunctionMixedCase

    // Gas Optimizations
    (AsmKeccak256, Severity::Gas, "asm-keccak256", "Hashing via keccak256 can be done with inline assembly to save gas.", ""),
    // TODO: PackStorageVariables
    // TODO: PackStructs
    // TODO: UseConstantVariable
    // TODO: UseImmutableVariable
    // TODO: UseCalldataInsteadOfMemory
);
