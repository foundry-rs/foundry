use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint, info::ContractNameReused},
};
use solar::{ast::ItemKind, interface::Span};
use std::collections::BTreeMap;

declare_forge_lint!(
    CONTRACT_NAME_REUSED,
    Severity::Info,
    "name-reused",
    "contract name is reused across multiple source files"
);

impl<'ast> ProjectLintPass<'ast> for ContractNameReused {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]) {
        if !ctx.is_lint_enabled(CONTRACT_NAME_REUSED.id()) {
            return;
        }

        // Group every contract-like declaration (contract, interface, library, abstract) in the
        // project by name. Stores source indices to avoid lifetime invariance issues with
        // `&ProjectSource<'ast>`. `BTreeMap` gives stable iteration order over names.
        let mut by_name: BTreeMap<&str, Vec<(usize, Span)>> = BTreeMap::new();
        for (idx, source) in sources.iter().enumerate() {
            for item in source.ast.items.iter() {
                if let ItemKind::Contract(c) = &item.kind {
                    by_name.entry(c.name.as_str()).or_default().push((idx, c.name.span));
                }
            }
        }

        for (name, mut occurrences) in by_name {
            // Sort by (path, span) so diagnostics — and the "also defined in" lists — are stable
            // regardless of input ordering.
            occurrences.sort_by(|a, b| {
                sources[a.0].path.cmp(&sources[b.0].path).then(a.1.lo().cmp(&b.1.lo()))
            });

            // Distinct files in which the name appears. If only one file defines it, there is no
            // cross-file collision to report.
            let mut files: Vec<usize> = occurrences.iter().map(|(idx, _)| *idx).collect();
            files.dedup();
            if files.len() < 2 {
                continue;
            }

            for (idx, span) in &occurrences {
                let others = files
                    .iter()
                    .filter(|&&j| j != *idx)
                    .map(|&j| {
                        sources[j]
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| sources[j].path.display().to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let msg = format!("contract name `{name}` is also defined in: {others}");
                ctx.emit_with_msg(&sources[*idx], &CONTRACT_NAME_REUSED, *span, msg);
            }
        }
    }
}
