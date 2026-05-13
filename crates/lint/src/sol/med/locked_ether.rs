use super::LockedEther;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, LitKind, StateMutability},
    interface::{kw, sym},
    sema::{
        builtins::Builtin,
        hir::{self, ExprKind, FunctionId, ItemId, Res, Visit as _},
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    LOCKED_ETHER,
    Severity::Med,
    "locked-ether",
    "contract can receive ETH but has no mechanism to send it out"
);

impl<'hir> LateLintPass<'hir> for LockedEther {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        contract_id: hir::ContractId,
    ) {
        if !ctx.is_lint_enabled(LOCKED_ETHER.id) {
            return;
        }

        let contract = hir.contract(contract_id);

        // Libraries and interfaces cannot hold ETH.
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract) {
            return;
        }
        if contract.linearization_failed() {
            return;
        }

        // `receive()` and payable `fallback()` are required to be `Payable`, so a single
        // mutability check also covers them along with payable constructors and functions.
        let has_payable_entry = contract.linearized_bases.iter().any(|&cid| {
            hir.contract(cid)
                .all_functions()
                .any(|fid| hir.function(fid).state_mutability == StateMutability::Payable)
        });
        if !has_payable_entry {
            return;
        }

        // Walk every function in `self` and its bases. Internal/library calls resolved to a
        // `FunctionId` are queued for transitive analysis; unresolved external calls are
        // conservatively ignored.
        let mut visited: HashSet<FunctionId> = HashSet::new();
        let mut worklist: Vec<FunctionId> = contract
            .linearized_bases
            .iter()
            .flat_map(|&cid| hir.contract(cid).all_functions())
            .collect();

        while let Some(fid) = worklist.pop() {
            if !visited.insert(fid) {
                continue;
            }
            let func = hir.function(fid);

            for modifier in func.modifiers {
                for arg in modifier.args.exprs() {
                    let mut checker =
                        SendChecker { hir, worklist: &mut worklist, visited: &visited };
                    if checker.visit_expr(arg).is_break() {
                        return;
                    }
                }
            }

            if let Some(body) = func.body {
                let mut checker = SendChecker { hir, worklist: &mut worklist, visited: &visited };
                for stmt in body.stmts {
                    if checker.visit_stmt(stmt).is_break() {
                        return;
                    }
                }
            }
        }

        ctx.emit(&LOCKED_ETHER, contract.name.span);
    }
}

/// HIR visitor that short-circuits on the first ETH-sending expression and queues
/// internally-resolved callees for transitive exploration by the outer worklist loop.
struct SendChecker<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    worklist: &'a mut Vec<FunctionId>,
    visited: &'a HashSet<FunctionId>,
}

impl<'hir> hir::Visit<'hir> for SendChecker<'_, 'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if expr_sends_ether(expr) {
            return ControlFlow::Break(());
        }

        // Queue calls whose callee resolves statically to a `FunctionId`.
        if let ExprKind::Call(callee, ..) = &expr.kind
            && let ExprKind::Ident(reses) = &callee.peel_parens().kind
        {
            for res in *reses {
                if let Res::Item(ItemId::Function(fid)) = res
                    && !self.visited.contains(fid)
                {
                    self.worklist.push(*fid);
                }
            }
        }

        self.walk_expr(expr)
    }
}

/// Returns `true` if `expr` unambiguously moves ETH out of the contract: a non-zero `{value: x}`
/// call option, `.transfer`/`.send` with a non-zero amount, low-level `.delegatecall`/`.callcode`
/// (drainable via `selfdestruct`), or the `selfdestruct` builtin. Only literal `0` is treated as
/// a zero amount; any other expression is assumed non-zero.
fn expr_sends_ether(expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, named_args) = &expr.kind else {
        return false;
    };

    // `foo{value: x}(...)` / `new C{value: x}(...)` with `x != 0`.
    if let Some(opts) = named_args
        && opts.iter().any(|arg| arg.name.name == sym::value && !is_literal_zero(&arg.value))
    {
        return true;
    }

    let callee = callee.peel_parens();
    match &callee.kind {
        ExprKind::Member(_, member) => {
            // Single-arg `.transfer`/`.send` to disambiguate from ERC20's 2-arg `transfer`.
            if matches!(member.name, sym::transfer | sym::send) && args.len() == 1 {
                let amt = args.exprs().next().expect("len == 1");
                if !is_literal_zero(amt) {
                    return true;
                }
            }
            if matches!(member.name, kw::Delegatecall | kw::Callcode) {
                return true;
            }
        }
        ExprKind::Ident(reses) => {
            if reses.iter().any(|r| matches!(r, Res::Builtin(Builtin::Selfdestruct))) {
                return true;
            }
        }
        _ => {}
    }

    false
}

/// Returns `true` if the expression is the integer literal `0`.
fn is_literal_zero(expr: &hir::Expr<'_>) -> bool {
    if let ExprKind::Lit(lit) = &expr.peel_parens().kind
        && let LitKind::Number(n) = &lit.kind
    {
        return n.is_zero();
    }
    false
}
