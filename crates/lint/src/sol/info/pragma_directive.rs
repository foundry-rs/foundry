use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint, info::PragmaDirective},
};
use solar::{ast, interface::Span};

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

        // Collect every `pragma solidity` directive across input sources, with its rendered
        // version-requirement string for grouping. Stores source index to avoid lifetime
        // invariance issues with `&ProjectSource<'ast>`.
        let mut entries: Vec<(usize, Span, String)> = Vec::new();
        for (idx, source) in sources.iter().enumerate() {
            for (span, req) in solidity_pragmas(source.ast) {
                entries.push((idx, span, req.to_string()));
            }
        }

        // Stable order for snapshots and JSON output.
        entries.sort_by(|a, b| {
            sources[a.0].path.cmp(&sources[b.0].path).then(a.1.lo().cmp(&b.1.lo()))
        });

        // Build the distinct list once and bail if all sources agree.
        let mut distinct: Vec<&str> = entries.iter().map(|(_, _, s)| s.as_str()).collect();
        distinct.sort_unstable();
        distinct.dedup();
        if distinct.len() < 2 {
            return;
        }

        for (idx, span, req_str) in &entries {
            let others = distinct
                .iter()
                .filter(|v| **v != req_str.as_str())
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            let msg = format!(
                "'pragma solidity {req_str};' conflicts with other version requirements in the project: {others}"
            );
            ctx.emit_with_msg(&sources[*idx], &PRAGMA_INCONSISTENT, *span, msg);
        }
    }
}

/// Yields every top-level `pragma solidity ...;` directive in `unit`.
fn solidity_pragmas<'ast>(
    unit: &'ast ast::SourceUnit<'ast>,
) -> impl Iterator<Item = (Span, &'ast ast::SemverReq<'ast>)> + 'ast {
    unit.items.iter().filter_map(|item| match &item.kind {
        ast::ItemKind::Pragma(p) => match &p.tokens {
            ast::PragmaTokens::Version(ident, req) if ident.as_str() == "solidity" => {
                Some((item.span, req))
            }
            _ => None,
        },
        _ => None,
    })
}
