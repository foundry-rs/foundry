use super::ArbitrarySendErc20;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    interface::{Span, data_structures::Never, kw, sym},
    sema::hir::{
        self, ContractKind, ElementaryType, ExprKind, ItemId, LoopSource, Res, StmtKind, StructId,
        TypeKind, Visit,
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    ARBITRARY_SEND_ERC20,
    Severity::High,
    "arbitrary-send-erc20",
    "`transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`"
);

impl<'hir> LateLintPass<'hir> for ArbitrarySendErc20 {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if !func.kind.is_function()
            || matches!(
                func.state_mutability,
                ast::StateMutability::Pure | ast::StateMutability::View
            )
        {
            return;
        }
        // Library functions typically forward `from` from their caller; flag at the call
        // site (in user contracts) instead, where the trust boundary actually lives.
        if func.contract.is_some_and(|cid| hir.contract(cid).kind == ContractKind::Library) {
            return;
        }
        let Some(body) = func.body else { return };

        let has_solady_lib = has_solady_safe_transfer_lib(hir);
        let mut a = Analyzer::new(hir, has_solady_lib);
        for m in func.modifiers {
            collect_modifier_safety(hir, has_solady_lib, m, &mut a.safe_vars);
        }
        for stmt in body.stmts {
            let _ = a.visit_stmt(stmt);
        }
        for span in a.hits {
            ctx.emit(&ARBITRARY_SEND_ERC20, span);
        }
    }
}

/// `(token, owner)` of an EIP-2612 permit recorded earlier on the current path with
/// `spender == address(this)`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PermitRecord {
    token: hir::VariableId,
    owner: hir::VariableId,
}

/// Outstanding EIP-3156 repayment licensed by a prior `onFlashLoan` call.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PendingRepayment {
    receiver: hir::VariableId,
    token: hir::VariableId,
    amount: hir::VariableId,
    fee: hir::VariableId,
}

struct Analyzer<'hir> {
    hir: &'hir hir::Hir<'hir>,
    /// Variables proven safe (equal to `msg.sender` or `address(this)`) on this path.
    /// Function-locals and `immutable`/`constant` state vars only — mutable storage may be
    /// rewritten between the check and the sink.
    safe_vars: HashSet<hir::VariableId>,
    /// Permits seen earlier on this path. Path-sensitive and killed on token/owner reassignment.
    permits: HashSet<PermitRecord>,
    /// Pending flash-loan repayments. Path-sensitive; killed on reassignment of any
    /// referenced var; consumed once by a matching sink.
    repayments: HashSet<PendingRepayment>,
    /// Gates the `using ... for address` sink branch on a `SafeTransferLib` being present.
    has_solady_lib: bool,
    hits: Vec<Span>,
}

#[derive(Clone)]
struct FlowState {
    safe_vars: HashSet<hir::VariableId>,
    permits: HashSet<PermitRecord>,
    repayments: HashSet<PendingRepayment>,
}

impl FlowState {
    fn intersection(a: &Self, b: &Self) -> Self {
        Self {
            safe_vars: a.safe_vars.intersection(&b.safe_vars).copied().collect(),
            permits: a.permits.intersection(&b.permits).copied().collect(),
            repayments: a.repayments.intersection(&b.repayments).copied().collect(),
        }
    }

    fn intersection_all(mut states: impl Iterator<Item = Self>) -> Self {
        let mut out = states.next().unwrap_or_else(|| Self {
            safe_vars: HashSet::new(),
            permits: HashSet::new(),
            repayments: HashSet::new(),
        });
        for state in states {
            out = Self::intersection(&out, &state);
        }
        out
    }
}

/// Recursion budget for `_msgSender()`-style helper chains.
const HELPER_DEPTH: u8 = 3;

impl<'hir> Analyzer<'hir> {
    fn new(hir: &'hir hir::Hir<'hir>, has_solady_lib: bool) -> Self {
        Self {
            hir,
            safe_vars: HashSet::new(),
            permits: HashSet::new(),
            repayments: HashSet::new(),
            has_solady_lib,
            hits: Vec::new(),
        }
    }

    fn snapshot(&self) -> FlowState {
        FlowState {
            safe_vars: self.safe_vars.clone(),
            permits: self.permits.clone(),
            repayments: self.repayments.clone(),
        }
    }

    fn restore(&mut self, state: FlowState) {
        self.safe_vars = state.safe_vars;
        self.permits = state.permits;
        self.repayments = state.repayments;
    }

    fn is_safe(&self, expr: &hir::Expr<'_>) -> bool {
        self.is_safe_inner(expr, HELPER_DEPTH)
    }

