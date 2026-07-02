use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint, info::UnusedError},
};
use solar::{
    ast::ContractKind,
    interface::{data_structures::Never, source_map::FileName},
    sema::hir::{self, Visit as _},
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

declare_forge_lint!(UNUSED_ERROR, Severity::Info, "unused-error", "custom error is never used");

impl<'ast> ProjectLintPass<'ast> for UnusedError {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]) {
        if !ctx.is_lint_enabled(UNUSED_ERROR.id()) {
            return;
        }

        let gcx = ctx.gcx();
        let hir = &gcx.hir;

        // Map every input source's HIR `SourceId` to the corresponding `ProjectSource` index, so
        // only errors declared in user-provided files are reported. Uses are still collected
        // across the whole unit, so an error declared here and reverted in a dependency (or the
        // other way around) is attributed correctly.
        let input_source_idx: HashMap<hir::SourceId, usize> = hir
            .sources_enumerated()
            .filter_map(|(sid, src)| {
                let path = match &src.file.name {
                    FileName::Real(p) => p,
                    _ => return None,
                };
                let idx = sources.iter().position(|s| &s.path == path)?;
                Some((sid, idx))
            })
            .collect();

        if input_source_idx.is_empty() {
            return;
        }

        let used = collect_used_errors(hir);

        // Report input-file error declarations that no expression in the unit references.
        for error_id in hir.error_ids() {
            let error = hir.error(error_id);
            let Some(&src_idx) = input_source_idx.get(&error.source) else { continue };
            // Errors declared in interfaces and abstract contracts are ABI surface meant for
            // implementers and off-chain consumers, which may live outside the compiled sources.
            if let Some(contract_id) = error.contract
                && matches!(
                    hir.contract(contract_id).kind,
                    ContractKind::Interface | ContractKind::AbstractContract
                )
            {
                continue;
            }
            if !used.contains(&error_id) {
                ctx.emit(&sources[src_idx], &UNUSED_ERROR, error.span);
            }
        }
    }
}

/// Collects every [`hir::ErrorId`] referenced by an expression anywhere in the unit.
///
/// Resolved identifiers cover almost every use: the lowering resolves the full path of
/// `revert Lib.Err(...)` into a single `Ident`, and `require(cond, Err(...))` or `Err.selector`
/// reference the error through a resolved `Ident` as well. The one exception is a qualified
/// member access such as `Lib.Err.selector`: the inner `Err` segment carries no resolution in
/// the HIR, so it is resolved by name against the items of the base contract or namespace.
fn collect_used_errors<'hir>(hir: &'hir hir::Hir<'hir>) -> HashSet<hir::ErrorId> {
    let mut collector = UsedErrorCollector { hir, used: HashSet::new() };
    // Walk every source of the unit: functions, modifiers, and variable initializers.
    for source_id in hir.source_ids() {
        let _ = collector.visit_nested_source(source_id);
    }
    collector.used
}

struct UsedErrorCollector<'hir> {
    hir: &'hir hir::Hir<'hir>,
    used: HashSet<hir::ErrorId>,
}

impl<'hir> hir::Visit<'hir> for UsedErrorCollector<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            // Direct resolved reference: `revert Err()`, `revert Lib.Err()` (fully resolved by
            // the lowering), `require(cond, Err())`, `Err.selector`, ...
            hir::ExprKind::Ident(resolutions) => {
                // Symbols can be overloaded: consider every resolution.
                for res in *resolutions {
                    if let hir::Res::Item(hir::ItemId::Error(error_id)) = res {
                        self.used.insert(*error_id);
                    }
                }
            }
            // `Lib.Err.selector`: the `Err` member carries no resolution, so look the name up in
            // the items of the scopes its base can designate.
            hir::ExprKind::Member(base, member) => {
                for items in self.scope_items(base) {
                    self.mark_named_error(items, member);
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

impl<'hir> UsedErrorCollector<'hir> {
    /// Marks as used the errors of `items` whose name matches `member`.
    fn mark_named_error(&mut self, items: &[hir::ItemId], member: &solar::ast::Ident) {
        // Scopes cannot hold two same-name errors, but scan every item to stay conservative.
        for item_id in items {
            if let hir::ItemId::Error(error_id) = item_id
                && self.hir.error(*error_id).name.name == member.name
            {
                self.used.insert(*error_id);
            }
        }
    }

    /// Returns the items of the named scopes `expr` can designate: a contract or library through
    /// a resolved identifier, a module alias, or a member chain leading to one (`NS.Lib`).
    fn scope_items(&self, expr: &hir::Expr<'_>) -> Vec<&'hir [hir::ItemId]> {
        let mut scopes = Vec::new();
        match &expr.kind {
            hir::ExprKind::Ident(resolutions) => {
                for res in *resolutions {
                    match res {
                        hir::Res::Item(hir::ItemId::Contract(contract_id)) => {
                            scopes.push(self.hir.contract(*contract_id).items);
                        }
                        // A module alias exposes the file's global symbols, including the ones it
                        // re-exports through its own imports: take the transitive closure.
                        hir::Res::Namespace(source_id) => {
                            for sid in self.reachable_sources(*source_id) {
                                scopes.push(self.hir.source(sid).items);
                            }
                        }
                        _ => {}
                    }
                }
            }
            // A chained scope access (`NS.Lib`): look the name up in the scopes of the base.
            hir::ExprKind::Member(inner_base, name) => {
                for items in self.scope_items(inner_base) {
                    for item_id in items {
                        if let hir::ItemId::Contract(contract_id) = item_id
                            && self.hir.contract(*contract_id).name.name == name.name
                        {
                            scopes.push(self.hir.contract(*contract_id).items);
                        }
                    }
                }
            }
            _ => {}
        }
        scopes
    }

    /// Returns `root` and every source transitively reachable through its imports.
    fn reachable_sources(&self, root: hir::SourceId) -> Vec<hir::SourceId> {
        let mut seen: HashSet<hir::SourceId> = HashSet::new();
        let mut stack = vec![root];
        // Imports can form cycles (a file may even import itself): track visited sources.
        while let Some(source_id) = stack.pop() {
            if seen.insert(source_id) {
                for (_, imported) in self.hir.source(source_id).imports {
                    stack.push(*imported);
                }
            }
        }
        seen.into_iter().collect()
    }
}
