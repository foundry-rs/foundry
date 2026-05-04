use super::{Lint, LintContext, LinterConfig};
use foundry_common::comments::inline_config::InlineConfig;
use foundry_config::lint::LintSpecificConfig;
use solar::{
    ast,
    interface::{Session, Span, diagnostics::DiagMsg, source_map::SourceFile},
};
use std::{path::PathBuf, sync::Arc};

/// A single source unit visible to a project-wide lint pass, pre-loaded with its inline config so
/// emits respect `// forge-lint: disable-*` markers without rebuilding it per emit.
pub struct ProjectSource<'ast> {
    pub path: PathBuf,
    pub file: Arc<SourceFile>,
    pub ast: &'ast ast::SourceUnit<'ast>,
    pub inline_config: InlineConfig<Vec<String>>,
}

/// Trait for lints that need to inspect every input source at once (e.g. cross-file checks).
///
/// `check_project` runs once after all per-file [`super::EarlyLintPass`] /
/// [`super::LateLintPass`] passes have completed.
pub trait ProjectLintPass<'ast>: Send + Sync {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]);
}

/// Helper passed to [`ProjectLintPass::check_project`] for emitting diagnostics against a specific
/// source.
pub struct ProjectLintEmitter<'s, 'c> {
    sess: &'s Session,
    with_description: bool,
    with_json_emitter: bool,
    lint_specific: &'c LintSpecificConfig,
    active_lints: Vec<&'static str>,
}

impl<'s, 'c> ProjectLintEmitter<'s, 'c> {
    pub const fn new(
        sess: &'s Session,
        with_description: bool,
        with_json_emitter: bool,
        lint_specific: &'c LintSpecificConfig,
        active_lints: Vec<&'static str>,
    ) -> Self {
        Self { sess, with_description, with_json_emitter, lint_specific, active_lints }
    }

    /// Returns `true` if the given lint id is enabled for this run. Project passes that perform
    /// expensive analysis should guard their work behind this check.
    pub fn is_lint_enabled(&self, id: &'static str) -> bool {
        self.active_lints.contains(&id)
    }

    /// Emits a diagnostic with the lint's default description as the message.
    pub fn emit<'a, 'ast, L: Lint>(
        &'a self,
        source: &'a ProjectSource<'ast>,
        lint: &'static L,
        span: Span,
    ) where
        'c: 'a,
    {
        self.build_ctx(source).emit(lint, span);
    }

    /// Emits a diagnostic with a caller-provided message.
    pub fn emit_with_msg<'a, 'ast, L: Lint>(
        &'a self,
        source: &'a ProjectSource<'ast>,
        lint: &'static L,
        span: Span,
        msg: impl Into<DiagMsg>,
    ) where
        'c: 'a,
    {
        self.build_ctx(source).emit_with_msg(lint, span, msg);
    }

    fn build_ctx<'a, 'ast>(&'a self, source: &'a ProjectSource<'ast>) -> LintContext<'s, 'a>
    where
        'c: 'a,
    {
        LintContext::new(
            self.sess,
            self.with_description,
            self.with_json_emitter,
            LinterConfig { inline: &source.inline_config, lint_specific: self.lint_specific },
            self.active_lints.clone(),
            Some(source.file.clone()),
        )
    }
}
