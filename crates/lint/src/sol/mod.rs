pub mod gas;
pub mod high;
pub mod info;
pub mod med;

use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
};

use eyre::Error;
use foundry_compilers::solc::SolcLanguage;
use solar_ast::{ast::SourceUnit, visit::Visit};
use solar_interface::{
    diagnostics::{DiagnosticBuilder, ErrorGuaranteed},
    ColorChoice, Session, Span,
};
use thiserror::Error;

use crate::{Lint, Linter, LinterOutput, Severity, SourceLocation};

#[derive(Debug, Clone)]
pub struct SolidityLinter {}

impl Linter for SolidityLinter {
    type Lint = SolLint;
    type Language = SolcLanguage;
    type LinterError = SolLintError;

    fn lint(&self, input: &[PathBuf]) -> Result<LinterOutput<Self>, Self::LinterError> {
        // let all_findings = input
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

        todo!()
    }
}

#[derive(Error, Debug)]
pub enum SolLintError {}

macro_rules! declare_sol_lints {
    ($(($name:ident, $severity:expr, $lint_name:expr, $description:expr)),* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
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
            pub fn lint(&mut self, source_unit: &SourceUnit<'_>) {
                match self {
                    $(
                        SolLint::$name(lint) => {
                            lint.visit_source_unit(source_unit);
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
            #[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
            pub struct $name {
                // TODO: make source location and option
                pub results: Vec<Span>,
            }

            impl $name {
                pub fn new() -> Self {
                    Self { results: Vec::new() }
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
