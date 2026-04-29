use super::Rtlo;
use crate::{
    linter::{EarlyLintPass, Lint, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    interface::{
        BytePos, Span,
        diagnostics::{DiagId, MultiSpan},
    },
};

declare_forge_lint!(
    RTLO,
    Severity::High,
    "rtlo",
    "unicode bidirectional override character can hide malicious code"
);

impl<'ast> EarlyLintPass<'ast> for Rtlo {
    fn check_full_source_unit(
        &mut self,
        ctx: &LintContext<'ast, '_>,
        unit: &'ast ast::SourceUnit<'ast>,
    ) {
        if !ctx.is_lint_enabled(RTLO.id()) {
            return;
        }

        let Some(first_item_span) = unit.items.first().map(|i| i.span) else {
            return;
        };
        let file = ctx.session().source_map().lookup_source_file(first_item_span.lo());

        for (offset, ch) in file.src.char_indices() {
            let Some(name) = bidi_char_name(ch) else { continue };

            let lo = file.start_pos + BytePos::from_usize(offset);
            let hi = lo + BytePos::from_usize(ch.len_utf8());
            let span = Span::new(lo, hi);

            // Replicate guards from `LintContext::emit` for inline-config suppression.
            if ctx.config.inline.is_id_disabled(span, RTLO.id()) || !ctx.is_lint_enabled(RTLO.id())
            {
                continue;
            }

            let msg = format!("U+{:04X} ({name}) detected", ch as u32);
            let diag = ctx
                .session()
                .dcx
                .diag(RTLO.severity().into(), msg)
                .code(DiagId::new_str(RTLO.id()))
                .span(MultiSpan::from_span(span));

            ctx.add_help(diag, RTLO.help()).emit();
        }
    }
}

const fn bidi_char_name(ch: char) -> Option<&'static str> {
    Some(match ch {
        '\u{200E}' => "Left-to-Right Mark",
        '\u{200F}' => "Right-to-Left Mark",
        '\u{202A}' => "Left-to-Right Embedding",
        '\u{202B}' => "Right-to-Left Embedding",
        '\u{202C}' => "Pop Directional Formatting",
        '\u{202D}' => "Left-to-Right Override",
        '\u{202E}' => "Right-to-Left Override",
        '\u{2066}' => "Left-to-Right Isolate",
        '\u{2067}' => "Right-to-Left Isolate",
        '\u{2068}' => "First Strong Isolate",
        '\u{2069}' => "Pop Directional Isolate",
        _ => return None,
    })
}
