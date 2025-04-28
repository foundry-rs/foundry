pub mod gas;
pub mod high;
pub mod info;
pub mod med;

use crate::linter::{EarlyLintPass, EarlyLintVisitor, Lint, LintContext, Linter, Severity};
use foundry_compilers::solc::SolcLanguage;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use solar_ast::{visit::Visit, Arena};
use solar_interface::{
    diagnostics::{EmittedDiagnostics, ErrorGuaranteed},
    ColorChoice, Session,
};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Linter implementation to analyze Solidity source code responsible for identifying
/// vulnerabilities gas optimizations, and best practices.
#[derive(Debug, Clone, Default)]
pub struct SolidityLinter {
    severity: Option<Vec<Severity>>,
    lints_included: Option<Vec<SolLint>>,
    lints_excluded: Option<Vec<SolLint>>,
    with_description: bool,
    // This field is only used for testing purposes, in production it will always be false.
    with_buffer_emitter: bool,
}

impl SolidityLinter {
    pub fn new() -> Self {
        Self {
            severity: None,
            lints_included: None,
            lints_excluded: None,
            with_description: false,
            with_buffer_emitter: false,
        }
    }

    pub fn with_severity(mut self, severity: Option<Vec<Severity>>) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_lints(mut self, lints: Option<Vec<SolLint>>) -> Self {
        self.lints_included = lints;
        self
    }

    pub fn without_lints(mut self, lints: Option<Vec<SolLint>>) -> Self {
        self.lints_excluded = lints;
        self
    }

    pub fn with_description(mut self, description: bool) -> Self {
        self.with_description = description;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_buffer_emitter(mut self, with: bool) -> Self {
        self.with_buffer_emitter = with;
        self
    }

    // Helper function to ease testing, despite `fn lint` being the public API for the `Linter`
    pub(crate) fn lint_file(&self, file: &Path) -> Option<EmittedDiagnostics> {
        let mut sess = if self.with_buffer_emitter {
            Session::builder().with_buffer_emitter(ColorChoice::Never).build()
        } else {
            Session::builder().with_stderr_emitter().build()
        };
        sess.dcx = sess.dcx.set_flags(|flags| flags.track_diagnostics = false);

        let arena = Arena::new();

        let _ = sess.enter(|| -> Result<(), ErrorGuaranteed> {
            // Declare all available passes and lints
            let passes_and_lints: Vec<(Box<dyn EarlyLintPass<'_>>, SolLint)> = vec![
                (Box::new(AsmKeccak256), ASM_KECCACK256),
                (Box::new(IncorrectShift), INCORRECT_SHIFT),
                (Box::new(DivideBeforeMultiply), DIVIDE_BEFORE_MULTIPLY),
                (Box::new(VariableMixedCase), VARIABLE_MIXED_CASE),
                (Box::new(ScreamingSnakeCase), SCREAMING_SNAKE_CASE),
                (Box::new(StructPascalCase), STRUCT_PASCAL_CASE),
                (Box::new(FunctionMixedCase), FUNCTION_MIXED_CASE),
            ];

            // Filter based on linter config
            let mut passes: Vec<Box<dyn EarlyLintPass<'_>>> = passes_and_lints
                .into_iter()
                .filter_map(|(pass, lint)| {
                    let matches_severity = match self.severity {
                        Some(ref target) => target.contains(&lint.severity()),
                        None => true,
                    };
                    let matches_lints_inc = match self.lints_included {
                        Some(ref target) => target.contains(&lint),
                        None => true,
                    };
                    let matches_lints_exc = match self.lints_excluded {
                        Some(ref target) => target.contains(&lint),
                        None => false,
                    };

                    if matches_severity && matches_lints_inc && !matches_lints_exc {
                        Some(pass)
                    } else {
                        None
                    }
                })
                .collect();

            // Initialize the parser and get the AST
            let mut parser = solar_parse::Parser::from_file(&sess, &arena, file)?;
            let ast = parser.parse_file().map_err(|e| e.emit())?;

            // Initialize and run the visitor
            let ctx = LintContext::new(&sess, self.with_description);
            let mut visitor = EarlyLintVisitor { ctx: &ctx, passes: &mut passes };
            visitor.visit_source_unit(&ast);

            Ok(())
        });

        sess.emitted_diagnostics()
    }
}

impl Linter for SolidityLinter {
    type Language = SolcLanguage;
    type Lint = SolLint;

