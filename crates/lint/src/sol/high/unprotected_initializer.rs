use super::UnprotectedInitializer;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{builtins, function_ids, is_builtin, runtime_entry_points},
    },
};
use alloy_primitives::map::HashSet;
use solar::{
    ast::{ContractKind, DataLocation, StateMutability},
    interface::{kw, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{self, ContractId, Expr, ExprKind, FunctionId, ItemId, Res, Visit},
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(
    UNPROTECTED_INITIALIZER,
    Severity::High,
    "unprotected-initializer",
    "upgradeable initializer is not protected against direct implementation calls"
);

impl<'gcx> LateLintPass<'gcx> for UnprotectedInitializer {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract_id: ContractId,
    ) {
        let contract = gcx.hir.contract(contract_id);
        if contract.kind != ContractKind::Contract || contract.linearization_failed() {
            return;
        }
        let bases = contract.linearized_bases;

        // The effective runtime dispatch surface: most-derived overrides plus the inherited
        // fallback/receive functions.
        let entries = runtime_entry_points(gcx, contract_id);

        let upgradeable = bases
            .iter()
            .any(|&cid| gcx.hir.contract(cid).name.as_str() == "Initializable")
            || entries.iter().any(|&fid| has_initializer_modifier(&gcx.hir, gcx.hir.function(fid)));
        if !upgradeable {
            return;
        }
        let locked = bases.iter().filter_map(|&cid| gcx.hir.contract(cid).ctor).any(|ctor| {
            reaches(gcx, bases, ctor, |expr| {
                let ExprKind::Call(callee, ..) = &expr.kind else { return false };
                callees(&gcx.hir, callee, bases).into_iter().any(|fid| {
                    let func = gcx.hir.function(fid);
                    func.contract.is_some_and(|cid| bases.contains(&cid))
                        && func.name.is_some_and(|name| name.as_str() == "_disableInitializers")
                })
            })
        });
        if locked {
            return;
        }
        let destructive = entries.iter().any(|&fid| {
            !has_modifier_named(&gcx.hir, gcx.hir.function(fid), "onlyProxy")
                && reaches(gcx, bases, fid, is_destructive_call)
        });
        if !destructive {
            return;
        }

        for fid in entries {
            let func = gcx.hir.function(fid);
            if func.is_part_of_external_interface()
                && !matches!(func.state_mutability, StateMutability::Pure | StateMutability::View)
                && has_initializer_modifier(&gcx.hir, func)
                && !has_modifier_named(&gcx.hir, func, "onlyProxy")
                && reaches(gcx, bases, fid, |expr| writes_state(gcx, expr))
            {
                ctx.emit(&UNPROTECTED_INITIALIZER, func.name.map_or(func.span, |name| name.span));
            }
        }
    }
}

fn has_initializer_modifier(hir: &hir::Hir<'_>, func: &hir::Function<'_>) -> bool {
    has_modifier_named(hir, func, "initializer") || has_modifier_named(hir, func, "reinitializer")
}

fn has_modifier_named(hir: &hir::Hir<'_>, func: &hir::Function<'_>, name: &str) -> bool {
    func.modifiers.iter().any(|modifier| {
        modifier
            .id
            .as_function()
            .is_some_and(|fid| hir.function(fid).name.is_some_and(|ident| ident.as_str() == name))
    })
}

/// True if `hit` matches an expression in `fid`'s body or, transitively, in the body of any
/// internal function it calls.
fn reaches<'gcx>(
    gcx: Gcx<'gcx>,
    bases: &'gcx [ContractId],
    fid: FunctionId,
    hit: impl FnMut(&'gcx Expr<'gcx>) -> bool,
) -> bool {
    Reach { gcx, bases, visited: HashSet::default(), hit }.visit_function_body(fid).is_break()
}

struct Reach<'gcx, F> {
    gcx: Gcx<'gcx>,
    bases: &'gcx [ContractId],
    // The predicate and dispatch context are fixed for the entire reachability check.
    visited: HashSet<FunctionId>,
    hit: F,
}