    fn is_safe_inner(&self, expr: &hir::Expr<'_>, depth: u8) -> bool {
        match &expr.peel_parens().kind {
            ExprKind::Member(base, ident) if ident.name == sym::sender => {
                is_builtin(base, sym::msg)
            }
            ExprKind::Ident(_) if is_builtin(expr, sym::this) => true,
            ExprKind::Ident(reses) => reses.iter().any(
                |r| matches!(r, Res::Item(ItemId::Variable(vid)) if self.safe_vars.contains(vid)),
            ),
            ExprKind::Call(callee, args, _) if is_address_cast(callee) => {
                args.exprs().next().is_some_and(|e| self.is_safe_inner(e, depth))
            }
            ExprKind::Payable(inner) => self.is_safe_inner(inner, depth),
            ExprKind::Ternary(_, t, f) => {
                self.is_safe_inner(t, depth) && self.is_safe_inner(f, depth)
            }
            // No-arg helper whose body is `return X;` with `X` statically safe (e.g. `_msgSender`).
            ExprKind::Call(callee, args, _)
                if depth > 0
                    && args.exprs().next().is_none()
                    && self.callee_returns_safe(callee, depth - 1) =>
            {
                true
            }
            _ => false,
        }
    }

    fn callee_returns_safe(&self, callee: &hir::Expr<'_>, depth: u8) -> bool {
        let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return false };
        reses.iter().any(|r| match r {
            Res::Item(ItemId::Function(fid)) => {
                let f = self.hir.function(*fid);
                let Some(body) = f.body else { return false };
                f.parameters.is_empty()
                    && body.stmts.len() == 1
                    && matches!(
                        &body.stmts[0].kind,
                        StmtKind::Return(Some(e)) if self.is_safe_inner(e, depth)
                    )
            }
            _ => false,
        })
    }

    /// Updates `safe_vars` only; permit kills are caller-owned (state-var writes skip this).
    fn assign(&mut self, target: hir::VariableId, rhs: &hir::Expr<'_>) {
        if self.is_safe(rhs) {
            self.safe_vars.insert(target);
        } else {
            self.safe_vars.remove(&target);
        }
    }

    /// Drops permits referencing `target`. Sound for any variable kind.
    fn kill_permits_for(&mut self, target: hir::VariableId) {
        self.permits.retain(|p| p.token != target && p.owner != target);
    }

    fn permit_covers(&self, token: Option<hir::VariableId>, from: &hir::Expr<'_>) -> bool {
        let (Some(token), Some(owner)) = (token, underlying_var(from)) else { return false };
        self.permits.contains(&PermitRecord { token, owner })
    }

    /// Drops pending repayments referencing `target`.
    fn kill_repayments_for(&mut self, target: hir::VariableId) {
        self.repayments.retain(|r| {
            r.receiver != target && r.token != target && r.amount != target && r.fee != target
        });
    }

    /// Matches `from`/`token` plus a sink call with `to == address(this)` and amount
    /// `amount + fee` against a pending repayment, consuming it on hit.
    fn consume_repayment(
        &mut self,
        call_expr: &hir::Expr<'_>,
        from: &hir::Expr<'_>,
        token: Option<hir::VariableId>,
    ) -> bool {
        let Some(from_v) = underlying_var(from) else { return false };
        let Some(token_v) = token else { return false };
        let ExprKind::Call(_, args, _) = &call_expr.kind else { return false };
        // Pick `to` and `amount` from whichever sink shape (3-arg member / 4-arg library).
        let (to_arg, amount_arg) = if let Some(a) =
            canonical_args(args.kind, &[&["from"], &["to"], &["value", "amount"]])
        {
            (a[1], a[2])
        } else if let Some(a) =
            canonical_args(args.kind, &[&["token"], &["from"], &["to"], &["value", "amount"]])
        {
            (a[2], a[3])
        } else {
            return false;
        };
        if !is_address_self(to_arg) {
            return false;
        }
        let matched = self.repayments.iter().copied().find(|r| {
            r.receiver == from_v
                && r.token == token_v
                && is_amount_plus_fee(amount_arg, r.amount, r.fee)
        });
        if let Some(rep) = matched {
            self.repayments.remove(&rep);
            true
        } else {
            false
        }
    }

    /// Records vars proven equal to a safe origin. Handles `==`/`!=`, `&&`/`||` (via De Morgan)
    /// and `!`. Disjunctions are skipped — they don't establish a must-fact.
    fn add_facts(&mut self, pred: &hir::Expr<'_>, negate: bool) {
        match &pred.peel_parens().kind {
            ExprKind::Binary(lhs, op, rhs) => {
                let (eq, and) = if negate {
                    (ast::BinOpKind::Ne, ast::BinOpKind::Or)
                } else {
                    (ast::BinOpKind::Eq, ast::BinOpKind::And)
                };
                if op.kind == and {
                    self.add_facts(lhs, negate);
                    self.add_facts(rhs, negate);
                } else if op.kind == eq {
                    for (a, b) in [(lhs, rhs), (rhs, lhs)] {
                        if self.is_safe(a)
                            && let Some(v) = underlying_var(b)
                            && self.is_safe_target(v)
                        {
                            self.safe_vars.insert(v);
                        }
                    }
                }
            }
            ExprKind::Unary(op, inner) if matches!(op.kind, ast::UnOpKind::Not) => {
                self.add_facts(inner, !negate);
            }
            _ => {}
        }
    }

    fn is_safe_target(&self, v: hir::VariableId) -> bool {
        let var = self.hir.variable(v);
        !var.kind.is_state() || var.is_immutable() || var.is_constant()
    }

    /// Handles single-var and tuple LHS; tuple slots align with a tuple-literal RHS.
    fn handle_assign(&mut self, lhs: &hir::Expr<'_>, rhs: &hir::Expr<'_>) {
        let lhs = lhs.peel_parens();
        if let ExprKind::Tuple(lhs_elems) = &lhs.kind {
            let rhs_elems = match &rhs.peel_parens().kind {
                ExprKind::Tuple(r) => Some(*r),
                _ => None,
            };
            for (i, lhs_elem) in lhs_elems.iter().enumerate() {
                if let Some(lhs_expr) = lhs_elem {
                    let rhs_expr = rhs_elems.and_then(|r| r.get(i).copied()).flatten();
                    self.assign_one(lhs_expr, rhs_expr);
                }
            }
        } else {
            self.assign_one(lhs, Some(rhs));
        }
    }

    /// `rhs == None` (unknown slot) drops the target's safe-fact.
    fn assign_one(&mut self, lhs: &hir::Expr<'_>, rhs: Option<&hir::Expr<'_>>) {
        let Some(target) = underlying_var(lhs) else { return };
        self.kill_permits_for(target);
        self.kill_repayments_for(target);
        // Drop any prior safe-fact before the state-var bail-out.
        self.safe_vars.remove(&target);
        if self.hir.variable(target).kind.is_state() {
            return;
        }
        if rhs.is_some_and(|r| self.is_safe(r)) {
            self.safe_vars.insert(target);
        }
    }

    /// Visits a body that may execute zero times or out-of-line (loops, try clauses):
    /// in-body kills survive, in-body additions don't.
    fn visit_isolated(&mut self, stmts: &'hir [hir::Stmt<'hir>]) {
        let mut exits = vec![self.snapshot()];
        if let Some(fallthrough) = self.visit_stmts_until_loop_exit(stmts, &mut exits) {
            exits.push(fallthrough);
        }
        self.restore(FlowState::intersection_all(exits.into_iter()));
    }

    fn visit_stmts_until_loop_exit(
        &mut self,
        stmts: &'hir [hir::Stmt<'hir>],
        exits: &mut Vec<FlowState>,
    ) -> Option<FlowState> {
        for stmt in stmts {
            self.visit_stmt_until_loop_exit(stmt, exits)?;
        }
        Some(self.snapshot())
    }

    fn visit_stmt_until_loop_exit(
        &mut self,
        stmt: &'hir hir::Stmt<'hir>,
        exits: &mut Vec<FlowState>,
    ) -> Option<()> {
        match &stmt.kind {
            StmtKind::Break | StmtKind::Continue => {
                exits.push(self.snapshot());
                None
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                let state = self.visit_stmts_until_loop_exit(block.stmts, exits)?;
                self.restore(state);
                Some(())
            }
            StmtKind::If(cond, then, else_) => {
                let _ = self.visit_expr(cond);
                let baseline = self.snapshot();

                self.add_facts(cond, false);
                let then_fallthrough = self
                    .visit_stmt_until_loop_exit(then, exits)
                    .and_then(|_| (!branch_always_exits(then)).then(|| self.snapshot()));

                self.restore(baseline);
                self.add_facts(cond, true);
                let else_fallthrough = match else_ {
                    Some(else_stmt) => self
                        .visit_stmt_until_loop_exit(else_stmt, exits)
                        .and_then(|_| (!branch_always_exits(else_stmt)).then(|| self.snapshot())),
                    None => Some(self.snapshot()),
                };

                match (then_fallthrough, else_fallthrough) {
                    (Some(then_state), Some(else_state)) => {
                        self.restore(FlowState::intersection(&then_state, &else_state));
                        Some(())
                    }
                    (Some(state), None) | (None, Some(state)) => {
                        self.restore(state);
                        Some(())
                    }
                    (None, None) => None,
                }
            }
            // Nested loops own their own `break` / `continue`; they do not exit this isolated body.
            StmtKind::Loop(..) => {
                let _ = self.visit_stmt(stmt);
                Some(())
            }
            _ => {
                let _ = self.visit_stmt(stmt);
                (!branch_always_exits(stmt)).then_some(())
            }
        }
    }
}

