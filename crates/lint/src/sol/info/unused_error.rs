use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint, info::UnusedError},
};
use solar::{
    ast::ContractKind,
    interface::{data_structures::Never, source_map::FileName},
    sema::{
        Gcx,
        hir::{self, Visit as _},
    },
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

        // Only errors declared in user-provided files are reported, while uses are collected
        // across the whole unit, so an error declared here and reverted in a dependency (or the
        // other way around) is attributed correctly.
        let input_source_idx: HashMap<_, _> = gcx
            .hir
            .sources_enumerated()
            .filter_map(|(sid, src)| {
                let FileName::Real(path) = &src.file.name else { return None };
                Some((sid, sources.iter().position(|s| &s.path == path)?))
            })
            .collect();
        if input_source_idx.is_empty() {
            return;
        }

        let mut collector = UsedErrorCollector { gcx, used: HashSet::new() };
        for source_id in gcx.hir.source_ids() {
            let _ = collector.visit_nested_source(source_id);
        }

        for error_id in gcx.hir.error_ids() {
            let error = gcx.hir.error(error_id);
            let Some(&src_idx) = input_source_idx.get(&error.source) else { continue };
            // Errors declared in interfaces and abstract contracts are ABI surface meant for
            // implementers and off-chain consumers, which may live outside the compiled sources.
            let abi_surface = error.contract.is_some_and(|id| {
                matches!(
                    gcx.hir.contract(id).kind,
                    ContractKind::Interface | ContractKind::AbstractContract
                )
            });
            if !abi_surface && !collector.used.contains(&error_id) {
                ctx.emit(&sources[src_idx], &UNUSED_ERROR, error.span);
            }
        }
    }
}

/// Collects every error referenced by an expression anywhere in the unit.
struct UsedErrorCollector<'gcx> {
    gcx: Gcx<'gcx>,
    used: HashSet<hir::ErrorId>,
}

impl<'gcx> hir::Visit<'gcx> for UsedErrorCollector<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx hir::Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        if let Some(hir::Res::Item(hir::ItemId::Error(error_id))) = self.gcx.resolved_expr(expr) {
            self.used.insert(error_id);
        }
        self.walk_expr(expr)
    }
}
