pub mod gas;
pub mod high;
pub mod info;
pub mod med;

use std::{
    collections::{BTreeMap, HashMap},
    hash::{Hash, Hasher},
    path::PathBuf,
};

use eyre::Error;
use foundry_compilers::solc::SolcLanguage;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use solar_ast::{
    ast::{Arena, SourceUnit},
    visit::Visit,
};
use solar_interface::{
    diagnostics::{DiagnosticBuilder, ErrorGuaranteed},
    ColorChoice, Session, Span,
};
use thiserror::Error;

use crate::{Lint, Linter, LinterOutput, Severity, SourceLocation};

#[derive(Debug, Clone)]
pub struct SolidityLinter {}

impl Linter for SolidityLinter {
    type Language = SolcLanguage;
    type Lint = SolLint;
    type LinterError = SolLintError;

    fn lint(&self, input: &[PathBuf]) -> Result<LinterOutput<Self>, Self::LinterError> {
        let all_findings = input.into_par_iter().map(|file| {
            // NOTE: use all solidity lints for now but this should be configurable via SolidityLinter
            let lints = SolLint::all();

            // Initialize session and parsing environment
            let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();
            let arena = Arena::new();

            // Enter the session context for this thread
            let _ = sess.enter(|| -> solar_interface::Result<LinterOutput<Self>> {
                let mut parser = solar_parse::Parser::from_file(&sess, &arena, file)?;
                let ast = parser.parse_file().map_err(|e| e.emit()).expect("Failed to parse file");

                let mut local_findings = LinterOutput::new();
                // Run all lints on the parsed AST and collect findings
                for mut lint in lints.into_iter() {
                    if let Some(findings) = lint.lint(&ast) {
                        let findings = findings
                            .into_iter()
                            .map(|span| SourceLocation::new(file.to_owned(), span))
                            .collect::<Vec<_>>();

                        local_findings.insert(lint, findings);
                    }
                }

                Ok(local_findings)
            });
        });

        todo!()
    }
}

#[derive(Error, Debug)]
pub enum SolLintError {}

macro_rules! declare_sol_lints {
    ($(($name:ident, $severity:expr, $lint_name:expr, $description:expr)),* $(,)?) => {

        // TODO: ord based on severity
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

            /// Lint a source unit and return the findings
            pub fn lint(&mut self, source_unit: &SourceUnit<'_>) -> Option<Vec<Span>> {
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
        }


        $(
            #[derive(Debug, Default, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
            pub struct $name {
                // TODO: make source location and option
                pub results: Option<Vec<Span>>,
            }

            impl $name {
                pub fn new() -> Self {
                    Self { results: None }
                }

                /// Returns the severity of the lint
                pub fn severity() -> Severity {
                    $severity
                }

                /// Returns the name of the lint
                pub fn name() -> &'static str {
                    $lint_name
                }

                /// Returns the description of the lint
                pub fn description() -> &'static str {
                    $description
                }
            }
        )*
    };
}

declare_sol_lints!(
    //High
    (IncorrectShift, Severity::High, "incorrect-shift", "TODO: description"),
    (ArbitraryTransferFrom, Severity::High, "arbitrary-transfer-from", "TODO: description"),
    // Med
    (DivideBeforeMultiply, Severity::Med, "divide-before-multiply", "TODO: description"),
    // Low
    // Info
    (VariableCamelCase, Severity::Info, "variable-camel-case", "TODO: description"),
    (VariableCapsCase, Severity::Info, "variable-caps-case", "TODO: description"),
    (StructPascalCase, Severity::Info, "struct-pascal-case", "TODO: description"),
    (FunctionCamelCase, Severity::Info, "function-camel-case", "TODO: description"),
    // Gas Optimizations
    (AsmKeccak256, Severity::Gas, "asm-keccak256", "TODO: description"),
    (PackStorageVariables, Severity::Gas, "pack-storage-variables", "TODO: description"),
    (PackStructs, Severity::Gas, "pack-structs", "TODO: description"),
    (UseConstantVariable, Severity::Gas, "use-constant-var", "TODO: description"),
    (UseImmutableVariable, Severity::Gas, "use-immutable-var", "TODO: description"),
    (UseExternalVisibility, Severity::Gas, "use-external-visibility", "TODO: description"),
    (
        AvoidUsingThis,
        Severity::Gas,
        "avoid-using-this",
        "Avoid using `this` to read public variables. This incurs an unncessary STATICCALL."
    ),
);
