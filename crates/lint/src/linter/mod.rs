mod early;
mod late;

pub use early::{EarlyLintPass, EarlyLintVisitor};
pub use late::{LateLintPass, LateLintVisitor};

use foundry_common::comments::inline_config::InlineConfig;
use foundry_compilers::Language;
use foundry_config::{DenyLevel, lint::Severity};
use solar::{
    interface::{
        Session, Span,
        diagnostics::{
            Applicability, DiagBuilder, DiagId, DiagMsg, MultiSpan, Style, SuggestionStyle,
        },
    },
    sema::Compiler,
};
use std::path::PathBuf;

/// Trait representing a generic linter for analyzing and reporting issues in smart contract source
/// code files.
///
/// A linter can be implemented for any smart contract language supported by Foundry.
pub trait Linter: Send + Sync {
    /// The target [`Language`].
    type Language: Language;
    /// The [`Lint`] type.
    type Lint: Lint;

    /// Run all lints.
    ///
    /// The `compiler` should have already been configured with all the sources necessary,
    /// as well as having performed parsing and lowering.
    ///
    /// Should return an error based on the configured [`DenyLevel`] and the emitted diagnostics.
    fn lint(&self, input: &[PathBuf], deny: DenyLevel, compiler: &mut Compiler)
    -> eyre::Result<()>;
}

pub trait Lint {
    fn id(&self) -> &'static str;
    fn severity(&self) -> Severity;
    fn description(&self) -> &'static str;
    fn help(&self) -> &'static str;
}

pub struct LintContext<'s, 'c> {
    sess: &'s Session,
    with_description: bool,
    with_json_emitter: bool,
    pub config: LinterConfig<'c>,
    active_lints: Vec<&'static str>,
}

pub struct LinterConfig<'s> {
    pub inline: &'s InlineConfig<Vec<String>>,
    pub mixed_case_exceptions: &'s [String],
}

impl<'s, 'c> LintContext<'s, 'c> {
    pub fn new(
        sess: &'s Session,
        with_description: bool,
        with_json_emitter: bool,
        config: LinterConfig<'c>,
        active_lints: Vec<&'static str>,
    ) -> Self {
        Self { sess, with_description, with_json_emitter, config, active_lints }
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
        if self.config.inline.is_id_disabled(span, lint.id()) || !self.is_lint_enabled(lint.id()) {
            return;
        }

        let desc = if self.with_description { lint.description() } else { "" };
        let mut diag: DiagBuilder<'_, ()> = self
            .sess
            .dcx
            .diag(lint.severity().into(), desc)
            .code(DiagId::new_str(lint.id()))
            .span(MultiSpan::from_span(span));

        // Avoid ANSI characters when using a JSON emitter
        if self.with_json_emitter {
            diag = diag.help(lint.help());
        } else {
            diag = diag.help(hyperlink(lint.help()));
        }

        diag.emit();
    }

    /// Emit a diagnostic with a code suggestion.
    ///
    /// If no span is provided for [`SuggestionKind::Fix`], it will use the lint's span.
    pub fn emit_with_suggestion<L: Lint>(
        &self,
        lint: &'static L,
        span: Span,
        suggestion: Suggestion,
    ) {
        if self.config.inline.is_id_disabled(span, lint.id()) || !self.is_lint_enabled(lint.id()) {
            return;
        }

        let desc = if self.with_description { lint.description() } else { "" };
        let mut diag: DiagBuilder<'_, ()> = self
            .sess
            .dcx
            .diag(lint.severity().into(), desc)
            .code(DiagId::new_str(lint.id()))
            .span(MultiSpan::from_span(span));

        diag = match suggestion.kind {
            SuggestionKind::Fix { span: fix_span, applicability, style } => diag
                .span_suggestion_with_style(
                    fix_span.unwrap_or(span),
                    suggestion.desc.unwrap_or_default(),
                    suggestion.content,
                    applicability,
                    style,
                ),
            SuggestionKind::Example => {
                if let Some(note) = suggestion.to_note() {
                    diag.note(note.iter().map(|l| l.0.as_str()).collect::<String>())
                } else {
                    diag
                }
            }
        };

        // Avoid ANSI characters when using a JSON emitter
        if self.with_json_emitter {
            diag = diag.help(lint.help());
        } else {
            diag = diag.help(hyperlink(lint.help()));
        }

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
pub enum SuggestionKind {
    /// A standalone block of code. Used for showing examples without suggesting a fix.
    ///
    /// Multi-line strings should include newlines.
    Example,

    /// A proposed code change, displayed as a diff. Used to suggest replacements, showing the code
    /// to be removed (from `span`) and the code to be added (from `add`).
    Fix {
        /// The `Span` of the source code to be removed. Note that, if uninformed,
        /// `fn emit_with_fix()` falls back to the lint span.
        span: Option<Span>,
        /// The applicability of the suggested fix.
        applicability: Applicability,
        /// The style of the suggested fix.
        style: SuggestionStyle,
    },
}

// An emittable diagnostic suggestion.
//
// Depending on its [`SuggestionKind`] will be emitted as a simple note (examples), or a fix
// suggestion.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Suggestion {
    /// An optional description displayed above the code block.
    desc: Option<&'static str>,
    /// The actual suggestion.
    content: String,
    /// The suggestion type and its specific data.
    kind: SuggestionKind,
}

impl Suggestion {
    /// Creates a new [`SuggestionKind::Example`] suggestion.
    pub fn example(content: String) -> Self {
        Self { desc: None, content, kind: SuggestionKind::Example }
    }

    /// Creates a new [`SuggestionKind::Fix`] suggestion.
    ///
    /// When possible, will attempt to inline the suggestion.
    pub fn fix(content: String, applicability: Applicability) -> Self {
        Self {
            desc: None,
            content,
            kind: SuggestionKind::Fix {
                span: None,
                applicability,
                style: SuggestionStyle::ShowCode,
            },
        }
    }

    /// Sets the description for the suggestion.
    pub fn with_desc(mut self, desc: &'static str) -> Self {
        self.desc = Some(desc);
        self
    }

    /// Sets the span for a [`SuggestionKind::Fix`] suggestion.
    pub fn with_span(mut self, span: Span) -> Self {
        if let SuggestionKind::Fix { span: ref mut s, .. } = self.kind {
            *s = Some(span);
        }
        self
    }

    /// Sets the style for a [`SuggestionKind::Fix`] suggestion.
    pub fn with_style(mut self, style: SuggestionStyle) -> Self {
        if let SuggestionKind::Fix { style: ref mut s, .. } = self.kind {
            *s = style;
        }
        self
    }

    fn to_note(&self) -> Option<Vec<(DiagMsg, Style)>> {
        if let SuggestionKind::Fix { .. } = &self.kind {
            return None;
        };

        let mut output = if let Some(desc) = self.desc {
            vec![(DiagMsg::from(desc), Style::NoStyle), (DiagMsg::from("\n\n"), Style::NoStyle)]
        } else {
            vec![(DiagMsg::from(" \n"), Style::NoStyle)]
        };

        output.extend(
            self.content.lines().map(|line| (DiagMsg::from(format!("{line}\n")), Style::NoStyle)),
        );
        output.push((DiagMsg::from("\n"), Style::NoStyle));
        Some(output)
    }
}

/// Creates a hyperlink of the input url.
fn hyperlink(url: &'static str) -> String {
    format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")
}
