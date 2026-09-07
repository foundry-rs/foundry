use super::PragmaDirective;
use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint},
};
use solar::ast;

declare_forge_lint!(
    PRAGMA_INCONSISTENT,
    Severity::Info,
    "pragma-inconsistent",
    "inconsistent Solidity pragma version requirements across the project"
);

impl<'ast> ProjectLintPass<'ast> for PragmaDirective {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]) {
        if !ctx.is_lint_enabled(PRAGMA_INCONSISTENT.id()) {
            return;
        }
        // Every `pragma solidity` directive across input sources, with its rendered version
        // requirement for grouping, in a stable (path, position) order for snapshots.
        let mut entries: Vec<(usize, _, String)> = sources
            .iter()
            .enumerate()
            .flat_map(|(idx, source)| {
                source.ast.items.iter().filter_map(move |item| match &item.kind {
                    ast::ItemKind::Pragma(ast::PragmaDirective {
                        tokens: ast::PragmaTokens::Version(ident, req),
                        ..
                    }) if ident.as_str() == "solidity" => Some((idx, item.span, req.to_string())),
                    _ => None,
                })
            })
            .collect();
        entries.sort_by(|a, b| {
            sources[a.0].path.cmp(&sources[b.0].path).then(a.1.lo().cmp(&b.1.lo()))
        });

        let mut distinct: Vec<&str> = entries.iter().map(|(_, _, req)| req.as_str()).collect();
        distinct.sort_unstable();
        distinct.dedup();
        if let [(idx, span, _), ..] = entries.as_slice()
            && distinct.len() > 1
        {
            let msg = format!(
                "{} different Solidity pragma version requirements are used: {}",
                distinct.len(),
                distinct.join(", ")
            );
            ctx.emit_with_msg(&sources[*idx], &PRAGMA_INCONSISTENT, *span, msg);
        }
    }
}
