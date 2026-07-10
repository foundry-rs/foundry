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
        let operator_bound = operator_bound_functions(hir);

        for function_id in hir.function_ids() {
            let function = hir.function(function_id);
            let Some(&src_idx) = input_source_idx.get(&function.source) else { continue };
            // A function bound as a user-defined operator is out of scope: its operator uses
            // are not `Ident`/`Member` references, so its count would lie, and inlining it is
            // not an option anyway, the `using {f as +}` binding requires a named function.
            if operator_bound.contains(&function_id) {
                continue;
            }
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
            // Exactly one reference, and it must be a direct call: zero references is dead
            // code, a different concern, and a reference used as a value (assigned to a
            // function pointer, returned, or passed as a callback) has no call site to inline
            // into, so it is not an inlining candidate. Self-references do not count and mark
            // the function recursive, which rules it out entirely: a recursive function cannot
            // be inlined into its caller. A single reference that only enters through a
            // reference cycle (mutually recursive helpers with no external caller) has no
            // caller to inline into either.
            let Some(info) = counts.get(&function_id) else { continue };
            if info.count == 1
                && info.value_count == 0
                && !info.self_referencing
                && !only_referenced_within_cycle(&counts, function_id)
            {
                ctx.emit(&sources[src_idx], &INTERNAL_FUNCTION_USED_ONCE, function.keyword_span());
            }
        }
    }
}

/// Collects every function the unit binds as a user-defined operator, through a
/// `using {f as +} for T` entry of a file-level or contract-level directive. The HIR
/// already resolved those entries to function ids.
fn operator_bound_functions(hir: &hir::Hir<'_>) -> HashSet<hir::FunctionId> {
    let mut bound = HashSet::new();
    // File-level directives, then contract-level ones: an operator entry can sit in either.
    let source_usings = hir.source_ids().flat_map(|id| hir.source(id).usings.iter());
    let contract_usings = hir.contract_ids().flat_map(|id| hir.contract(id).usings.iter());
    for directive in source_usings.chain(contract_usings) {
        for entry in directive.entries {
            // Only braced function entries can carry an operator binding.
            if entry.operator.is_some()
                && let hir::UsingEntryKind::Functions(ids) = entry.kind
            {
                bound.extend(ids.iter().copied());
            }
        }
    }
    bound
}

/// The references resolving to one function: how many, whether the function references
/// itself, and which function the first reference came from, `None` when it came from
/// outside any function body (a variable initializer). Self-references are recorded apart
/// rather than counted.
#[derive(Default)]
struct RefInfo {
    count: usize,
    value_count: usize,
    self_referencing: bool,
    first_from: Option<hir::FunctionId>,
}

/// Counts, for every function of the unit, the expressions that resolve to it, calls and
/// references used as values alike. `type_of_expr` gives the single declaration the type
/// checker selected, so overload selection, the qualified and `using for` forms and import
/// aliases are all attributed to the right function.
fn count_function_references(gcx: Gcx<'_>) -> HashMap<hir::FunctionId, RefInfo> {
    let hir = &gcx.hir;
    let mut counter = ReferenceCounter { gcx, hir, current: None, refs: HashMap::new() };
    // Walk every source of the unit: functions, modifiers, and variable initializers.
    for source_id in hir.source_ids() {
        let _ = counter.visit_nested_source(source_id);
    }
    counter.refs
}

/// Whether a function's single reference only enters it through a reference cycle: the
/// chain of single-reference sources loops back on itself, so there is no non-recursive
/// caller to inline into (mutually recursive helpers with no external caller).
fn only_referenced_within_cycle(
    refs: &HashMap<hir::FunctionId, RefInfo>,
    start: hir::FunctionId,
) -> bool {
    let mut visited = vec![start];
    let Some(mut current) = refs.get(&start).and_then(|info| info.first_from) else {
        return false;
    };
    // Each hop follows the unique referencing function. The chain is linear, so when it
    // loops, the cycle contains `start` exactly when the loop closes on `start` itself: a
    // loop closing on a later node is someone else's cycle, and `start` hangs off it as an
    // inlineable tail.
    loop {
        if visited.contains(&current) {
            return current == start;
        }
        visited.push(current);
        let Some(info) = refs.get(&current) else { return false };
        // A fork (several references) or a reference from outside a function ends the
        // chain: the start is reachable from a non-cyclic context.
        if info.count != 1 {
            return false;
        }
        let Some(next) = info.first_from else { return false };
        current = next;
    }
}

struct ReferenceCounter<'gcx> {
    gcx: Gcx<'gcx>,
    hir: &'gcx hir::Hir<'gcx>,
    current: Option<hir::FunctionId>,
    refs: HashMap<hir::FunctionId, RefInfo>,
}

impl<'gcx> hir::Visit<'gcx> for ReferenceCounter<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        self.hir
    }

    fn visit_nested_function(&mut self, id: hir::FunctionId) -> ControlFlow<Self::BreakValue> {
        // The enclosing function is tracked so each reference knows its source.
        let previous = self.current.replace(id);
        let result = self.visit_function(self.hir.function(id));
        self.current = previous;
        result
    }

    fn visit_expr(&mut self, expr: &'gcx hir::Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        // The callee of a call is a direct, inlinable use: record it as a call and descend into
        // the callee's own sub-expressions (a member receiver, a nested call) without also
        // counting the callee node as a value reference. Every other name or member expression
        // typed as a function is a value-position reference, such as a function pointer that is
        // assigned, returned or passed as a callback, which has no call site to inline into.
        if let hir::ExprKind::Call(callee, args, opts) = &expr.kind {
            let peeled = callee.peel_parens();
            match peeled.kind {
                hir::ExprKind::Ident(_) => self.record_reference(peeled, true),
                hir::ExprKind::Member(receiver, _) => {
                    self.record_reference(peeled, true);
                    let _ = self.visit_expr(receiver);
                }
                _ => {
                    let _ = self.visit_expr(peeled);
                }
            }
            if let Some(opts) = opts {
                for arg in opts.args {
                    let _ = self.visit_expr(&arg.value);
                }
            }
            return self.visit_call_args(args);
        }
        if matches!(expr.kind, hir::ExprKind::Ident(..) | hir::ExprKind::Member(..)) {
            self.record_reference(expr, false);
        }
        self.walk_expr(expr)
    }
}

impl<'gcx> ReferenceCounter<'gcx> {
    /// Records one resolved function reference from `expr`, marking whether it is a direct call
    /// or a value-position use. `type_of_expr` gives the single declaration the type checker
    /// selected, so overloads, the qualified and `using for` forms and import aliases are all
    /// attributed to the right function. A self-reference marks the function recursive rather
    /// than counting.
    fn record_reference(&mut self, expr: &hir::Expr<'_>, is_call: bool) {
        let Some(ty) = self.gcx.type_of_expr(expr.peel_parens().id) else { return };
        let TyKind::Fn(function_ty) = ty.kind else { return };
        let Some(function_id) = function_ty.function_id else { return };
        let info = self.refs.entry(function_id).or_default();
        if self.current == Some(function_id) {
            info.self_referencing = true;
        } else {
            info.count += 1;
            if info.count == 1 {
                info.first_from = self.current;
            }
            if !is_call {
                info.value_count += 1;
            }
        }
    }
}
