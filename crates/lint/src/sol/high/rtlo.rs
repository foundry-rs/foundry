use super::Rtlo;
use crate::{
    linter::{EarlyLintPass, Lint, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    interface::{BytePos, Span},
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
        _unit: &'ast ast::SourceUnit<'ast>,
    ) {
        if !ctx.is_lint_enabled(RTLO.id()) {
            return;
        }

        // Scan the raw source so bidi chars in comments are also caught.
        let Some(file) = ctx.source_file() else { return };

        for (offset, ch) in file.src.char_indices() {
            let Some(name) = bidi_char_name(ch) else { continue };

            let lo = file.start_pos + BytePos::from_usize(offset);
            let hi = lo + BytePos::from_usize(ch.len_utf8());
            let span = Span::new(lo, hi);

            ctx.emit_with_msg(&RTLO, span, format!("U+{:04X} ({name}) detected", ch as u32));
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