impl<'hir> hir::Visit<'hir> for Analyzer<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match &stmt.kind {
            // Branch-sensitive join: positive facts flow into `then`; if the then-branch
            // always exits, negated facts (`!cond`) flow into the fall-through.
            StmtKind::If(cond, then, else_) => {
                let _ = self.visit_expr(cond);

                let baseline_safe = self.safe_vars.clone();
                let baseline_permits = self.permits.clone();
                let baseline_repayments = self.repayments.clone();
                self.add_facts(cond, false);
                let _ = self.visit_stmt(then);
                let then_exits = branch_always_exits(then);
                let after_then_safe = std::mem::replace(&mut self.safe_vars, baseline_safe);
                let after_then_permits = std::mem::replace(&mut self.permits, baseline_permits);
                let after_then_repayments =
                    std::mem::replace(&mut self.repayments, baseline_repayments);

                // Both the explicit `else` body and the implicit fall-through inherit `!cond`.
                let (after_else_safe, after_else_permits, after_else_repayments, else_exits) =
                    match else_ {
                        Some(e) => {
                            self.add_facts(cond, true);
                            let _ = self.visit_stmt(e);
                            (
                                std::mem::take(&mut self.safe_vars),
                                std::mem::take(&mut self.permits),
                                std::mem::take(&mut self.repayments),
                                branch_always_exits(e),
                            )
                        }
                        None => {
                            self.add_facts(cond, true);
                            (
                                std::mem::take(&mut self.safe_vars),
                                std::mem::take(&mut self.permits),
                                std::mem::take(&mut self.repayments),
                                false,
                            )
                        }
                    };

                let (sv, pm, rp) = match (then_exits, else_exits) {
                    (true, true) => (
                        after_then_safe.union(&after_else_safe).copied().collect(),
                        after_then_permits.union(&after_else_permits).copied().collect(),
                        after_then_repayments.union(&after_else_repayments).copied().collect(),
                    ),
                    (true, false) => (after_else_safe, after_else_permits, after_else_repayments),
                    (false, true) => (after_then_safe, after_then_permits, after_then_repayments),
                    (false, false) => (
                        after_then_safe.intersection(&after_else_safe).copied().collect(),
                        after_then_permits.intersection(&after_else_permits).copied().collect(),
                        after_then_repayments
                            .intersection(&after_else_repayments)
                            .copied()
                            .collect(),
                    ),
                };
                self.safe_vars = sv;
                self.permits = pm;
                self.repayments = rp;
                return ControlFlow::Continue(());
            }

