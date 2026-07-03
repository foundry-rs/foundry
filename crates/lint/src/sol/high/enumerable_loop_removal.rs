use super::EnumerableLoopRemoval;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::Span,
    sema::{
        Gcx,
        hir::{
            self, CallArgsKind, Expr, ExprKind, Hir, ItemId, Res, Stmt, StmtKind, VariableId, Visit,
        },
        ty::TyKind,
    },
};
use std::{collections::HashSet, convert::Infallible, ops::ControlFlow};

declare_forge_lint!(
    ENUMERABLE_LOOP_REMOVAL,
    Severity::High,
    "enumerable-loop-removal",
    "`remove` on an EnumerableSet inside a loop that iterates it with `at` corrupts the iteration"
);

impl<'hir> LateLintPass<'hir> for EnumerableLoopRemoval {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        // EnumerableSet removal is swap-and-pop, so removing while iterating the same set at
        // a varying index in the same loop skips elements or reads out-of-bounds indices. The
        // safe patterns (collect during the loop and remove in a later loop without `at`,
        // drain at a literal index, iterate a different set) stay clean.
        if let Some(body) = &func.body {
            let mut finder = LoopFinder { gcx, hir, ctx, emitted: HashSet::new() };
            // A `Block` has no dedicated visit hook: walk its statements directly.
            for stmt in body.stmts {
                let _ = finder.visit_stmt(stmt);
            }
        }
    }
}

/// Walks a function body and, for each loop, flags the EnumerableSet `remove` calls of its
/// subtree when that same subtree also iterates the removed set with `at` at a varying index.
struct LoopFinder<'ctx, 's, 'c, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    ctx: &'ctx LintContext<'s, 'c>,
    // A loop nested in a flagged loop sees the same calls: dedupe emissions by span.
    emitted: HashSet<Span>,
}

impl<'hir> Visit<'hir> for LoopFinder<'_, '_, '_, 'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        // Every loop form lowers to `StmtKind::Loop`, so one case covers for/while/do-while.
        if let StmtKind::Loop(body, _) = &stmt.kind {
            let mut ops = SetOpsCollector {
                gcx: self.gcx,
                hir: self.hir,
                removes: Vec::new(),
                at_identities: Vec::new(),
            };
            for inner in body.stmts {
                let _ = ops.visit_stmt(inner);
            }
            // Only removing from a set the same subtree iterates at a varying index corrupts
            // that iteration: `remove` alone is the recommended collect-then-remove pattern,
            // `at` alone is a plain read.
            for (identity, span) in ops.removes {
                // A removal corrupts the loop when any iterated set can be the removed one.
                let mut corrupts = false;
                for at_identity in &ops.at_identities {
                    if identities_can_alias(identity, *at_identity) {
                        corrupts = true;
                    }
                }
                if corrupts && self.emitted.insert(span) {
                    self.ctx.emit(&ENUMERABLE_LOOP_REMOVAL, span);
                }
            }
        }
        self.walk_stmt(stmt)
    }
}

/// Collects the EnumerableSet `remove` calls of a subtree (with the identity of the removed
/// set) and the identities of the sets the subtree iterates with `at` at a varying index.
struct SetOpsCollector<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    removes: Vec<(Option<VariableId>, Span)>,
    at_identities: Vec<Option<VariableId>>,
}

