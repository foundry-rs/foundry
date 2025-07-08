use crate::{
    inline_config::{InlineConfig, InlineConfigItem},
    linter::{EarlyLintPass, EarlyLintVisitor, Lint, LintContext, Linter},
};
use foundry_common::comments::Comments;
use foundry_compilers::{ProjectPathsConfig, solc::SolcLanguage};
use foundry_config::lint::Severity;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use solar_ast::{Arena, SourceUnit, visit::Visit};
use solar_interface::{
    Session, SourceMap,
    diagnostics::{self, DiagCtxt, JsonEmitter},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

#[macro_use]
pub mod macros;

pub mod gas;
pub mod high;
pub mod info;
pub mod med;

/// Linter implementation to analyze Solidity source code responsible for identifying
/// vulnerabilities gas optimizations, and best practices.
#[derive(Debug, Clone)]
pub struct SolidityLinter {
    path_config: ProjectPathsConfig,
    severity: Option<Vec<Severity>>,
    lints_included: Option<Vec<SolLint>>,
    lints_excluded: Option<Vec<SolLint>>,
    with_description: bool,
    with_json_emitter: bool,
}

impl SolidityLinter {
    pub fn new(path_config: ProjectPathsConfig) -> Self {
        Self {
            path_config,
            severity: None,
            lints_included: None,
            lints_excluded: None,
            with_description: true,
            with_json_emitter: false,
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

    pub fn with_json_emitter(mut self, with: bool) -> Self {
        self.with_json_emitter = with;
        self
    }

    fn process_file(&self, sess: &Session, path: &Path) {
        let arena = Arena::new();

        let _ = sess.enter(|| -> Result<(), diagnostics::ErrorGuaranteed> {
            // Initialize the parser, get the AST, and process the inline-config
            let mut parser = solar_parse::Parser::from_file(sess, &arena, path)?;
            let file = sess
                .source_map()
                .load_file(path)
                .map_err(|e| sess.dcx.err(e.to_string()).emit())?;
            let ast = parser.parse_file().map_err(|e| e.emit())?;

            // Declare all available passes and lints
            let mut passes_and_lints = Vec::new();
            passes_and_lints.extend(high::create_lint_passes());
            passes_and_lints.extend(med::create_lint_passes());
            passes_and_lints.extend(info::create_lint_passes());

            // Do not apply gas-severity rules on tests and scripts
            if !self.path_config.is_test_or_script(path) {
                passes_and_lints.extend(gas::create_lint_passes());
            }

            // Filter based on linter config
            let (lints, mut passes): (Vec<&str>, Vec<Box<dyn EarlyLintPass<'_>>>) =
                passes_and_lints
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
                            Some((lint.id(), pass))
                        } else {
                            None
                        }
                    })
                    .unzip();

            // Process the inline-config
            let source = file.src.as_str();
            let comments = Comments::new(&file);
            let inline_config = parse_inline_config(sess, &comments, &lints, &ast, source);

            // Initialize and run the visitor
            let ctx = LintContext::new(sess, self.with_description, inline_config);
            let mut visitor = EarlyLintVisitor { ctx: &ctx, passes: &mut passes };
            _ = visitor.visit_source_unit(&ast);
            visitor.post_source_unit(&ast);

            Ok(())
        });
    }
}

impl Linter for SolidityLinter {
    type Language = SolcLanguage;
    type Lint = SolLint;

    fn lint(&self, input: &[PathBuf]) {
        let mut builder = Session::builder();

        // Build session based on the linter config
        if self.with_json_emitter {
            let map = Arc::<SourceMap>::default();
            let json_emitter = JsonEmitter::new(Box::new(std::io::stderr()), map.clone())
                .rustc_like(true)
                .ui_testing(false);

            builder = builder.dcx(DiagCtxt::new(Box::new(json_emitter))).source_map(map);
        } else {
            builder = builder.with_stderr_emitter();
        };

        // Create a single session for all files
        let mut sess = builder.build();
        sess.dcx = sess.dcx.set_flags(|flags| flags.track_diagnostics = false);

        // Process the files in parallel
        sess.enter_parallel(|| {
            input.into_par_iter().for_each(|file| {
                self.process_file(&sess, file);
            });
        });
    }
}

fn parse_inline_config<'ast>(
    sess: &Session,
    comments: &Comments,
    lints: &[&'static str],
    ast: &'ast SourceUnit<'ast>,
    src: &str,
) -> InlineConfig {
    let items = comments.iter().filter_map(|comment| {
        let mut item = comment.lines.first()?.as_str();
        if let Some(prefix) = comment.prefix() {
            item = item.strip_prefix(prefix).unwrap_or(item);
        }
        if let Some(suffix) = comment.suffix() {
            item = item.strip_suffix(suffix).unwrap_or(item);
        }
        let item = item.trim_start().strip_prefix("forge-lint:")?.trim();
        let span = comment.span;
        match InlineConfigItem::parse(item, lints) {
            Ok(item) => Some((span, item)),
            Err(e) => {
                sess.dcx.warn(e.to_string()).span(span).emit();
                None
            }
        }
    });

    InlineConfig::new(items, ast, src)
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
    help: &'static str,
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
    fn help(&self) -> &'static str {
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