impl<'gcx, F: FnMut(&'gcx Expr<'gcx>) -> bool> Reach<'gcx, F> {
    fn visit_function_body(&mut self, fid: FunctionId) -> ControlFlow<()> {
        if !self.visited.insert(fid) {
            return ControlFlow::Continue(());
        }
        let Some(body) = self.gcx.hir.function(fid).body else {
            return ControlFlow::Continue(());
        };
        body.stmts.iter().try_for_each(|stmt| self.visit_stmt(stmt))
    }
}

impl<'gcx, F: FnMut(&'gcx Expr<'gcx>) -> bool> Visit<'gcx> for Reach<'gcx, F> {
    type BreakValue = ();

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<()> {
        if (self.hit)(expr) {
            return ControlFlow::Break(());
        }
        if let ExprKind::Call(callee, ..) = &expr.kind {
            for fid in callees(&self.gcx.hir, callee, self.bases) {
                self.visit_function_body(fid)?;
            }
        }
        self.walk_expr(expr)
    }
}

/// Functions an internal call may dispatch to: bare identifiers (all overloads), `super.f` (every
/// base `f`) and `Contract.f`.
fn callees(hir: &hir::Hir<'_>, callee: &Expr<'_>, bases: &[ContractId]) -> Vec<FunctionId> {
    let ExprKind::Member(base, method) = &callee.peel_parens().kind else {
        return function_ids(callee).collect();
    };
    let ExprKind::Ident(reses) = &base.peel_parens().kind else { return Vec::new() };
    let contracts = if is_builtin(base, sym::super_) {
        bases.get(1..).unwrap_or_default().to_vec()
    } else {
        reses
            .iter()
            .filter_map(|res| match res {
                Res::Item(ItemId::Contract(cid)) => Some(*cid),
                _ => None,
            })
            .collect()
    };
    contracts
        .into_iter()
        .flat_map(|cid| hir.contract(cid).all_functions())
        .filter(|&fid| hir.function(fid).name.is_some_and(|name| name.name == method.name))
        .collect()
}

/// `x.delegatecall(..)`, `x.callcode(..)` or `selfdestruct(..)`.
fn is_destructive_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, ..) = &expr.kind else { return false };
    match &callee.peel_parens().kind {
        ExprKind::Member(_, member) => matches!(member.name, kw::Delegatecall | kw::Callcode),
        _ => builtins(callee).any(|builtin| builtin == Builtin::Selfdestruct),
    }
}

/// An assignment, `delete`, `++`/`--` or `push`/`pop` whose target lives in contract storage.
fn writes_state(gcx: Gcx<'_>, expr: &Expr<'_>) -> bool {
    match &expr.kind {
        ExprKind::Assign(lhs, ..) | ExprKind::Delete(lhs) => lhs_writes_state(gcx, lhs),
        ExprKind::Unary(op, lhs) => op.kind.has_side_effects() && lhs_writes_state(gcx, lhs),
        ExprKind::Call(callee, ..) => {
            matches!(&callee.peel_parens().kind, ExprKind::Member(base, member)
                if matches!(member.as_str(), "push" | "pop") && references_storage(gcx, base))
        }
        _ => false,
    }
}

/// A state variable, or a member/index of an expression that denotes contract storage.
fn lhs_writes_state(gcx: Gcx<'_>, lhs: &Expr<'_>) -> bool {
    match &lhs.peel_parens().kind {
        ExprKind::Ident(reses) => {
            reses.iter().filter_map(Res::as_variable).any(|v| gcx.hir.variable(v).kind.is_state())
        }
        ExprKind::Index(base, _) | ExprKind::Slice(base, ..) | ExprKind::Member(base, _) => {
            references_storage(gcx, base)
        }
        ExprKind::Tuple(elems) => elems.iter().flatten().any(|elem| lhs_writes_state(gcx, elem)),
        _ => false,
    }
}

fn references_storage(gcx: Gcx<'_>, expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().filter_map(Res::as_variable).any(|v| {
            let var = gcx.hir.variable(v);
            var.kind.is_state() || var.data_location == Some(DataLocation::Storage)
        }),
        ExprKind::Index(base, _) | ExprKind::Slice(base, ..) | ExprKind::Member(base, _) => {
            references_storage(gcx, base)
        }
        _ => gcx
            .type_of_expr(expr.peel_parens().id)
            .is_some_and(|ty| ty.loc() == Some(DataLocation::Storage)),
    }
}
