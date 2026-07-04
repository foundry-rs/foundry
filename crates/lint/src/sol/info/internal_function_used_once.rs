use super::InternalFunctionUsedOnce;
use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint},
};
use solar::{
    interface::{data_structures::Never, source_map::FileName},
    sema::{
        Gcx,
        hir::{self, Visit},
        ty::TyKind,
    },
};
use std::{collections::HashMap, ops::ControlFlow};

declare_forge_lint!(
    INTERNAL_FUNCTION_USED_ONCE,
    Severity::Info,
    "internal-function-used-once",
    "this internal function is used only once; consider inlining it into its caller"
);

impl<'ast> ProjectLintPass<'ast> for InternalFunctionUsedOnce {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]) {
        if !ctx.is_lint_enabled(INTERNAL_FUNCTION_USED_ONCE.id()) {
            return;
        }
        let gcx = ctx.gcx();
        let hir = &gcx.hir;

        // Map every input source's HIR `SourceId` to the corresponding `ProjectSource` index:
        // only functions declared in user-provided files are reported, while references are
        // counted across the whole unit, dependencies included.
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

        let counts = count_function_references(gcx);

        for function_id in hir.function_ids() {
            let function = hir.function(function_id);
            let Some(&src_idx) = input_source_idx.get(&function.source) else { continue };
            // Only ordinary internal functions with a body qualify. A name starting with `_`
            // follows the hook convention (OpenZeppelin style) and stays out, and so do
            // `virtual` functions and overrides: they exist for dynamic dispatch, so inlining
            // them is not an option and their reference count does not tell the story.
            if function.visibility != hir::Visibility::Internal
                || !function.is_ordinary()
                || function.body.is_none()
                || function.virtual_
                || function.override_
            {
                continue;
            }
            let Some(name) = function.name else { continue };
            if name.as_str().starts_with('_') {
                continue;
            }
            // Exactly one reference: zero references is dead code, a different concern.
            if counts.get(&function_id).copied().unwrap_or(0) == 1 {
                ctx.emit(&sources[src_idx], &INTERNAL_FUNCTION_USED_ONCE, function.keyword_span());
            }
        }
    }
}

/// Counts, for every function of the unit, the expressions that resolve to it, calls and
/// references used as values alike. `type_of_expr` gives the single declaration the type
/// checker selected, so overload selection, the qualified and `using for` forms and import
/// aliases are all attributed to the right function.
fn count_function_references(gcx: Gcx<'_>) -> HashMap<hir::FunctionId, usize> {
    let hir = &gcx.hir;
    let mut counter = ReferenceCounter { gcx, hir, counts: HashMap::new() };
    // Walk every source of the unit: functions, modifiers, and variable initializers.
    for source_id in hir.source_ids() {
        let _ = counter.visit_nested_source(source_id);
    }
    counter.counts
}

struct ReferenceCounter<'gcx> {
    gcx: Gcx<'gcx>,
    hir: &'gcx hir::Hir<'gcx>,
    counts: HashMap<hir::FunctionId, usize>,
}

impl<'gcx> hir::Visit<'gcx> for ReferenceCounter<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'gcx hir::Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        // A name or member expression typed as a function is one resolved reference.
        if matches!(expr.kind, hir::ExprKind::Ident(..) | hir::ExprKind::Member(..))
            && let Some(ty) = self.gcx.type_of_expr(expr.peel_parens().id)
            && let TyKind::Fn(function_ty) = ty.kind
            && let Some(function_id) = function_ty.function_id
        {
            *self.counts.entry(function_id).or_insert(0) += 1;
        }
        self.walk_expr(expr)
    }
}
