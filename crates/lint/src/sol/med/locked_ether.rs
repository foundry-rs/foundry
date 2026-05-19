use super::LockedEther;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, ElementaryType, LitKind, StateMutability, Visibility},
    interface::{Symbol, kw, sym},
    sema::{
        builtins::Builtin,
        hir::{
            self, CallArgs, CallArgsKind, ExprKind, FunctionId, FunctionKind, ItemId, Res,
            StmtKind, TypeKind, VariableId, Visit as _,
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

        // Any `Payable` function counts as a receive vector except those that always revert.
        let has_payable_entry = contract.linearized_bases.iter().any(|&cid| {
            hir.contract(cid).all_functions().any(|fid| {
                let f = hir.function(fid);
                f.state_mutability == StateMutability::Payable && !function_always_reverts(hir, f)
            })
        });
        if !has_payable_entry {
            return;
        }

        // Seed entry points; internal helpers are reached transitively by `SendChecker`.
        let mut visited: HashSet<FunctionId> = HashSet::new();
        let mut worklist: Vec<FunctionId> = contract
            .linearized_bases
            .iter()
            .flat_map(|&cid| hir.contract(cid).all_functions())
            .filter(|&fid| is_externally_reachable(hir.function(fid)))
            .collect();

        while let Some(fid) = worklist.pop() {
            if !visited.insert(fid) {
                continue;
            }
            let func = hir.function(fid);

            for modifier in func.modifiers {
                for arg in modifier.args.exprs() {
                    let mut checker = SendChecker {
                        hir,
                        bases: contract.linearized_bases,
                        worklist: &mut worklist,
                        visited: &visited,
                    };
                    if checker.visit_expr(arg).is_break() {
                        return;
                    }
                }
                if let Some(modifier_fid) = modifier.id.as_function() {
                    worklist.push(modifier_fid);
                }
            }

            if let Some(body) = func.body {
                let mut checker = SendChecker {
                    hir,
                    bases: contract.linearized_bases,
                    worklist: &mut worklist,
                    visited: &visited,
                };
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

/// Returns `true` if invoking `func` always reverts, either via its body or an attached modifier.
fn function_always_reverts(hir: &hir::Hir<'_>, func: &hir::Function<'_>) -> bool {
    if func
        .modifiers
        .iter()
        .any(|m| m.id.as_function().is_some_and(|mid| modifier_always_reverts(hir.function(mid))))
    {
        return true;
    }
    func.body.is_some_and(|body| stmts_always_revert(body.stmts))
}

/// Returns `true` if the modifier always reverts: before the first `_`, or after the last one.
fn modifier_always_reverts(modifier: &hir::Function<'_>) -> bool {
    let Some(body) = modifier.body else { return false };
    let Some(first) = body.stmts.iter().position(|s| matches!(s.kind, StmtKind::Placeholder))
    else {
        return stmts_always_revert(body.stmts);
    };
    let last = body.stmts.iter().rposition(|s| matches!(s.kind, StmtKind::Placeholder)).unwrap();
    stmts_always_revert(&body.stmts[..first]) || stmts_always_revert(&body.stmts[last + 1..])
}

fn stmts_always_revert(stmts: &[hir::Stmt<'_>]) -> bool {
    stmts.last().is_some_and(stmt_always_reverts)
}

fn stmt_always_reverts(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Revert(_) => true,
        StmtKind::Expr(expr) => is_unconditional_revert_call(expr),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            stmts_always_revert(block.stmts)
        }
        StmtKind::If(_, t, Some(e)) => stmt_always_reverts(t) && stmt_always_reverts(e),
        _ => false,
    }
}

/// Matches `revert()`/`revert("msg")`, `require(false[, "msg"])`, and `assert(false)`.
fn is_unconditional_revert_call(expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return false };
    let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return false };
    reses.iter().any(|r| match r {
        Res::Builtin(Builtin::Revert | Builtin::RevertMsg) => true,
        Res::Builtin(Builtin::Require | Builtin::RequireMsg | Builtin::Assert) => {
            args.exprs().next().is_some_and(is_literal_false)
        }
        _ => false,
    })
}

fn is_literal_false(expr: &hir::Expr<'_>) -> bool {
    if let ExprKind::Lit(lit) = &expr.peel_parens().kind
        && let LitKind::Bool(b) = &lit.kind
    {
        return !b;
    }
    false
}

/// Returns `true` if `func` is callable from outside the contract.
const fn is_externally_reachable(func: &hir::Function<'_>) -> bool {
    match func.kind {
        FunctionKind::Constructor | FunctionKind::Receive | FunctionKind::Fallback => true,
        FunctionKind::Function => {
            matches!(func.visibility, Visibility::Public | Visibility::External)
        }
        FunctionKind::Modifier => false,
    }
}

/// HIR visitor that short-circuits on the first ETH-sending expression and queues
/// internally-resolved callees for transitive exploration by the outer worklist loop.
struct SendChecker<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    bases: &'a [hir::ContractId],
    worklist: &'a mut Vec<FunctionId>,
    visited: &'a HashSet<FunctionId>,
}

