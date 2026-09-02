use crate::{
    linter::{EarlyLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint, naming::check_pascal_case as check_pascal_case_pure},
};
use foundry_config::lint::LintSpecificConfig;
use solar::ast::ItemStruct;
use std::sync::Arc;

declare_forge_lint!(
    PASCAL_CASE_STRUCT,
    Severity::Info,
    "pascal-case-struct",
    "structs should use PascalCase"
);

#[derive(Debug)]
pub(super) struct PascalCaseStructPass {
    config: Arc<LintSpecificConfig>,
}

impl PascalCaseStructPass {
    pub(super) const fn new(config: Arc<LintSpecificConfig>) -> Self {
        Self { config }
    }
}

impl<'ast> EarlyLintPass<'ast> for PascalCaseStructPass {
    fn check_item_struct(&mut self, ctx: &LintContext, strukt: &'ast ItemStruct<'ast>) {
        let name = strukt.name.as_str();
        if let Some(expected) = check_pascal_case(name, &self.config.mixed_case_exceptions) {
            ctx.emit_with_suggestion(
                &PASCAL_CASE_STRUCT,
                strukt.name.span,
                Suggestion::fix(
                    expected,
                    solar::interface::diagnostics::Applicability::MachineApplicable,
                )
                .with_desc("consider using"),
            );
        }
    }
}

/// Wraps [`check_pascal_case_pure`] with the same configurable acronym
/// exceptions `mixed-case-*` uses (e.g. `ERC20Data`, `EIP712Domain`), so a
/// struct/enum name is not flagged just because it contains an allowed
/// all-caps acronym.
fn check_pascal_case(s: &str, allowed_patterns: &[String]) -> Option<String> {
    if s.len() <= 1 {
        return None;
    }

    for pattern in allowed_patterns {
        if let Some(pos) = s.find(pattern.as_str()) {
            let (pre, post) = s.split_at(pos);
            let post = &post[pattern.len()..];

            // Text before the pattern must already be valid PascalCase (or empty).
            let is_pre_valid = pre.is_empty() || pre == heck::AsUpperCamelCase(pre).to_string();

            // Text after the pattern must be valid PascalCase, allowing leading digits.
            let post_trimmed = post.trim_start_matches(|c: char| c.is_numeric());
            let is_post_valid = post_trimmed.is_empty()
                || post_trimmed == heck::AsUpperCamelCase(post_trimmed).to_string();

            if is_pre_valid && is_post_valid {
                return None;
            }
        }
    }

    check_pascal_case_pure(s)
}