            StmtKind::Loop(block, source) => {
                // `do-while` runs the body at least once, so facts flow out — unless a user
                // `break`/`continue` can skip later assignments.
                if matches!(source, LoopSource::DoWhile)
                    && !body_has_break_or_continue(do_while_user_stmts(block.stmts))
                {
                    for s in block.stmts {
                        let _ = self.visit_stmt(s);
                    }
                } else {
                    self.visit_isolated(block.stmts);
                }
                return ControlFlow::Continue(());
            }
            StmtKind::Try(t) => {
                let _ = self.visit_expr(&t.expr);
                for clause in t.clauses {
                    self.visit_isolated(clause.block.stmts);
                }
                return ControlFlow::Continue(());
            }

            StmtKind::DeclSingle(vid) if is_address(self.hir, *vid) => {
                if let Some(init) = self.hir.variable(*vid).initializer {
                    self.assign(*vid, init);
                }
            }

            // Position-aligned propagation from a tuple literal RHS.
            StmtKind::DeclMulti(vars, init) => {
                if let ExprKind::Tuple(rhs) = &init.peel_parens().kind {
                    for (lhs, rhs) in vars.iter().zip(rhs.iter()) {
                        if let (Some(vid), Some(expr)) = (lhs, rhs)
                            && is_address(self.hir, *vid)
                        {
                            self.assign(*vid, expr);
                        }
                    }
                }
            }

            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        // Short-circuit `&&` / `||`: `rhs` may not execute, so its state mutations must
        // not survive the expression. `lhs` facts flow into `rhs` only. `hits` persist —
        // a sink in `rhs` is still reported.
        if let ExprKind::Binary(lhs, op, rhs) = &expr.kind
            && matches!(op.kind, ast::BinOpKind::And | ast::BinOpKind::Or)
        {
            let _ = self.visit_expr(lhs);
            let negate = matches!(op.kind, ast::BinOpKind::Or);
            let skipped_rhs = self.snapshot();
            self.add_facts(lhs, negate);
            let result = self.visit_expr(rhs);
            let ran_rhs = self.snapshot();
            self.restore(FlowState::intersection(&skipped_rhs, &ran_rhs));
            return result;
        }

