pub mod gas;
pub mod high;
pub mod info;
pub mod med;

use std::{
    hash::{Hash, Hasher},
    ops::ControlFlow,
    path::PathBuf,
};

use crate::linter::{EarlyLintPass, Lint, LintContext, Severity};
use foundry_compilers::solc::SolcLanguage;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use solar_ast::{visit::Visit, Arena, SourceUnit};
use solar_interface::{
    diagnostics::{ErrorGuaranteed, Level},
    Session, Span,
};
use thiserror::Error;
use yansi::Paint;

/// Linter implementation to analyze Solidity source code responsible for identifying
/// vulnerabilities gas optimizations, and best practices.
pub struct SolidityLinter<'s> {
    pub severity: Option<Vec<Severity>>,
    pub description: bool,
    // Store registered passes. Using Box for dynamic dispatch.
    // The lifetime 's links the passes to the Session lifetime.
    pub passes: Vec<Box<dyn EarlyLintPass<'s> + 's>>,
    pub session: &'s Session, 
}

impl<'s> SolidityLinter<'s> {
    // Pass the session during creation
    pub fn new(sess: &'s Session) -> Self {
        Self { passes: Vec::new(), sess }
    }

    // Method to register passes
    pub fn register_early_pass(&mut self, pass: Box<dyn EarlyLintPass<'s> + 's>) {
        self.passes.push(pass);
    }

    // TODO: Add logic to register passes based on config (e.g., severity from LintArgs)

    // The main method to run the linting for a single AST
    pub fn run_passes<'ast>(&mut self, source_unit: &'ast SourceUnit<'ast>)
    where
         's: 'ast, // Ensure session lives at least as long as AST
     {
        // Create the context for this run
        let cx = LintContext::new(self.sess);

        // Create a visitor helper struct or implement Visit directly
        let mut visitor = LintVisitor { cx: &cx, passes: &mut self.passes };
        visitor.visit_source_unit(source_unit);
    }
}

#[derive(Error, Debug)]
pub enum SolLintError {}

macro_rules! declare_forge_lints {
    ($(($struct_id:ident, $lint_id:ident, $severity:expr, $str_id:expr, $description:expr, $help:expr)),* $(,)?) => {
        // Declare the static `Lint` metadata
        $(
            pub static $lint_id: crate::linter::Lint = crate::linter::Lint {
                id: $str_id,
                severity: $severity,
                description: $description,
                help: if $help.is_empty() { None } else { Some($help) }
            };
        )*

        // Declare the structs that will implement the pass trait
        $(
            #[derive(Debug, Default, Clone, Copy)]
            pub struct $struct_id;

            impl $struct_id {
               pub fn new() -> Self { Self }
            }
        )*
    };
}

/// Macro for defining lints and relevant metadata for the Solidity linter.
///
/// This macro generates the [`SolLint`] enum with each lint along with utility methods and
/// corresponding structs for each lint specified.
///
/// # Parameters
///
/// Each lint is defined as a tuple with the following fields:
/// - `$id`: Identitifier used as the struct and enum variant created for the lint.
/// - `$severity`: The [`Severity`] of the lint (e.g. `High`, `Med`, `Low`, `Info`, `Gas`).
/// - `$description`: A short description of the lint.
/// - `$help`: Link to additional information about the lint or best practices.
/// - `$str_id`: A unique identifier used to reference a specific lint during configuration.
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
    // TODO: FunctionMixedCase

    // Gas Optimizations
    (AsmKeccak256, ASM_KECCACK256, Severity::Gas, "asm-keccack256", "Hash via inline assembly to save gas", ""),
    // TODO: PackStorageVariables
    // TODO: PackStructs
    // TODO: UseConstantVariable
    // TODO: UseImmutableVariable
    // TODO: UseCalldataInsteadOfMemory
);
