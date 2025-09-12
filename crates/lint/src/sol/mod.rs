use crate::{
    inline_config::{InlineConfig, InlineConfigItem},
    linter::{
        EarlyLintPass, EarlyLintVisitor, LateLintPass, LateLintVisitor, Lint, LintContext, Linter,
        LinterConfig,
    },
};
use foundry_common::{comments::Comments, errors::convert_solar_errors};
use foundry_compilers::{ProjectPathsConfig, solc::SolcLanguage};
use foundry_config::lint::Severity;
use rayon::prelude::*;
use solar::{
    ast::{self as ast, visit::Visit as _},
    interface::{
        Session,
        diagnostics::{self, HumanEmitter, JsonEmitter},
        source_map::{FileName, SourceFile},
    },
    sema::{
        Compiler, Gcx,
        hir::{self, Visit as _},
    },
};
use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};
use thiserror::Error;

#[macro_use]
pub mod macros;

pub mod codesize;
pub mod gas;
pub mod high;
pub mod info;
pub mod med;

static ALL_REGISTERED_LINTS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut lints = Vec::new();
    lints.extend_from_slice(high::REGISTERED_LINTS);
    lints.extend_from_slice(med::REGISTERED_LINTS);
    lints.extend_from_slice(info::REGISTERED_LINTS);
    lints.extend_from_slice(gas::REGISTERED_LINTS);
    lints.extend_from_slice(codesize::REGISTERED_LINTS);
    lints.into_iter().map(|lint| lint.id()).collect()
});

/// Linter implementation to analyze Solidity source code responsible for identifying
/// vulnerabilities gas optimizations, and best practices.
#[derive(Debug)]
pub struct SolidityLinter<'a> {
    path_config: ProjectPathsConfig,
    severity: Option<Vec<Severity>>,
    lints_included: Option<Vec<SolLint>>,
    lints_excluded: Option<Vec<SolLint>>,
    with_description: bool,
    with_json_emitter: bool,
    mixed_case_exceptions: &'a [String],
}

impl<'a> SolidityLinter<'a> {
    pub fn new(path_config: ProjectPathsConfig) -> Self {
        Self {
            path_config,
            with_description: true,
            severity: None,
            lints_included: None,
            lints_excluded: None,
            with_json_emitter: false,
            mixed_case_exceptions: &[],
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

    pub fn with_mixed_case_exceptions(mut self, exceptions: &'a [String]) -> Self {
        self.mixed_case_exceptions = exceptions;
        self
    }

    fn config(&self, inline: InlineConfig) -> LinterConfig<'_> {
        LinterConfig { inline, mixed_case_exceptions: self.mixed_case_exceptions }
    }

    fn include_lint(&self, lint: SolLint) -> bool {
        self.severity.as_ref().is_none_or(|sev| sev.contains(&lint.severity()))
            && self.lints_included.as_ref().is_none_or(|incl| incl.contains(&lint))
            && !self.lints_excluded.as_ref().is_some_and(|excl| excl.contains(&lint))
    }

    fn process_source_ast<'gcx>(
        &self,
        sess: &'gcx Session,
        ast: &'gcx ast::SourceUnit<'gcx>,
        file: &SourceFile,
        path: &Path,
    ) -> Result<(), diagnostics::ErrorGuaranteed> {
        // Declare all available passes and lints
        let mut passes_and_lints = Vec::new();
        passes_and_lints.extend(high::create_early_lint_passes());
        passes_and_lints.extend(med::create_early_lint_passes());
        passes_and_lints.extend(info::create_early_lint_passes());

        // Do not apply 'gas' and 'codesize' severity rules on tests and scripts
        if !self.path_config.is_test_or_script(path) {
            passes_and_lints.extend(gas::create_early_lint_passes());
            passes_and_lints.extend(codesize::create_early_lint_passes());
        }

        // Filter passes based on linter config
        let (mut passes, lints): (Vec<Box<dyn EarlyLintPass<'_>>>, Vec<_>) = passes_and_lints
            .into_iter()
            .fold((Vec::new(), Vec::new()), |(mut passes, mut ids), (pass, lints)| {
                let included_ids: Vec<_> = lints
                    .iter()
                    .filter_map(|lint| if self.include_lint(*lint) { Some(lint.id) } else { None })
                    .collect();

                if !included_ids.is_empty() {
                    passes.push(pass);
                    ids.extend(included_ids);
                }

                (passes, ids)
            });

        // Process the inline-config
        let comments = Comments::new(file);
        let inline_config = parse_inline_config(sess, &comments, InlineConfigSource::Ast(ast));

        // Initialize and run the early lint visitor
        let ctx = LintContext::new(
            sess,
            self.with_description,
            self.with_json_emitter,
            self.config(inline_config),
            lints,
        );
        let mut early_visitor = EarlyLintVisitor::new(&ctx, &mut passes);
        _ = early_visitor.visit_source_unit(ast);
        early_visitor.post_source_unit(ast);

        Ok(())
    }

    fn process_source_hir<'gcx>(
        &self,
        gcx: Gcx<'gcx>,
        source_id: hir::SourceId,
        file: &'gcx SourceFile,
    ) -> Result<(), diagnostics::ErrorGuaranteed> {
        // Declare all available passes and lints
        let mut passes_and_lints = Vec::new();
        passes_and_lints.extend(high::create_late_lint_passes());
        passes_and_lints.extend(med::create_late_lint_passes());
        passes_and_lints.extend(info::create_late_lint_passes());

        // Do not apply 'gas' and 'codesize' severity rules on tests and scripts
        if let FileName::Real(path) = &file.name
            && !self.path_config.is_test_or_script(path)
        {
            passes_and_lints.extend(gas::create_late_lint_passes());
            passes_and_lints.extend(codesize::create_late_lint_passes());
        }

        // Filter passes based on config
        let (mut passes, lints): (Vec<Box<dyn LateLintPass<'_>>>, Vec<_>) = passes_and_lints
            .into_iter()
            .fold((Vec::new(), Vec::new()), |(mut passes, mut ids), (pass, lints)| {
                let included_ids: Vec<_> = lints
                    .iter()
                    .filter_map(|lint| if self.include_lint(*lint) { Some(lint.id) } else { None })
                    .collect();

                if !included_ids.is_empty() {
                    passes.push(pass);
                    ids.extend(included_ids);
                }

                (passes, ids)
            });

        // Process the inline-config
        let comments = Comments::new(file);
        let inline_config = parse_inline_config(
            gcx.sess,
            &comments,
            InlineConfigSource::Hir((&gcx.hir, source_id)),
        );

        // Run late lint visitor
        let ctx = LintContext::new(
            gcx.sess,
            self.with_description,
            self.with_json_emitter,
            self.config(inline_config),
            lints,
        );
        let mut late_visitor = LateLintVisitor::new(&ctx, &mut passes, &gcx.hir);

        // Visit this specific source
        let _ = late_visitor.visit_nested_source(source_id);

        Ok(())
    }
}

impl<'a> Linter for SolidityLinter<'a> {
    type Language = SolcLanguage;
    type Lint = SolLint;

