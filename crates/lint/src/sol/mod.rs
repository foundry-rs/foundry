pub mod macros;

pub mod gas;
pub mod high;
pub mod info;
pub mod med;

use crate::linter::{EarlyLintPass, EarlyLintVisitor, Lint, LintContext, Linter};
use foundry_compilers::solc::SolcLanguage;
use foundry_config::lint::Severity;
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

    pub fn with_description(mut self, with: bool) -> Self {
        self.with_description = with;
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
            let mut passes_and_lints = Vec::new();
            passes_and_lints.extend(gas::create_lint_passes());
            passes_and_lints.extend(high::create_lint_passes());
            passes_and_lints.extend(med::create_lint_passes());
            passes_and_lints.extend(info::create_lint_passes());

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
            let _ = visitor.visit_source_unit(&ast);

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

impl<'a> TryFrom<&'a str> for SolLint {
    type Error = SolLintError;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        for &lint in high::REGISTERED_LINTS {
            if lint.id() == value {
                return Ok(lint);
            }
        }

        for &lint in med::REGISTERED_LINTS {
            if lint.id() == value {
                return Ok(lint);
            }
        }

        for &lint in info::REGISTERED_LINTS {
            if lint.id() == value {
                return Ok(lint);
            }
        }

        for &lint in gas::REGISTERED_LINTS {
            if lint.id() == value {
                return Ok(lint);
            }
        }

        Err(SolLintError::InvalidId(value.to_string()))
    }
}
