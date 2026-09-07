use super::ModifierUsedOnlyOnce;
use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint},
};
use solar::{ast::FunctionKind, interface::source_map::FileName, sema::hir};
use std::collections::HashMap;

declare_forge_lint!(
    MODIFIER_USED_ONLY_ONCE,
    Severity::Info,
    "modifier-used-only-once",
    "this modifier is used only once; consider inlining its checks into the function"
);

impl<'ast> ProjectLintPass<'ast> for ModifierUsedOnlyOnce {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]) {
        if !ctx.is_lint_enabled(MODIFIER_USED_ONLY_ONCE.id()) {
            return;
        }

        // Only modifiers declared in user-provided files are reported, while invocations are
        // counted across the whole unit, dependencies included.
        let input_source_idx: HashMap<_, _> = ctx
            .gcx()
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

        // Invocations live in each function's resolved modifier list, where base-constructor
        // calls carry a contract id and stay out of the count.
        let mut counts = HashMap::<hir::FunctionId, usize>::new();
        for function_id in ctx.gcx().hir.function_ids() {
            for invocation in ctx.gcx().hir.function(function_id).modifiers {
                if let hir::ItemId::Function(modifier_id) = invocation.id {
                    *counts.entry(modifier_id).or_default() += 1;
                }
            }
        }

        for function_id in ctx.gcx().hir.function_ids() {
            let function = ctx.gcx().hir.function(function_id);
            let Some(&src_idx) = input_source_idx.get(&function.source) else { continue };
            // Only modifier declarations with a body qualify. `virtual` modifiers and overrides
            // exist for dynamic dispatch, so inlining them is not an option. Exactly one
            // invocation: zero invocations is dead code, a different concern.
            if function.kind == FunctionKind::Modifier
                && function.body.is_some()
                && !function.virtual_
                && !function.override_
                && counts.get(&function_id) == Some(&1)
            {
                ctx.emit(&sources[src_idx], &MODIFIER_USED_ONLY_ONCE, function.keyword_span());
            }
        }
    }
}
