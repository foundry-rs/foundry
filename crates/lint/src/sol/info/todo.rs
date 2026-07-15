use super::TodoComment;
use crate::{
    linter::{EarlyLintPass, Lint, LintContext},
    sol::{Severity, SolLint},
};
use alloy_primitives::map::HashSet;
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
        _ctx: &LintContext<'ast, '_>,
        _ast: &'ast ast::SourceUnit<'ast>,
    ) {
        if !_ctx.is_lint_enabled(TODO_COMMENT.id()) {
            return;
        }

        let Some(file) = _ctx.source_file() else { return };

        // Build comments from the source, same call the crate already uses in mod.rs.
        let comments = Comments::new(file, _ctx.session().source_map(), false, false, None);

        for comment in comments.iter() {
            if is_control_comment(comment) {
                continue;
            }

            let mut seen = HashSet::new();
            let mut found = Vec::new();
            for line in &comment.lines {
                let mut allow_bare = true;
                for token in strip_comment_prefix(line, comment).split_whitespace() {
                    if let Some(marker) = marker_at_start(token, allow_bare)
                        && seen.insert(marker)
                    {
                        found.push(marker);
                    }

                    if token != "*" {
                        allow_bare = token.starts_with('@');
                    }
                }
            }

            if found.is_empty() {
                continue;
            }

            let markers = found.join(", ");
            let noun = if found.len() > 1 { "comments" } else { "comment" };
            _ctx.emit_with_msg(
                &TODO_COMMENT,
                comment.span,
                format!("unresolved `{markers}` {noun}"),
            );
        }
    }
}

fn is_control_comment(comment: &Comment) -> bool {
    let Some(first_line) = comment.lines.first() else { return false };
    let content = strip_comment_prefix(first_line, comment).trim_start();
    content.starts_with("@compile-flags:") || content.starts_with("forge-lint:")
}

/// If `token` begins with a marker followed by a valid boundary, return that marker.
fn marker_at_start(token: &str, allow_bare: bool) -> Option<&str> {
    MARKERS.iter().find_map(|m| {
        let prefix = token.get(..m.len())?;
        if !prefix.eq_ignore_ascii_case(m) {
            return None;
        }

        let suffix = &token[m.len()..];
        if suffix.is_empty() {
            return allow_bare.then_some(*m);
        }

        let mut trailing = suffix.chars();
        let after = trailing.next()?;
        if !TRAILING.contains(&after)
            || (after == '.' && trailing.next().is_some_and(|c| c.is_alphanumeric() || c == '_'))
        {
            return None;
        }
        Some(*m)
    })
}

fn strip_comment_prefix<'a>(line: &'a str, comment: &Comment) -> &'a str {
    comment.prefix().and_then(|p| line.strip_prefix(p)).unwrap_or(line)
}
