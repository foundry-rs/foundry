use super::TodoComment;
use crate::{
    linter::{EarlyLintPass, Lint, LintContext},
    sol::{Severity, SolLint},
};
use foundry_common::comments::{Comment, Comments};
use solar::ast;

declare_forge_lint!(
    TODO_COMMENT,
    Severity::Info,
    "todo-comment",
    "TODO/FIXME comments should be resolved before production"
);

const MARKERS: &[&str] = &["TODO", "FIXME"];

/// Characters that may directly follow a marker and still count as a real marker.
const TRAILING: &[char] = &[':', '(', ',', ';', '.', ')'];

impl<'ast> EarlyLintPass<'ast> for TodoComment {
    fn check_full_source_unit(
        &mut self,
        ctx: &LintContext<'ast, '_>,
        _ast: &'ast ast::SourceUnit<'ast>,
    ) {
        if !ctx.is_lint_enabled(TODO_COMMENT.id()) {
            return;
        }
        let Some(file) = ctx.source_file() else { return };
        let comments = Comments::new(file, ctx.session().source_map(), false, false, None);
        for comment in comments.iter().filter(|comment| !is_control_comment(comment)) {
            let mut found = Vec::new();
            // Unnormalized block comments are stored as one string, so split physical lines here.
            for line in comment.lines.iter().flat_map(|line| line.lines()) {
                // A bare marker only counts at the start of a line or right after a NatSpec tag.
                let mut allow_bare = true;
                for token in strip_comment_prefix(line, comment).split_whitespace() {
                    if let Some(marker) = marker_at_start(token, allow_bare)
                        && !found.contains(&marker)
                    {
                        found.push(marker);
                    }
                    if token != "*" {
                        allow_bare = token.starts_with('@');
                    }
                }
            }
            if !found.is_empty() {
                let noun = if found.len() > 1 { "comments" } else { "comment" };
                let msg = format!("unresolved `{}` {noun}", found.join(", "));
                ctx.emit_with_msg(&TODO_COMMENT, comment.span, msg);
            }
        }
    }
}

fn is_control_comment(comment: &Comment) -> bool {
    comment.lines.first().is_some_and(|first_line| {
        let content = strip_comment_prefix(first_line, comment).trim_start();
        content.starts_with("@compile-flags:") || content.starts_with("forge-lint:")
    })
}

/// If `token` begins with a marker followed by a valid boundary, return that marker.
fn marker_at_start(token: &str, allow_bare: bool) -> Option<&str> {
    MARKERS.iter().copied().find(|m| {
        let Some((prefix, suffix)) = token.split_at_checked(m.len()) else { return false };
        if !prefix.eq_ignore_ascii_case(m) {
            return false;
        }
        let mut trailing = suffix.chars();
        match trailing.next() {
            None => allow_bare,
            // A `.` must end the marker, not start an identifier (`TODO.md`).
            Some('.') => !trailing.next().is_some_and(|c| c.is_alphanumeric() || c == '_'),
            Some(after) => TRAILING.contains(&after),
        }
    })
}

fn strip_comment_prefix<'a>(line: &'a str, comment: &Comment) -> &'a str {
    comment.prefix().and_then(|p| line.strip_prefix(p)).unwrap_or(line)
}