    fn lint(&self, input: &[PathBuf], compiler: &mut Compiler) -> eyre::Result<()> {
        let ui_testing = std::env::var_os("FOUNDRY_LINT_UI_TESTING").is_some();

        let sm = compiler.sess().clone_source_map();
        let prev_emitter = compiler.dcx().set_emitter(if self.with_json_emitter {
            let writer = Box::new(std::io::BufWriter::new(std::io::stderr()));
            let json_emitter = JsonEmitter::new(writer, sm).rustc_like(true).ui_testing(ui_testing);
            Box::new(json_emitter)
        } else {
            Box::new(HumanEmitter::stderr(Default::default()).source_map(Some(sm)))
        });
        let sess = compiler.sess_mut();
        sess.dcx.set_flags_mut(|f| f.track_diagnostics = false);
        if ui_testing {
            sess.opts.unstable.ui_testing = true;
            sess.reconfigure();
        }

        compiler.enter_mut(|compiler| -> eyre::Result<()> {
            if compiler.gcx().stage() == Some(solar::config::CompilerStage::Parsing) {
                let _ = compiler.lower_asts();
                convert_solar_errors(compiler.dcx())?;
            }

            let gcx = compiler.gcx();

            input.par_iter().for_each(|path| {
                let path = &self.path_config.root.join(path);
                let Some((_, ast_source)) = gcx.get_ast_source(path) else {
                    panic!("AST source not found for {}", path.display());
                };
                let Some(ast) = &ast_source.ast else {
                    panic!("AST missing for {}", path.display());
                };
                let _ = self.process_source_ast(gcx.sess, ast, &ast_source.file, path);

                let Some((hir_source_id, hir_source)) = gcx.get_hir_source(path) else {
                    panic!("HIR source not found for {}", path.display());
                };
                let _ = self.process_source_hir(gcx, hir_source_id, &hir_source.file);
            });

            Ok(())
        })?;

        let sess = compiler.sess_mut();
        sess.dcx.set_emitter(prev_emitter);
        if ui_testing {
            sess.opts.unstable.ui_testing = false;
            sess.reconfigure();
        }

        convert_solar_errors(compiler.dcx())
    }
}

enum InlineConfigSource<'ast, 'hir> {
    Ast(&'ast ast::SourceUnit<'ast>),
    Hir((&'hir hir::Hir<'hir>, hir::SourceId)),
}

fn parse_inline_config<'ast, 'hir>(
    sess: &Session,
    comments: &Comments,
    source: InlineConfigSource<'ast, 'hir>,
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
        match InlineConfigItem::parse(item, &ALL_REGISTERED_LINTS) {
            Ok(item) => Some((span, item)),
            Err(e) => {
                sess.dcx.warn(e.to_string()).span(span).emit();
                None
            }
        }
    });

    match source {
        InlineConfigSource::Ast(ast) => InlineConfig::from_ast(items, ast, sess.source_map()),
        InlineConfigSource::Hir((hir, id)) => {
            InlineConfig::from_hir(items, hir, id, sess.source_map())
        }
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

        for &lint in codesize::REGISTERED_LINTS {
            if lint.id() == value {
                return Ok(lint);
            }
        }

        Err(SolLintError::InvalidId(value.to_string()))
    }
}
