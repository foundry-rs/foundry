use std::collections::HashSet;

use super::Todo;
use crate::{
    linter::{EarlyLintPass, Lint, LintContext},
    sol::{Severity, SolLint},
};
use foundry_common::comments::Comments;
use solar::ast;

declare_forge_lint!(
    TODO,
    Severity::Info,
    "todo",
    "TODO/FIXME comments should be resolved before production"
);

const MARKERS: &[&str] = &["TODO", "FIXME"];

impl<'ast> EarlyLintPass<'ast> for Todo {
    fn check_full_source_unit(
        &mut self,
        _ctx: &LintContext<'ast, '_>,
        _ast: &'ast ast::SourceUnit<'ast>,
    ) {
        if !_ctx.is_lint_enabled(TODO.id()) {
            return;
        }

        let Some(file) = _ctx.source_file() else { return };

        // Build comments from the source, same call the crate already uses in mod.rs.
        let comments = Comments::new(file, _ctx.session().source_map(), false, false, None);

        for comment in comments.iter() {
            let mut seen = HashSet::new();

            let found: Vec<&str> = comment
                .lines
                .iter()
                .flat_map(|line| line.split(|c: char| !c.is_alphanumeric()))
                .filter(|&word| MARKERS.iter().any(|m| word.eq_ignore_ascii_case(m)))
                .filter(|&word| seen.insert(word))
                .collect();

            if found.is_empty() {
                continue;
            }

            let markers = found.iter().map(|s| s.to_string()).collect::<Vec<String>>().join(", ");
            let noun = if found.len() > 1 { "comments" } else { "comment" };
            _ctx.emit_with_msg(&TODO, comment.span, format!("unresolved `{markers}` {noun}"));
        }
    }
}
