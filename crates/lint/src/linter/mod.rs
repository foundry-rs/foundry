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
            Snippet::Diff { desc, span: diff_span, add, trim_code } => {
                // Use the provided span or fall back to the lint span
                let target_span = diff_span.unwrap_or(span);

                // Check if we can get the original code
                if self.span_to_snippet(target_span).is_some() {
                    Snippet::Diff { desc, span: Some(target_span), add, trim_code }
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

    /// Gets the "raw" source code (snippet) of the given span.
    pub fn span_to_snippet(&self, span: Span) -> Option<String> {
        self.sess.source_map().span_to_snippet(span).ok()
    }

    /// Gets the number of leading whitespaces (indentation) of the line where the span begins.
    pub fn get_span_indentation(&self, span: Span) -> usize {
        if !span.is_dummy() {
            // Get the line text and compute the indentation prior to the span's position.
            let loc = self.sess.source_map().lookup_char_pos(span.lo());
            if let Some(line_text) = loc.file.get_line(loc.line) {
                let col_offset = loc.col.to_usize();
                if col_offset <= line_text.len() {
                    let prev_text = &line_text[..col_offset];
                    return prev_text.len() - prev_text.trim().len();
                }
            }
        }

        0
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Snippet {
    /// A standalone block of code. Used for showing examples without suggesting a fix.
    Block {
        /// An optional description displayed above the code block.
        desc: Option<&'static str>,
        /// The source code to display. Multi-line strings should include newlines.
        code: String,
    },

    /// A proposed code change, displayed as a diff. Used to suggest replacements, showing the code
    /// to be removed (from `span`) and the code to be added (from `add`).
    Diff {
        /// An optional description displayed above the diff.
        desc: Option<&'static str>,
        /// The `Span` of the source code to be removed. Note that, if uninformed,
        /// `fn emit_with_fix()` falls back to the lint span.
        span: Option<Span>,
        /// The fix.
        add: String,
        /// If `true`, the leading whitespaces of the first line will be trimmed from the whole
        /// code block. Applies to both, the added and removed code. This is useful for
        /// aligning the indentation of multi-line replacements.
        trim_code: bool,
    },
}

impl Snippet {
    pub fn to_note(self, ctx: &LintContext<'_>) -> Vec<(DiagMsg, Style)> {
        let mut output = if let Some(desc) = self.desc() {
            vec![(DiagMsg::from(desc), Style::NoStyle), (DiagMsg::from("\n\n"), Style::NoStyle)]
        } else {
            vec![(DiagMsg::from(" \n"), Style::NoStyle)]
        };

        match self {
            Self::Diff { span, add, trim_code: trim, .. } => {
                // Get the original code from the span if provided and normalize its indentation
                if let Some(span) = span
                    && let Some(rmv) = ctx.span_to_snippet(span)
                {
                    let ind = ctx.get_span_indentation(span);
                    let diag_msg = |line: &str, prefix: &str, style: Style| {
                        let content = if trim { Self::trim_start_limited(line, ind) } else { line };
                        (DiagMsg::from(format!("{prefix}{content}\n")), style)
                    };
                    output.extend(rmv.lines().map(|line| diag_msg(line, "- ", Style::Removal)));
                    output.extend(add.lines().map(|line| diag_msg(line, "+ ", Style::Addition)));
                } else {
                    // Should never happen, but fall back to `Self::Block` behavior.
                    output.extend(
                        add.lines()
                            .map(|line| (DiagMsg::from(format!("{line}\n")), Style::NoStyle)),
                    );
                }
            }
            Self::Block { code, .. } => {
                output.extend(
                    code.lines().map(|line| (DiagMsg::from(format!("{line}\n")), Style::NoStyle)),
                );
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

    /// Removes up to `max_chars` whitespaces from the start of the string.
    fn trim_start_limited(s: &str, max_chars: usize) -> &str {
        let (mut chars, mut byte_offset) = (0, 0);
        for c in s.chars() {
            if chars >= max_chars || !c.is_whitespace() {
                break;
            }
            chars += 1;
            byte_offset += c.len_utf8();
        }

        &s[byte_offset..]
    }
}