        match &expr.kind {
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                // Walk the predicate before recording its facts so sinks inside the
                // predicate see the pre-guard state.
                let result = self.walk_expr(expr);
                if let Some(cond) = args.exprs().next() {
                    self.add_facts(cond, false);
                }
                return result;
            }
            ExprKind::Call(..) => {
                // EIP-3156: a matching repayment sink consumes the recorded obligation.
                if let Some(rep) = match_flash_loan_call(self.hir, expr) {
                    self.repayments.insert(rep);
                } else if let Some(p) = match_permit_call(expr) {
                    self.permits.insert(p);
                } else if let Some((from, token)) = match_sink(self.hir, self.has_solady_lib, expr)
                    && !self.is_safe(from)
                    && !self.permit_covers(token, from)
                    && !self.consume_repayment(expr, from, token)
                {
                    self.hits.push(expr.span);
                }
            }
            ExprKind::Assign(lhs, _, rhs) => self.handle_assign(lhs, rhs),
            // `delete x` resets `x`; treat as unknown reassignment.
            ExprKind::Delete(target) => self.assign_one(target.peel_parens(), None),
            _ => {}
        }
        self.walk_expr(expr)
    }
}

/// EIP-3156 `onFlashLoan(address,address,uint256,uint256,bytes) returns (bytes32)`. Only
/// returns a record when the receiver type declares the exact sig and every tracked arg
/// resolves to a `VariableId`; literal args yield `None`.
fn match_flash_loan_call(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<PendingRepayment> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
    if ident.name.as_str() != "onFlashLoan" {
        return None;
    }
    let args =
        canonical_args(args.kind, &[&["initiator"], &["token"], &["amount"], &["fee"], &["data"]])?;
    let cid = receiver_contract_id(hir, recv)?;
    if !contract_has_function(
        hir,
        cid,
        "onFlashLoan",
        &["address", "address", "uint256", "uint256", "bytes"],
        &["bytes32"],
    ) {
        return None;
    }
    Some(PendingRepayment {
        receiver: underlying_var(recv)?,
        token: underlying_var(args[1])?,
        amount: underlying_var(args[2])?,
        fee: underlying_var(args[3])?,
    })
}

/// True when `expr` is `amount + fee` or `fee + amount`, parens-tolerant.
fn is_amount_plus_fee(expr: &hir::Expr<'_>, amount: hir::VariableId, fee: hir::VariableId) -> bool {
    let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind else { return false };
    if !matches!(op.kind, ast::BinOpKind::Add) {
        return false;
    }
    let a = underlying_var(lhs);
    let b = underlying_var(rhs);
    (a == Some(amount) && b == Some(fee)) || (a == Some(fee) && b == Some(amount))
}

/// Matches an ERC20-like transfer sink. Returns `(from_arg, token_var)` where `token_var` is
/// the receiver's underlying variable id when available (used for permit correlation).
///
/// Recognised:
/// - `recv.transferFrom(from, to, amt)` / `recv.safeTransferFrom(from, to, amt)` where `recv` is
///   typed as a contract declaring ERC20's `transferFrom(address,address,uint256)→bool` (ERC721's
///   same-named, no-return overload is excluded).
/// - `Lib.safeTransferFrom(token, from, to, amt)` library form.
fn match_sink<'hir>(
    hir: &'hir hir::Hir<'hir>,
    has_solady_lib: bool,
    expr: &'hir hir::Expr<'hir>,
) -> Option<(&'hir hir::Expr<'hir>, Option<hir::VariableId>)> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
    let name = ident.name.as_str();

    if (name == "transferFrom" || name == "safeTransferFrom")
        && let Some(canonical) =
            canonical_args(args.kind, &[&["from"], &["to"], &["value", "amount"]])
    {
        // Contract-typed receiver: must actually declare ERC20's `transferFrom`.
        if let Some(cid) = receiver_contract_id(hir, recv)
            && contract_has_function(
                hir,
                cid,
                "transferFrom",
                &["address", "address", "uint256"],
                &["bool"],
            )
        {
            return Some((canonical[0], underlying_var(recv)));
        }
        // `addr.safeTransferFrom(...)` via `using SafeTransferLib for address`. HIR doesn't
        // expose `using-for` bindings, so we proxy by requiring `SafeTransferLib` to be
        // declared in the compiled sources.
        if name == "safeTransferFrom" && has_solady_lib && expr_is_address(hir, recv) {
            return Some((canonical[0], underlying_var(recv)));
        }
    }

    if name == "safeTransferFrom"
        && let Some(canonical) =
            canonical_args(args.kind, &[&["token"], &["from"], &["to"], &["value", "amount"]])
        && let Some(cid) = receiver_contract_id(hir, recv)
        && hir.contract(cid).kind == ContractKind::Library
        && library_has_safe_transfer_from(hir, cid)
    {
        return Some((canonical[1], underlying_var(canonical[0])));
    }

    None
}

