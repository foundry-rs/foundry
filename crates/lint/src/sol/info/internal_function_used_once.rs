use super::InternalFunctionUsedOnce;
use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint, analysis::resolved_function},
};
use solar::{
    interface::{data_structures::Never, source_map::FileName},
    sema::{
        Gcx,
        hir::{self, Visit},
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

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

        // Only functions declared in user-provided files are reported, while references are
        // counted across the whole unit, dependencies included.
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

        // A function bound as a user-defined operator (`using {f as +} for T`) is out of scope:
        // its operator uses are not `Ident`/`Member` references, and the binding requires a
        // named function anyway.
        let source_usings = gcx.hir.source_ids().flat_map(|id| gcx.hir.source(id).usings);
        let contract_usings = gcx.hir.contract_ids().flat_map(|id| gcx.hir.contract(id).usings);
        let operator_bound: HashSet<_> = source_usings
            .chain(contract_usings)
            .flat_map(|directive| directive.entries)
            .filter(|entry| entry.operator.is_some())
            .filter_map(|entry| match entry.kind {
                hir::UsingEntryKind::Functions(ids) => Some(ids),
                _ => None,
            })
            .flatten()
            .copied()
            .collect();

        let mut counter =
            ReferenceCounter { gcx, current: None, callee: None, refs: HashMap::new() };
        for source_id in gcx.hir.source_ids() {
            let _ = counter.visit_nested_source(source_id);
        }
        let refs = counter.refs;

        for function_id in gcx.hir.function_ids() {
            let function = gcx.hir.function(function_id);
            let Some(&src_idx) = input_source_idx.get(&function.source) else { continue };
            // Only ordinary internal functions with a body qualify. `virtual` functions and
            // overrides exist for dynamic dispatch, and a `_`-prefixed name follows the hook
            // convention (OpenZeppelin style).
            if function.visibility != hir::Visibility::Internal
                || !function.is_ordinary()
                || function.body.is_none()
                || function.virtual_
                || function.override_
                || operator_bound.contains(&function_id)
                || function.name.is_none_or(|name| name.as_str().starts_with('_'))
            {
                continue;
            }
            // Exactly one reference, and it must be a direct call: zero references is dead
            // code, a value-position reference (function pointer, callback) has no call site
            // to inline into, a recursive function cannot be inlined, and a reference that
            // only enters through a reference cycle has no caller to inline into either.
            let Some(info) = refs.get(&function_id) else { continue };
            if info.count == 1
                && !info.used_as_value
                && !info.self_referencing
                && !only_referenced_within_cycle(&refs, function_id)
            {
                ctx.emit(&sources[src_idx], &INTERNAL_FUNCTION_USED_ONCE, function.keyword_span());
            }
        }
    }
}

/// The references resolving to one function. Self-references are recorded apart rather than
/// counted; `first_from` is `None` for a reference outside any function body.
#[derive(Default)]
struct RefInfo {
    count: usize,
    used_as_value: bool,
    self_referencing: bool,
    first_from: Option<hir::FunctionId>,
}

/// Whether a function's single reference only enters it through a reference cycle: the chain
/// of single-reference sources loops back on `start` itself. A loop closing on a later node is
/// someone else's cycle, and `start` hangs off it as an inlineable tail.
fn only_referenced_within_cycle(
    refs: &HashMap<hir::FunctionId, RefInfo>,
    start: hir::FunctionId,
) -> bool {
    let mut visited = vec![start];
    let mut current = start;
    loop {
        let Some(info) = refs.get(&current) else { return false };
        // A fork (several references) or a reference from outside a function ends the chain.
        let Some(next) = info.first_from.filter(|_| info.count == 1) else { return false };
        if visited.contains(&next) {
            return next == start;
        }
        visited.push(next);
        current = next;
    }
}

/// Counts, for every function of the unit, the expressions the type checker resolved to it,
/// direct calls and value-position references alike.
struct ReferenceCounter<'gcx> {
    gcx: Gcx<'gcx>,
    /// The enclosing function, so each reference knows its source.
    current: Option<hir::FunctionId>,
    /// The callee of the call being walked: that reference is a direct call.
    callee: Option<hir::ExprId>,
    refs: HashMap<hir::FunctionId, RefInfo>,
}

impl<'gcx> hir::Visit<'gcx> for ReferenceCounter<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_nested_function(&mut self, id: hir::FunctionId) -> ControlFlow<Self::BreakValue> {
        let previous = self.current.replace(id);
        let result = self.visit_function(self.gcx.hir.function(id));
        self.current = previous;
        result
    }

    fn visit_expr(&mut self, expr: &'gcx hir::Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            hir::ExprKind::Call(callee, ..) => self.callee = Some(callee.peel_parens().id),
            hir::ExprKind::Ident(..) | hir::ExprKind::Member(..) => {
                if let Some(function_id) = resolved_function(self.gcx, expr) {
                    let is_call = self.callee == Some(expr.id);
                    let info = self.refs.entry(function_id).or_default();
                    if self.current == Some(function_id) {
                        info.self_referencing = true;
                    } else {
                        info.count += 1;
                        info.used_as_value |= !is_call;
                        if info.count == 1 {
                            info.first_from = self.current;
                        }
                    }
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}