impl<'hir> SendChecker<'_, 'hir> {
    /// Queues the overload of `member` actually invoked on `receiver`.
    fn queue_member_callee(
        &mut self,
        receiver: &hir::Expr<'_>,
        member: solar::interface::Ident,
        args: &CallArgs<'_>,
    ) {
        let ExprKind::Ident(reses) = &receiver.peel_parens().kind else { return };
        for res in *reses {
            match res {
                Res::Builtin(Builtin::Super) => {
                    self.queue_resolved(&self.bases[1..], member.name, args);
                }
                Res::Builtin(Builtin::This) => {
                    self.queue_resolved(self.bases, member.name, args);
                }
                Res::Item(ItemId::Contract(cid)) => {
                    self.queue_resolved(std::slice::from_ref(cid), member.name, args);
                }
                _ => {}
            }
        }
    }

    /// Queues arity-matching overloads of `name` from the most-derived contract that defines any.
    fn queue_resolved(
        &mut self,
        contracts: &[hir::ContractId],
        name: solar::interface::Symbol,
        args: &CallArgs<'_>,
    ) {
        for &cid in contracts {
            let mut found = false;
            for fid in self.hir.contract(cid).all_functions() {
                let func = self.hir.function(fid);
                if func.name.is_some_and(|n| n.name == name)
                    && args_match(self.hir, args, func.parameters)
                {
                    found = true;
                    if !self.visited.contains(&fid) {
                        self.worklist.push(fid);
                    }
                }
            }
            if found {
                return;
            }
        }
    }
}

/// Returns `true` if `args` can target `params` by arity, and named-arg names map to params.
fn args_match(hir: &hir::Hir<'_>, args: &CallArgs<'_>, params: &[VariableId]) -> bool {
    if args.len() != params.len() {
        return false;
    }
    match &args.kind {
        CallArgsKind::Unnamed(_) => true,
        CallArgsKind::Named(named) => named.iter().all(|arg| {
            params.iter().any(|&p| hir.variable(p).name.is_some_and(|n| n.name == arg.name.name))
        }),
    }
}

impl<'hir> hir::Visit<'hir> for SendChecker<'_, 'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    /// Inline assembly is lowered to `StmtKind::Err` by Solar; we cannot soundly inspect it
    /// for ETH-sending opcodes (`call`, `selfdestruct`, ...). Bail conservatively to avoid
    /// false positives on contracts whose only exit lives in assembly. Reusing `Break(())`
    /// here is intentional: the outer loop treats it the same as "found an exit" — skip
    /// the warning for this contract.
    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        if matches!(stmt.kind, StmtKind::Err(_)) {
            return ControlFlow::Break(());
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if expr_sends_ether(self.hir, expr) {
            return ControlFlow::Break(());
        }

        // Queue calls whose callee resolves statically to a `FunctionId`.
        if let ExprKind::Call(callee, args, _) = &expr.kind {
            match &callee.peel_parens().kind {
                ExprKind::Ident(reses) => {
                    for res in *reses {
                        if let Res::Item(ItemId::Function(fid)) = res
                            && !self.visited.contains(fid)
                            && args_match(self.hir, args, self.hir.function(*fid).parameters)
                        {
                            self.worklist.push(*fid);
                        }
                    }
                }
                ExprKind::Member(receiver, member) => {
                    self.queue_member_callee(receiver, *member, args);
                }
                _ => {}
            }
        }

        self.walk_expr(expr)
    }
}

/// Returns `true` if `expr` unambiguously moves ETH out of the contract: a non-zero `{value: x}`
/// call option, `.transfer`/`.send` with a non-zero amount, low-level `.delegatecall`/`.callcode`
/// (drainable via `selfdestruct`), or the `selfdestruct` builtin. Only literal `0` is treated as
/// a zero amount; any other expression is assumed non-zero.
fn expr_sends_ether(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
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
        ExprKind::Member(receiver, member) => {
            // Only address-typed receivers can move ETH via these members.
            if !receiver_is_address(hir, receiver) {
                return false;
            }
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
        ExprKind::Ident(reses)
            if reses.iter().any(|r| matches!(r, Res::Builtin(Builtin::Selfdestruct))) =>
        {
            return true;
        }
        _ => {}
    }

    false
}

/// Returns `true` if `expr` is statically known to be an `address`/`address payable` value.
/// Unknown types return `false` so userland members named like `.transfer` aren't taken for exits.
fn receiver_is_address(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Payable(_) => true,
        ExprKind::Lit(lit) => matches!(lit.kind, LitKind::Address(_)),
        // `address(x)` cast.
        ExprKind::Call(callee, ..) => matches!(
            &callee.peel_parens().kind,
            ExprKind::Type(hir::Type {
                kind: TypeKind::Elementary(ElementaryType::Address(_)),
                ..
            })
        ),
        ExprKind::Member(base, member) => is_address_builtin_member(base, member.name),
        ExprKind::Ident(reses) => reses.iter().any(|res| match res {
            Res::Item(ItemId::Variable(id)) => is_address_type(&hir.variable(*id).ty),
            _ => false,
        }),
        _ => false,
    }
}

/// `msg.sender`, `tx.origin`, `block.coinbase`.
fn is_address_builtin_member(base: &hir::Expr<'_>, member: Symbol) -> bool {
    let ExprKind::Ident(reses) = &base.peel_parens().kind else { return false };
    reses.iter().any(|res| {
        let Res::Builtin(builtin) = res else { return false };
        matches!(
            (builtin.name(), member),
            (sym::msg, sym::sender) | (sym::tx, kw::Origin) | (sym::block, kw::Coinbase)
        )
    })
}

const fn is_address_type(ty: &hir::Type<'_>) -> bool {
    matches!(ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
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
