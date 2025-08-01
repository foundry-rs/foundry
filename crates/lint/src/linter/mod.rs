mod early;
mod late;

pub use early::{EarlyLintPass, EarlyLintVisitor};
pub use late::{LateLintPass, LateLintVisitor};

use foundry_compilers::Language;
use foundry_config::lint::Severity;
use solar_interface::{
    Session, Span,
    diagnostics::{DiagBuilder, DiagId, DiagMsg, MultiSpan, Style},
};
use solar_sema::ParsingContext;
use std::path::PathBuf;

use crate::inline_config::InlineConfig;

/// Trait representing a generic linter for analyzing and reporting issues in smart contract source
/// code files. A linter can be implemented for any smart contract language supported by Foundry.
///
/// # Type Parameters
///
/// - `Language`: Represents the target programming language. Must implement the [`Language`] trait.
/// - `Lint`: Represents the types of lints performed by the linter. Must implement the [`Lint`]
///   trait.
///
/// # Required Methods
///
/// - `init`: Creates a new solar `Session` with the appropriate linter configuration.
/// - `early_lint`: Scans the source files (using the AST) emitting a diagnostic for lints found.
/// - `late_lint`: Scans the source files (using the HIR) emitting a diagnostic for lints found.
///
/// # Note:
///
/// - For `early_lint` and `late_lint`, the `ParsingContext` should have the sources pre-loaded.
pub trait Linter: Send + Sync + Clone {
    type Language: Language;
    type Lint: Lint;

    fn init(&self) -> Session;
    fn early_lint<'sess>(&self, input: &[PathBuf], pcx: ParsingContext<'sess>);
    fn late_lint<'sess>(&self, input: &[PathBuf], pcx: ParsingContext<'sess>);
}

pub trait Lint {
    fn id(&self) -> &'static str;
    fn severity(&self) -> Severity;
    fn description(&self) -> &'static str;
    fn help(&self) -> &'static str;
}

pub struct LintContext<'s> {
    sess: &'s Session,
    with_description: bool,
    pub inline_config: InlineConfig,
    active_lints: Vec<&'static str>,
}

impl<'s> LintContext<'s> {
    pub fn new(
        sess: &'s Session,
        with_description: bool,
        config: InlineConfig,
        active_lints: Vec<&'static str>,
    ) -> Self {
        Self { sess, with_description, inline_config: config, active_lints }
    }

    pub fn session(&self) -> &'s Session {
        self.sess
    }

    // Helper method to check if a lint id is enabled.
    //
    // For performance reasons, some passes check several lints at once. Thus, this method is
    // required to avoid unintended warnings.
    pub fn is_lint_enabled(&self, id: &'static str) -> bool {
        self.active_lints.contains(&id)
    }

    /// Helper method to emit diagnostics easily from passes
    pub fn emit<L: Lint>(&self, lint: &'static L, span: Span) {
        if self.inline_config.is_disabled(span, lint.id()) || !self.is_lint_enabled(lint.id()) {
            return;
        }

        let desc = if self.with_description { lint.description() } else { "" };
        let diag: DiagBuilder<'_, ()> = self
            .sess
            .dcx
            .diag(lint.severity().into(), desc)
            .code(DiagId::new_str(lint.id()))
            .span(MultiSpan::from_span(span))
            .help(lint.help());

        diag.emit();
    }

    /// Emit a diagnostic with a code fix proposal.
    ///
    /// For Diff snippets, if no span is provided, it will use the lint's span.
    /// If unable to get code from the span, it will fall back to a Block snippet.
    pub fn emit_with_fix<L: Lint>(&self, lint: &'static L, span: Span, snippet: Snippet) {
        if self.inline_config.is_disabled(span, lint.id()) || !self.is_lint_enabled(lint.id()) {
            return;
        }

        // Convert the snippet to ensure we have the appropriate type
        let snippet = match snippet {
            Snippet::Diff { desc, span: diff_span, add } => {
                // Use the provided span or fall back to the lint span
                let target_span = diff_span.unwrap_or(span);

                // Check if we can get the original code
                if self.span_to_snippet(target_span).is_some() {
                    Snippet::Diff { desc, span: Some(target_span), add }
                } else {
                    // Fall back to Block if we can't get the original code
                    Snippet::Block { desc, code: add }
                }
            }
            // Block snippets remain unchanged
            block => block,
        };

        let desc = if self.with_description { lint.description() } else { "" };
        let diag: DiagBuilder<'_, ()> = self
            .sess
            .dcx
            .diag(lint.severity().into(), desc)
            .code(DiagId::new_str(lint.id()))
            .span(MultiSpan::from_span(span))
            .highlighted_note(snippet.to_note(self))
            .help(lint.help());

        diag.emit();
    }

    pub fn span_to_snippet(&self, span: Span) -> Option<String> {
        self.sess.source_map().span_to_snippet(span).ok()
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Snippet {
    /// Represents a code block. Can have an optional description.
    Block { desc: Option<&'static str>, code: String },
    /// Represents a code diff. Can have an optional description and a span for the code to remove.
    Diff { desc: Option<&'static str>, span: Option<Span>, add: String },
}

impl Snippet {
    pub fn to_note(self, ctx: &LintContext<'_>) -> Vec<(DiagMsg, Style)> {
        let mut output = Vec::new();
        match self.desc() {
            Some(desc) => {
                output.push((DiagMsg::from(desc), Style::NoStyle));
                output.push((DiagMsg::from("\n\n"), Style::NoStyle));
            }
            None => output.push((DiagMsg::from(" \n"), Style::NoStyle)),
        }
        match self {
            Self::Diff { span, add, .. } => {
                // Get the original code from the span if provided
                if let Some(span) = span
                    && let Some(rmv) = ctx.span_to_snippet(span)
                {
                    for line in rmv.lines() {
                        output.push((DiagMsg::from(format!("- {line}\n")), Style::Removal));
                    }
                }
                for line in add.lines() {
                    output.push((DiagMsg::from(format!("+ {line}\n")), Style::Addition));
                }
            }
            Self::Block { code, .. } => {
                for line in code.lines() {
                    output.push((DiagMsg::from(format!("- {line}\n")), Style::NoStyle));
                }
            }
        }
        output.push((DiagMsg::from("\n"), Style::NoStyle));
        output
    }

    pub fn desc(&self) -> Option<&'static str> {
        match self {
            Self::Diff { desc, .. } => *desc,
            Self::Block { desc, .. } => *desc,
        }
    }
}