    fn lint(&self, input: &[PathBuf]) {
        input.into_par_iter().for_each(|file| {
            _ = self.lint_file(file);
        });
    }
}

#[derive(Error, Debug)]
pub enum SolLintError {
    #[error("Unknown lint ID: {0}")]
    InvalidId(String),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SolLint {
    id: &'static str,
    description: &'static str,
    help: Option<&'static str>,
    severity: Severity,
}

impl Lint for SolLint {
    fn id(&self) -> &'static str {
        self.id
    }
    fn severity(&self) -> Severity {
        self.severity
    }
    fn description(&self) -> &'static str {
        self.description
    }
    fn help(&self) -> Option<&'static str> {
        self.help
    }
}

macro_rules! declare_forge_lints {
    ($(($pass_id:ident, $lint_id:ident, $severity:expr, $str_id:expr, $description:expr, $help:expr)),* $(,)?) => {
        // Declare the static `Lint` metadata
        $(
            pub static $lint_id: SolLint = SolLint {
                id: $str_id,
                severity: $severity,
                description: $description,
                help: if $help.is_empty() { None } else { Some($help) }
            };
        )*

        // Implement TryFrom<&str> for `SolLint`
        impl<'a> TryFrom<&'a str> for SolLint {
            type Error = SolLintError;

            fn try_from(value: &'a str) -> Result<Self, Self::Error> {
                match value {
                    $(
                        $str_id => Ok($lint_id),
                    )*
                    _ => Err(SolLintError::InvalidId(value.to_string())),
                }
            }
        }

        // Declare the structs that will implement the pass trait
        $(
            #[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
            pub struct $pass_id;
        )*
    };
}

// Macro for defining lints and relevant metadata for the Solidity linter.
//
// This macro generates the [`SolLint`] enum with each lint along with utility methods and
// corresponding structs for each lint specified.
//
// # Parameters
//
// Each lint is defined as a tuple with the following fields:
// - `$id`: Identitifier used as the struct and enum variant created for the lint.
// - `$severity`: The [`Severity`] of the lint (e.g. `High`, `Med`, `Low`, `Info`, `Gas`).
// - `$description`: A short description of the lint.
// - `$help`: Link to additional information about the lint or best practices.
// - `$str_id`: A unique identifier used to reference a specific lint during configuration.
declare_forge_lints!(
    //High
    (
        IncorrectShift,
        INCORRECT_SHIFT,
        Severity::High,
        "incorrect-shift",
        "The order of args in a shift operation is incorrect",
        ""
    ),
    // Med
    (
        DivideBeforeMultiply,
        DIVIDE_BEFORE_MULTIPLY,
        Severity::Med,
        "divide-before-multiply",
        "Multiplication should occur before division to avoid loss of precision",
        ""
    ),
    // Low

    // Info
    (
        VariableMixedCase,
        VARIABLE_MIXED_CASE,
        Severity::Info,
        "variable-mixed-case",
        "Mutable variables should use mixedCase",
        ""
    ),
    (
        ScreamingSnakeCase,
        SCREAMING_SNAKE_CASE,
        Severity::Info,
        "screaming-snake-case",
        "Constants and immutables should use SCREAMING_SNAKE_CASE",
        "https://docs.soliditylang.org/en/latest/style-guide.html#contract-and-library-names"
    ),
    (
        StructPascalCase,
        STRUCT_PASCAL_CASE,
        Severity::Info,
        "struct-pascal-case",
        "Structs should use PascalCase.",
        "https://docs.soliditylang.org/en/latest/style-guide.html#struct-names"
    ),
    (
        FunctionMixedCase,
        FUNCTION_MIXED_CASE,
        Severity::Info,
        "function-mixed-case",
        "Function names should use mixedCase.",
        "https://docs.soliditylang.org/en/latest/style-guide.html#function-names"
    ),
    // Gas Optimizations
    (
        AsmKeccak256,
        ASM_KECCACK256,
        Severity::Gas,
        "asm-keccack256",
        "Hash via inline assembly to save gas",
        ""
    ),
    // TODO: PackStorageVariables
    // TODO: PackStructs
    // TODO: UseConstantVariable
    // TODO: UseImmutableVariable
    // TODO: UseCalldataInsteadOfMemory
);
