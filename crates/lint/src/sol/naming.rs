//! Naming-convention helpers shared by Solidity lints.
//!
//! Each `check_*` returns `Some(suggestion)` when `s` violates the convention,
//! `None` when it already matches. Leading/trailing underscores are preserved.

use crate::{
    linter::{LintContext, Suggestion},
    sol::SolLint,
};
use solar::interface::{Span, diagnostics::Applicability};

/// `Some(suggestion)` if `s` is not `PascalCase`.
pub fn check_pascal_case(s: &str) -> Option<String> {
    suggest(s, heck::AsPascalCase(s).to_string())
}

/// `Some(suggestion)` if `s` is not `SCREAMING_SNAKE_CASE`.
pub fn check_screaming_snake_case(s: &str) -> Option<String> {
    suggest(s, preserve_underscores(s, heck::AsShoutySnakeCase(s).to_string()))
}

/// `Some(suggestion)` if `s` is not `mixedCase`. Pure check — domain
/// exceptions (test-prefixes, allowed patterns, ...) live in the lint.
pub fn check_mixed_case(s: &str) -> Option<String> {
    suggest(s, preserve_underscores(s, heck::AsLowerCamelCase(s).to_string()))
}

/// Emits `lint` at `span` with a machine-applicable rename to `expected`.
pub fn emit_rename(ctx: &LintContext, lint: &'static SolLint, span: Span, expected: String) {
    let suggestion =
        Suggestion::fix(expected, Applicability::MachineApplicable).with_desc("consider using");
    ctx.emit_with_suggestion(lint, span, suggestion);
}

/// Single-character names are exempt from every convention.
fn suggest(s: &str, expected: String) -> Option<String> {
    (s.len() > 1 && s != expected).then_some(expected)
}

fn preserve_underscores(s: &str, body: String) -> String {
    let prefix = if s.starts_with('_') { "_" } else { "" };
    let suffix = if s.ends_with('_') { "_" } else { "" };
    format!("{prefix}{body}{suffix}")
}