/// Matches `token.permit(owner, address(this), value, deadline, v, r, s)` (EIP-2612 shape).
/// Only `spender == address(this)` permits are recorded; others can't suppress a sink.
fn match_permit_call(expr: &hir::Expr<'_>) -> Option<PermitRecord> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
    if ident.name.as_str() != "permit" {
        return None;
    }
    let args = canonical_args(
        args.kind,
        &[&["owner"], &["spender"], &["value"], &["deadline"], &["v"], &["r"], &["s"]],
    )?;
    if !is_address_self(args[1]) {
        return None;
    }
    Some(PermitRecord { token: underlying_var(recv)?, owner: underlying_var(args[0])? })
}

/// Hoists `require(modParam == msg.sender)` style guards from the modifier prefix (statements
/// before its single top-level `_;`), mapping modifier params back to caller args.
fn collect_modifier_safety(
    hir: &hir::Hir<'_>,
    has_solady_lib: bool,
    invocation: &hir::Modifier<'_>,
    out_safe: &mut HashSet<hir::VariableId>,
) {
    let ItemId::Function(fid) = invocation.id else { return };
    let modifier = hir.function(fid);
    if !matches!(modifier.kind, hir::FunctionKind::Modifier) {
        return;
    }
    let Some(body) = modifier.body else { return };

    // Skip multi-`_` or nested-placeholder modifiers — guard ordering can't be assumed sound.
    if count_placeholders(body.stmts) != 1 {
        return;
    }
    let Some(placeholder_idx) =
        body.stmts.iter().position(|s| matches!(s.kind, StmtKind::Placeholder))
    else {
        return;
    };

    let arg_map: Vec<(hir::VariableId, hir::VariableId)> = invocation
        .args
        .exprs()
        .enumerate()
        .filter_map(|(i, arg)| Some((*modifier.parameters.get(i)?, underlying_var(arg)?)))
        .collect();
    if arg_map.is_empty() {
        return;
    }

    let mut a = Analyzer::new(hir, has_solady_lib);
    for stmt in &body.stmts[..placeholder_idx] {
        let _ = a.visit_stmt(stmt);
    }
    // Skip caller-args that can't carry a safe-fact (mutable storage).
    for (mp, caller) in arg_map {
        if a.safe_vars.contains(&mp) && a.is_safe_target(caller) {
            out_safe.insert(caller);
        }
    }
}

/// Strips the synthesized trailing `if (!cond) break;` from the HIR `do-while` lowering.
fn do_while_user_stmts<'a, 'hir>(stmts: &'a [hir::Stmt<'hir>]) -> &'a [hir::Stmt<'hir>] {
    match stmts.split_last() {
        Some((last, rest)) if is_loop_termination_if(last) => rest,
        _ => stmts,
    }
}

fn is_loop_termination_if(stmt: &hir::Stmt<'_>) -> bool {
    let StmtKind::If(_, then_, else_) = &stmt.kind else { return false };
    is_break_stmt(then_) || else_.as_ref().is_some_and(|e| is_break_stmt(e))
}

fn is_break_stmt(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Break => true,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => {
            b.stmts.len() == 1 && is_break_stmt(&b.stmts[0])
        }
        _ => false,
    }
}

/// `break`/`continue` targeting the current loop (nested loops shadow them).
fn body_has_break_or_continue(stmts: &[hir::Stmt<'_>]) -> bool {
    fn in_stmt(stmt: &hir::Stmt<'_>) -> bool {
        match &stmt.kind {
            StmtKind::Break | StmtKind::Continue => true,
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => body_has_break_or_continue(b.stmts),
            StmtKind::If(_, t, e) => in_stmt(t) || e.as_ref().is_some_and(|s| in_stmt(s)),
            StmtKind::Try(t) => t.clauses.iter().any(|c| body_has_break_or_continue(c.block.stmts)),
            StmtKind::Loop(..) => false,
            _ => false,
        }
    }
    stmts.iter().any(in_stmt)
}

fn count_placeholders(stmts: &[hir::Stmt<'_>]) -> usize {
    fn count_in_stmt(stmt: &hir::Stmt<'_>) -> usize {
        match &stmt.kind {
            StmtKind::Placeholder => 1,
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => count_placeholders(b.stmts),
            StmtKind::If(_, t, e) => count_in_stmt(t) + e.as_ref().map_or(0, |s| count_in_stmt(s)),
            StmtKind::Loop(b, _) => count_placeholders(b.stmts),
            StmtKind::Try(t) => t.clauses.iter().map(|c| count_placeholders(c.block.stmts)).sum(),
            _ => 0,
        }
    }
    stmts.iter().map(count_in_stmt).sum()
}

/// Resolves a `VariableId` for bare idents and `address(...)` / `payable(...)` / parens wraps.
fn underlying_var(expr: &hir::Expr<'_>) -> Option<hir::VariableId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Variable(vid)) => Some(*vid),
            _ => None,
        }),
        ExprKind::Call(callee, args, _) if is_address_cast(callee) => {
            args.exprs().next().and_then(underlying_var)
        }
        ExprKind::Payable(inner) => underlying_var(inner),
        _ => None,
    }
}

