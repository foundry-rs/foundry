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
            for marker in MARKERS {
                if comment.lines.iter().any(|line| line.contains(marker)) {
                    _ctx.emit_with_msg(
                        &TODO,
                        comment.span,
                        format!("unresolved `{marker}` comment"),
                    );
                    break;
                }
            }
        }
    }
}