impl<'hir> Visit<'hir> for SetOpsCollector<'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        // Both call forms matter: `set.remove(value)` bound by `using for` and the qualified
        // `EnumerableSet.remove(set, value)`. The type checker resolves the callee either
        // way, so judge the resolved function and locate the set operand from the call shape.
        if let ExprKind::Call(callee, args, _) = &expr.kind
            && let Some(function) = self.enumerable_set_function(callee)
        {
            // The set operand is the bound receiver in the method form and the first
            // argument in the qualified form; the index of `at` sits right after it.
            let (set_expr, index_position) = match &callee.peel_parens().kind {
                ExprKind::Member(receiver, _)
                    if is_enumerable_set_value(self.gcx, self.hir, receiver) =>
                {
                    (Some(&**receiver), 0)
                }
                _ => (nth_positional_arg(args, 0), 1),
            };
            let identity = set_identity(set_expr);
            if function.name.is_some_and(is_named_remove) {
                self.removes.push((identity, expr.span));
            } else if function.name.is_some_and(is_named_at)
                && !nth_positional_arg(args, index_position).is_some_and(is_literal)
            {
                // Reading at a literal index does not iterate the set: the swap-and-pop
                // refills that position, so a drain like `remove(at(0))` stays clean. A
                // missing or named-form index cannot be inspected and counts as varying.
                self.at_identities.push(identity);
            }
        }
        self.walk_expr(expr)
    }
}

impl<'hir> SetOpsCollector<'hir> {
    /// The function the call dispatches to, when it is declared in a library named
    /// `EnumerableSet`. Resolving through the type checker covers the `using for` method
    /// form, the library-qualified form and import aliases, and keeps same-name functions
    /// from other libraries out.
    fn enumerable_set_function(&self, callee: &Expr<'_>) -> Option<&'hir hir::Function<'hir>> {
        let ty = self.gcx.type_of_expr(callee.peel_parens().id)?;
        let TyKind::Fn(function_ty) = ty.kind else { return None };
        let function = self.hir.function(function_ty.function_id?);
        let contract = self.hir.contract(function.contract?);
        (contract.kind.is_library() && contract.name.as_str() == "EnumerableSet")
            .then_some(function)
    }
}

fn is_named_remove(name: solar::ast::Ident) -> bool {
    name.as_str() == "remove"
}

fn is_named_at(name: solar::ast::Ident) -> bool {
    name.as_str() == "at"
}

/// Whether two set identities can refer to the same set: equal variables do, and an
/// expression too complex to name a single variable (`None`) can alias either set.
fn identities_can_alias(removed: Option<VariableId>, iterated: Option<VariableId>) -> bool {
    match (removed, iterated) {
        (Some(removed_id), Some(iterated_id)) => removed_id == iterated_id,
        _ => true,
    }
}

/// The `n`-th argument of a call made with positional arguments.
fn nth_positional_arg<'hir>(args: &hir::CallArgs<'hir>, n: usize) -> Option<&'hir Expr<'hir>> {
    match &args.kind {
        CallArgsKind::Unnamed(exprs) => exprs.get(n),
        CallArgsKind::Named(_) => None,
    }
}

/// The state variable a set expression refers to, the identity telling two sets apart.
fn set_identity(set_expr: Option<&Expr<'_>>) -> Option<VariableId> {
    // A set operand that could not be located has no identity.
    let expr = set_expr?;
    // Only a plain name resolving to a variable identifies a set; anything else (an index
    // into a mapping of sets, a call result) stays `None` and matches conservatively.
    if let ExprKind::Ident(resolutions) = &expr.peel_parens().kind {
        for res in *resolutions {
            if let Res::Item(ItemId::Variable(variable_id)) = res {
                return Some(*variable_id);
            }
        }
    }
    None
}

/// Whether the index expression is a plain literal.
fn is_literal(expr: &Expr<'_>) -> bool {
    matches!(expr.peel_parens().kind, ExprKind::Lit(..))
}

/// Whether `receiver` is a value of one of the set struct types declared in a library (or
/// contract) named `EnumerableSet` (`AddressSet` / `UintSet` / `Bytes32Set`), which tells the
/// bound method form apart from the library-qualified form.
fn is_enumerable_set_value<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    receiver: &Expr<'_>,
) -> bool {
    let Some(ty) = gcx.type_of_expr(receiver.peel_parens().id) else { return false };
    let TyKind::Struct(id) = ty.peel_refs().kind else { return false };
    let Some(contract_id) = hir.strukt(id).contract else { return false };
    hir.contract(contract_id).name.as_str() == "EnumerableSet"
}
