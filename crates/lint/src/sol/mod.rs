use crate::{
    inline_config::{InlineConfig, InlineConfigItem},
    linter::{
        EarlyLintPass, EarlyLintVisitor, LateLintPass, LateLintVisitor, Lint, LintContext, Linter,
    },
};
use foundry_common::comments::Comments;
use foundry_compilers::{ProjectPathsConfig, solc::SolcLanguage};
use foundry_config::lint::Severity;
use rayon::iter::{ParallelBridge, ParallelIterator};
use solar_ast::{self as ast, visit::Visit as VisitAST};
use solar_interface::{
    Session, SourceMap,
    diagnostics::{self, DiagCtxt, JsonEmitter},
    source_map::{FileName, SourceFile},
};
use solar_sema::{
    ParsingContext,
    hir::{self, Visit as VisitHIR},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
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

    fn include_lint(&self, lint: SolLint) -> bool {
        self.severity.as_ref().is_none_or(|sev| sev.contains(&lint.severity()))
            && self.lints_included.as_ref().is_none_or(|incl| incl.contains(&lint))
            && !self.lints_excluded.as_ref().is_some_and(|excl| excl.contains(&lint))
    }

    fn process_source_ast<'ast>(
        &self,
        sess: &'ast Session,
        ast: &'ast ast::SourceUnit<'ast>,
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
        let ctx = LintContext::new(sess, self.with_description, inline_config, lints);
        let mut early_visitor = EarlyLintVisitor::new(&ctx, &mut passes);
        _ = early_visitor.visit_source_unit(ast);
        early_visitor.post_source_unit(ast);

        Ok(())
    }

    fn process_source_hir<'hir>(
        &self,
        sess: &Session,
        gcx: &solar_sema::ty::Gcx<'hir>,
        source_id: hir::SourceId,
        file: &'hir SourceFile,
    ) -> Result<(), diagnostics::ErrorGuaranteed> {
        // Declare all available passes and lints
        let mut passes_and_lints = Vec::new();
        passes_and_lints.extend(high::create_late_lint_passes());
        passes_and_lints.extend(med::create_late_lint_passes());
        passes_and_lints.extend(info::create_late_lint_passes());

        // Do not apply 'gas' and 'codesize' severity rules on tests and scripts
        if let FileName::Real(ref path) = file.name
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
        let inline_config =
            parse_inline_config(sess, &comments, InlineConfigSource::Hir((&gcx.hir, source_id)));

        // Run late lint visitor
        let ctx = LintContext::new(sess, self.with_description, inline_config, lints);
        let mut late_visitor = LateLintVisitor::new(&ctx, &mut passes, &gcx.hir);

        // Visit this specific source
        _ = late_visitor.visit_nested_source(source_id);

        Ok(())
    }
}

impl Linter for SolidityLinter {
    type Language = SolcLanguage;
    type Lint = SolLint;

    /// Build solar session based on the linter config
    fn init(&self) -> Session {
        let mut builder = Session::builder();
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
        sess
    }

    /// Run AST-based lints.
    ///
    /// Note: the `ParsingContext` should already have the sources loaded.
    fn early_lint<'sess>(&self, input: &[PathBuf], pcx: ParsingContext<'sess>) {
        let sess = pcx.sess;
        _ = sess.enter_parallel(|| -> Result<(), diagnostics::ErrorGuaranteed> {
            // Parse the sources
            let ast_arena = solar_sema::thread_local::ThreadLocal::new();
            let ast_result = pcx.parse(&ast_arena);

            // Process each source in parallel
            ast_result.sources.iter().par_bridge().for_each(|source| {
                if let (FileName::Real(path), Some(ast)) = (&source.file.name, &source.ast)
                    && input.iter().any(|input_path| path.ends_with(input_path))
                {
                    _ = self.process_source_ast(sess, ast, &source.file, path)
                }
            });

            Ok(())
        });
    }

    /// Run HIR-based lints.
    ///
    /// Note: the `ParsingContext` should already have the sources loaded.
    fn late_lint<'sess>(&self, input: &[PathBuf], pcx: ParsingContext<'sess>) {
        let sess = pcx.sess;
        _ = sess.enter_parallel(|| -> Result<(), diagnostics::ErrorGuaranteed> {
            // Parse and lower to HIR
            let hir_arena = solar_sema::thread_local::ThreadLocal::new();
            let hir_result = pcx.parse_and_lower(&hir_arena);

            if let Ok(Some(gcx_wrapper)) = hir_result {
                let gcx = gcx_wrapper.get();

                // Process each source in parallel
                gcx.hir.sources_enumerated().par_bridge().for_each(|(source_id, source)| {
                    if let FileName::Real(ref path) = source.file.name
                        && input.iter().any(|input_path| path.ends_with(input_path))
                    {
                        _ = self.process_source_hir(sess, &gcx, source_id, &source.file);
                    }
                });
            }

            Ok(())
        });
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
