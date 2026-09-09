use super::LockedEther;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            block_outcome, expr_is_address, is_address_self, is_builtin, is_contract_cast,
            is_literal_zero,
        },
    },
};
use solar::{
    ast::{ContractKind, StateMutability, Visibility},
    interface::{Span, kw, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, Block, ContractId, ExprKind, FunctionId, FunctionKind, ItemId, Res, StmtKind,
            TypeKind, Visit,
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
            func.state_mutability == StateMutability::Payable && !always_reverts(gcx, func)
        };
        // Runtime entries and the constructor are separate inflow channels: only the leaf's own
        // constructor receives deployment value, and it has no runtime exit path.
        let entries = runtime_dispatch_surface(gcx, contract.linearized_bases);
        if !entries.iter().any(|&fid| receives(fid)) && !contract.ctor.is_some_and(receives) {
            return;
        }

        // Explore the runtime entries and, transitively, the helpers and modifiers they reach.
        // Constructor bodies are excluded so their exits don't count.
        let mut visited = HashSet::new();
        let mut checker = SendChecker { gcx, bases: contract.linearized_bases, worklist: entries };
        while let Some(fid) = checker.worklist.pop() {
            let func = gcx.hir.function(fid);
            // Any ETH movement inside an always-reverting function rolls back.
            if !visited.insert(fid) || always_reverts(gcx, func) {
                continue;
            }
            for modifier in func.modifiers {
                if checker.visit_call_args(&modifier.args).is_break() {
                    return;
                }
                checker.worklist.extend(modifier.id.as_function());
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
fn always_reverts(gcx: Gcx<'_>, func: &hir::Function<'_>) -> bool {
    let reverts = |stmts: &[hir::Stmt<'_>]| {
        !block_outcome(Block { span: Span::DUMMY, stmts }).can_skip_placeholder()
    };
    func.body.is_some_and(|body| reverts(body.stmts))
        || func.modifiers.iter().any(|m| {
            let Some(body) = m.id.as_function().and_then(|id| gcx.hir.function(id).body) else {
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

/// Runtime entry points reachable on the deployed contract: the most-derived implementation of
/// each `(name, parameter types)` plus the most-derived `receive` / `fallback`. `bases` must be
/// the C3 linearization (leaf first). Constructors and modifiers are excluded.
fn runtime_dispatch_surface<'gcx>(gcx: Gcx<'gcx>, bases: &[ContractId]) -> Vec<FunctionId> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for fid in bases.iter().flat_map(|&cid| gcx.hir.contract(cid).all_functions()) {
        let func = gcx.hir.function(fid);
        let params = match func.kind {
            FunctionKind::Function
                if matches!(func.visibility, Visibility::Public | Visibility::External) =>
            {
                gcx.item_parameter_types(fid)
            }
            FunctionKind::Receive | FunctionKind::Fallback => &[],
            _ => continue,
        };
        if seen.insert((func.kind, func.name.map(|n| n.name), params)) {
            out.push(fid);
        }
    }
    out
}

/// HIR visitor that short-circuits on the first ETH-sending expression and queues statically
/// resolved callees for transitive exploration by the outer worklist loop.
struct SendChecker<'gcx> {
    gcx: Gcx<'gcx>,
    /// Linearization of the linted contract, which resolves virtual dispatch.
    bases: &'gcx [ContractId],
    worklist: Vec<FunctionId>,
}

impl SendChecker<'_> {
    /// Redirects `fid` to the linted contract's most-derived override of the same `(name,
    /// parameter types)`. Functions not inheritable from it (free functions, library helpers,
    /// private functions, constructors and modifiers) are returned as-is.
    fn resolve_virtual(&self, fid: FunctionId) -> FunctionId {
        let func = self.gcx.hir.function(fid);
        if !func.contract.is_some_and(|origin| self.bases.contains(&origin))
            || func.visibility == Visibility::Private
            || func.kind != FunctionKind::Function
        {
            return fid;
        }
        let Some(name) = func.name else { return fid };
        let params = self.gcx.item_parameter_types(fid);
        self.bases
            .iter()
            .flat_map(|&cid| self.gcx.hir.contract(cid).functions())
            .find(|&candidate| {
                let c = self.gcx.hir.function(candidate);
                c.name.is_some_and(|n| n.name == name.name)
                    && self.gcx.item_parameter_types(candidate) == params
            })
            .unwrap_or(fid)
    }
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
                        if is_builtin(base, sym::super_) || is_contract_cast(base));
                    self.worklist.push(if direct { fid } else { self.resolve_virtual(fid) });
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
    }) && !receiver.is_some_and(|r| is_address_self(r))
    {
        return true;
    }
    match &callee.kind {
        // Only address-typed receivers can move ETH out: `.transfer`/`.send` on a contract type
        // dispatch to a user-defined member.
        ExprKind::Member(receiver, member)
            if expr_is_address(gcx, receiver) && !is_address_self(receiver) =>
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
        ExprKind::Ident(reses)
            if reses.iter().any(|r| matches!(r, Res::Builtin(Builtin::Selfdestruct))) =>
        {
            // `selfdestruct(self)` burns the balance in place.
            !args.exprs().next().is_some_and(is_address_self)
        }
        _ => false,
    }
}
