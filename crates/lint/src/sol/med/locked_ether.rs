use super::LockedEther;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            block_outcome, expr_is_address, is_address_self, is_builtin, is_contract_cast,
            is_literal_zero, runtime_entry_points,
        },
    },
};
use solar::{
    ast::{ContractKind, StateMutability},
    interface::{Span, kw, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, Block, ContractId, ExprKind, FunctionId, ItemId, Res, StmtKind, TypeKind, Visit,
        },
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    LOCKED_ETHER,
    Severity::Med,
    "locked-ether",
    "contract can receive ETH but has no mechanism to send it out"
);

impl<'gcx> LateLintPass<'gcx> for LockedEther {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract_id: ContractId,
    ) {
        let contract = gcx.hir.contract(contract_id);
        // Libraries and interfaces cannot hold ETH.
        if !ctx.is_lint_enabled(LOCKED_ETHER.id)
            || !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract)
            || contract.linearization_failed()
        {
            return;
        }

        let receives = |fid: FunctionId| {
            let func = gcx.hir.function(fid);
            func.state_mutability == StateMutability::Payable
                && !always_reverts(gcx, contract_id, func)
        };
        // Runtime entries and the constructor are separate inflow channels: only the leaf's own
        // constructor receives deployment value, and it has no runtime exit path.
        let entries = runtime_entry_points(gcx, contract_id);
        if !entries.iter().any(|&fid| receives(fid)) && !contract.ctor.is_some_and(receives) {
            return;
        }

        // Explore the runtime entries and, transitively, the helpers and modifiers they reach.
        // Constructor bodies are excluded so their exits don't count.
        let mut visited = HashSet::new();
        let mut checker = SendChecker { gcx, contract_id, worklist: entries };
        while let Some(fid) = checker.worklist.pop() {
            let func = gcx.hir.function(fid);
            // Any ETH movement inside an always-reverting function rolls back.
            if !visited.insert(fid) || always_reverts(gcx, contract_id, func) {
                continue;
            }
            for modifier in func.modifiers {
                if checker.visit_call_args(&modifier.args).is_break() {
                    return;
                }
                checker.worklist.extend(gcx.resolve_modifier_target(contract_id, modifier));
            }
            if let Some(body) = func.body
                && body.stmts.iter().any(|stmt| checker.visit_stmt(stmt).is_break())
            {
                return;
            }
        }

        ctx.emit(&LOCKED_ETHER, contract.name.span);
    }
}

/// True if invoking `func` always reverts, through its body or an attached modifier (one that
/// reverts before its first `_` or after its last one).
fn always_reverts(gcx: Gcx<'_>, contract: ContractId, func: &hir::Function<'_>) -> bool {
    let reverts = |stmts: &[hir::Stmt<'_>]| {
        !block_outcome(gcx, Block { span: Span::DUMMY, stmts }).can_skip_placeholder()
    };
    func.body.is_some_and(|body| reverts(body.stmts))
        || func.modifiers.iter().any(|m| {
            let Some(body) =
                gcx.resolve_modifier_target(contract, m).and_then(|id| gcx.hir.function(id).body)
            else {
                return false;
            };
            let is_placeholder = |s: &hir::Stmt<'_>| matches!(s.kind, StmtKind::Placeholder);
            let Some(first) = body.stmts.iter().position(is_placeholder) else {
                return reverts(body.stmts);
            };
            let last = body.stmts.iter().rposition(is_placeholder).unwrap();
            reverts(&body.stmts[..first]) || reverts(&body.stmts[last + 1..])
        })
}

/// HIR visitor that short-circuits on the first ETH-sending expression and queues statically
/// resolved callees for transitive exploration by the outer worklist loop.
struct SendChecker<'gcx> {
    gcx: Gcx<'gcx>,
    /// The linted contract, which resolves virtual dispatch.
    contract_id: ContractId,
    worklist: Vec<FunctionId>,
}

impl<'gcx> Visit<'gcx> for SendChecker<'gcx> {
    type BreakValue = ();

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    /// Inline assembly can contain ETH-sending opcodes (`call`, `selfdestruct`, ...): bail
    /// conservatively, as if an exit was found.
    fn visit_stmt(&mut self, stmt: &'gcx hir::Stmt<'gcx>) -> ControlFlow<()> {
        if matches!(stmt.kind, StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_))
        {
            return ControlFlow::Break(());
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'gcx hir::Expr<'gcx>) -> ControlFlow<()> {
        if expr_sends_ether(self.gcx, expr) {
            return ControlFlow::Break(());
        }
        if let ExprKind::Call(callee, ..) = &expr.kind {
            match self.gcx.resolved_expr(callee) {
                Some(Res::Item(ItemId::Function(fid))) => {
                    // `super.f()`, `Base.f()` and `Lib.f()` name one implementation; every other
                    // call dispatches through the leaf's linearization.
                    let direct = matches!(&callee.peel_parens().kind, ExprKind::Member(base, _)
                        if is_builtin(self.gcx, base, sym::super_) || is_contract_cast(self.gcx, base));
                    self.worklist.push(if direct {
                        fid
                    } else {
                        self.gcx.resolve_virtual_function(self.contract_id, fid)
                    });
                }
                // Function-typed variable: the bound target is unknown, treat the call as opaque.
                Some(Res::Item(ItemId::Variable(id)))
                    if matches!(self.gcx.hir.variable(id).ty.kind, TypeKind::Function(_)) =>
                {
                    return ControlFlow::Break(());
                }
                _ => {}
            }
        }
        self.walk_expr(expr)
    }
}

/// True if `expr` unambiguously moves ETH out of the contract: a non-zero `{value: x}` call
/// option, `.transfer`/`.send` with a non-zero amount, low-level `.delegatecall`/`.callcode`
/// (drainable via `selfdestruct`), or the `selfdestruct` builtin. Only literal `0` is treated as
/// a zero amount, and sends targeting this contract's own address are not exits.
fn expr_sends_ether<'gcx>(gcx: Gcx<'gcx>, expr: &'gcx hir::Expr<'gcx>) -> bool {
    let ExprKind::Call(callee, args, opts) = &expr.kind else { return false };
    let callee = callee.peel_parens();
    let receiver = match &callee.kind {
        ExprKind::Member(receiver, _) => Some(receiver),
        _ => None,
    };
    if opts.is_some_and(|opts| {
        opts.args.iter().any(|arg| arg.name.name == sym::value && !is_literal_zero(&arg.value))
    }) && !receiver.is_some_and(|r| is_address_self(gcx, r))
    {
        return true;
    }
    match &callee.kind {
        // Only address-typed receivers can move ETH out: `.transfer`/`.send` on a contract type
        // dispatch to a user-defined member.
        ExprKind::Member(receiver, member)
            if expr_is_address(gcx, receiver) && !is_address_self(gcx, receiver) =>
        {
            match member.name {
                // Single-arg form, to tell it apart from ERC20's 2-arg `transfer`.
                sym::transfer | sym::send => {
                    args.len() == 1 && !args.exprs().next().is_some_and(is_literal_zero)
                }
                kw::Delegatecall | kw::Callcode => true,
                kw::Call | kw::Staticcall => false,
                // Any other member is a `using for` binding: assume the bound library function
                // could move ETH.
                _ => true,
            }
        }
        ExprKind::Ident(_) if gcx.resolved_builtin(callee) == Some(Builtin::Selfdestruct) => {
            // `selfdestruct(self)` burns the balance in place.
            !args.exprs().next().is_some_and(|expr| is_address_self(gcx, expr))
        }
        _ => false,
    }
}
