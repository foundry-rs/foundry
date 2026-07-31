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
        let gcx = ctx.gcx();
        let hir = &gcx.hir;

        // Map every input source's HIR `SourceId` to the corresponding `ProjectSource` index:
        // only modifiers declared in user-provided files are reported, while invocations are
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

        let counts = count_modifier_invocations(hir);

        for function_id in hir.function_ids() {
            let function = hir.function(function_id);
            let Some(&src_idx) = input_source_idx.get(&function.source) else { continue };
            // Only modifier declarations with a body qualify. `virtual` modifiers and
            // overrides exist for dynamic dispatch, so inlining them is not an option and
            // their invocation count does not tell the story.
            if function.kind != FunctionKind::Modifier
                || function.body.is_none()
                || function.virtual_
                || function.override_
            {
                continue;
            }
            // Exactly one invocation: zero invocations is dead code, a different concern.
            if counts.get(&function_id).copied().unwrap_or(0) == 1 {
                ctx.emit(&sources[src_idx], &MODIFIER_USED_ONLY_ONCE, function.keyword_span());
            }
        }
    }
}

/// Counts, for every modifier of the unit, the functions that invoke it. Invocations live in
/// each function's resolved modifier list, where base-constructor calls carry a contract id
/// and stay out of the count.
fn count_modifier_invocations(hir: &hir::Hir<'_>) -> HashMap<hir::FunctionId, usize> {
    let mut counts = HashMap::new();
    // Every function of the unit, constructors included, can invoke modifiers.
    for function_id in hir.function_ids() {
        for invocation in hir.function(function_id).modifiers {
            if let hir::ItemId::Function(modifier_id) = invocation.id {
                *counts.entry(modifier_id).or_insert(0) += 1;
            }
        }
    }
    counts
}