/// Resolves the static contract type of `recv`: a contract-typed variable, a direct contract
/// reference (e.g. a library), an `IERC20(addr)` interface wrap, a struct field, or an
/// array/mapping element of contract type.
fn receiver_contract_id(hir: &hir::Hir<'_>, recv: &hir::Expr<'_>) -> Option<hir::ContractId> {
    let recv = recv.peel_parens();
    match &recv.kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Variable(vid)) => match hir.variable(*vid).ty.kind {
                TypeKind::Custom(ItemId::Contract(cid)) => Some(cid),
                _ => None,
            },
            Res::Item(ItemId::Contract(cid)) => Some(*cid),
            _ => None,
        }),
        ExprKind::Call(callee, ..) => match &callee.peel_parens().kind {
            ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
                Res::Item(ItemId::Contract(cid)) => Some(*cid),
                _ => None,
            }),
            _ => None,
        },
        // `cfg.token.transferFrom(...)`.
        ExprKind::Member(base, ident) => {
            let sid = struct_of(hir, base)?;
            let strukt = hir.strukt(sid);
            strukt.fields.iter().find_map(|fid| {
                let v = hir.variable(*fid);
                if v.name.is_some_and(|n| n.as_str() == ident.as_str())
                    && let TypeKind::Custom(ItemId::Contract(cid)) = v.ty.kind
                {
                    Some(cid)
                } else {
                    None
                }
            })
        }
        // `tokens[i].transferFrom(...)`, `tokenMap[k].transferFrom(...)`. Direct-ident bases only.
        ExprKind::Index(base, _) => {
            let ExprKind::Ident(reses) = &base.peel_parens().kind else { return None };
            let var = reses.iter().find_map(|r| match r {
                Res::Item(ItemId::Variable(vid)) => Some(hir.variable(*vid)),
                _ => None,
            })?;
            let element = match &var.ty.kind {
                TypeKind::Array(arr) => &arr.element.kind,
                TypeKind::Mapping(m) => &m.value.kind,
                _ => return None,
            };
            match element {
                TypeKind::Custom(ItemId::Contract(cid)) => Some(*cid),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Returns the [`StructId`] of `expr` when it is a (possibly chained) struct value.
fn struct_of(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<StructId> {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Variable(vid)) => match hir.variable(*vid).ty.kind {
                TypeKind::Custom(ItemId::Struct(sid)) => Some(sid),
                _ => None,
            },
            _ => None,
        }),
        ExprKind::Member(inner, member) => {
            let sid = struct_of(hir, inner)?;
            let strukt = hir.strukt(sid);
            strukt.fields.iter().find_map(|fid| {
                let v = hir.variable(*fid);
                if v.name.is_some_and(|n| n.as_str() == member.as_str())
                    && let TypeKind::Custom(ItemId::Struct(inner_sid)) = v.ty.kind
                {
                    Some(inner_sid)
                } else {
                    None
                }
            })
        }
        _ => None,
    }
}

/// Whether the sources declare a Solady-shaped `SafeTransferLib` library.
fn has_solady_safe_transfer_lib(hir: &hir::Hir<'_>) -> bool {
    hir.contracts_enumerated().any(|(cid, c)| {
        c.kind == ContractKind::Library
            && c.name.as_str() == "SafeTransferLib"
            && library_has_safe_transfer_from(hir, cid)
    })
}

fn contract_has_function(
    hir: &hir::Hir<'_>,
    cid: hir::ContractId,
    name: &str,
    params: &[&str],
    returns: &[&str],
) -> bool {
    hir.contract_item_ids(cid).any(|item| {
        let Some(fid) = item.as_function() else { return false };
        let f = hir.function(fid);
        f.name.is_some_and(|n| n.name.as_str() == name)
            && f.parameters.len() == params.len()
            && f.returns.len() == returns.len()
            && f.parameters.iter().zip(params).all(|(id, abi)| is_elementary(hir, *id, abi))
            && f.returns.iter().zip(returns).all(|(id, abi)| is_elementary(hir, *id, abi))
    })
}

/// 4-arg `safeTransferFrom(token, address, address, uint256)`. `token` is either `address`
/// (Solady) or a contract declaring ERC20's `transferFrom(...)→bool` (OZ SafeERC20);
/// ERC721/1155 helpers are excluded since their `transferFrom` has no return.
fn library_has_safe_transfer_from(hir: &hir::Hir<'_>, cid: hir::ContractId) -> bool {
    hir.contract_item_ids(cid).any(|item| {
        let Some(fid) = item.as_function() else { return false };
        let f = hir.function(fid);
        if f.parameters.len() != 4 || f.name.is_none_or(|n| n.name.as_str() != "safeTransferFrom") {
            return false;
        }
        let token_ok = match hir.variable(f.parameters[0]).ty.kind {
            TypeKind::Elementary(ElementaryType::Address(_)) => true,
            TypeKind::Custom(ItemId::Contract(token_cid)) => contract_has_function(
                hir,
                token_cid,
                "transferFrom",
                &["address", "address", "uint256"],
                &["bool"],
            ),
            _ => false,
        };
        token_ok
            && is_address(hir, f.parameters[1])
            && is_address(hir, f.parameters[2])
            && is_elementary(hir, f.parameters[3], "uint256")
    })
}

fn is_address(hir: &hir::Hir<'_>, id: hir::VariableId) -> bool {
    matches!(hir.variable(id).ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
}

/// True when `expr`'s static type is `address` / `address payable`. Covers bare idents,
/// `address(...)` / `payable(...)` casts, and parenthesised wraps.
fn expr_is_address(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().any(|r| {
            matches!(
                r, Res::Item(ItemId::Variable(vid)) if is_address(hir, *vid)
            )
        }),
        ExprKind::Call(callee, _, _) if is_address_cast(callee) => true,
        ExprKind::Payable(_) => true,
        _ => false,
    }
}

fn is_elementary(hir: &hir::Hir<'_>, id: hir::VariableId, abi: &str) -> bool {
    matches!(&hir.variable(id).ty.kind, TypeKind::Elementary(ty) if ty.to_abi_str() == abi)
}

/// Resolves positional or named call args to a fixed positional ordering. `aliases[i]`
/// holds the parameter names accepted for slot `i` in the named form. Returns `None` if
/// arity differs or any slot is unmatched.
fn canonical_args<'hir>(
    kind: hir::CallArgsKind<'hir>,
    aliases: &[&[&str]],
) -> Option<Vec<&'hir hir::Expr<'hir>>> {
    match kind {
        hir::CallArgsKind::Unnamed(exprs) => {
            (exprs.len() == aliases.len()).then(|| exprs.iter().collect())
        }
        hir::CallArgsKind::Named(named) => {
            if named.len() != aliases.len() {
                return None;
            }
            aliases
                .iter()
                .map(|accepted| {
                    named.iter().find_map(|a| {
                        accepted.iter().any(|n| a.name.as_str() == *n).then_some(&a.value)
                    })
                })
                .collect()
        }
    }
}

fn is_address_cast(callee: &hir::Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ElementaryType::Address(_)), .. })
    )
}

fn is_require_or_assert(callee: &hir::Expr<'_>) -> bool {
    let ExprKind::Ident(reses) = &callee.kind else { return false };
    reses.iter().any(
        |r| matches!(r, Res::Builtin(b) if b.name() == sym::require || b.name() == sym::assert),
    )
}

/// `address(this)` or bare `this`.
fn is_address_self(expr: &hir::Expr<'_>) -> bool {
    let expr = expr.peel_parens();
    if is_builtin(expr, sym::this) {
        return true;
    }
    matches!(&expr.kind, ExprKind::Call(callee, args, _) if is_address_cast(callee)
        && args.exprs().next().is_some_and(is_address_self))
}

fn is_builtin(expr: &hir::Expr<'_>, name: solar::interface::Symbol) -> bool {
    let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return false };
    reses.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == name))
}

/// `return`, custom-error `revert`, `revert(...)`, or `assert(false)` / `require(false, ...)`.
fn branch_always_exits(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Revert(_) => true,
        StmtKind::Expr(expr) => is_exit_call(expr),
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => {
            b.stmts.last().is_some_and(branch_always_exits)
        }
        StmtKind::If(_, t, Some(e)) => branch_always_exits(t) && branch_always_exits(e),
        _ => false,
    }
}

fn is_exit_call(expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return false };
    if is_builtin(callee, kw::Revert) {
        return true;
    }
    if is_require_or_assert(callee)
        && let hir::CallArgsKind::Unnamed(unnamed) = args.kind
        && let Some(first) = unnamed.first()
        && matches!(
            &first.peel_parens().kind,
            ExprKind::Lit(lit) if matches!(lit.kind, ast::LitKind::Bool(false))
        )
    {
        return true;
    }
    false
}
