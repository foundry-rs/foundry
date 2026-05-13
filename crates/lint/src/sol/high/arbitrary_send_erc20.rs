use super::ArbitrarySendErc20;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    interface::{Span, data_structures::Never, kw, sym},
    sema::hir::{
        self, ContractKind, ElementaryType, ExprKind, ItemId, Res, StmtKind, TypeKind, Visit,
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

        let mut a = Analyzer::new(hir);
        for m in func.modifiers {
            collect_modifier_safety(hir, m, &mut a.safe_vars);
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

struct Analyzer<'hir> {
    hir: &'hir hir::Hir<'hir>,
    /// Variables proven safe (equal to `msg.sender` or `address(this)`) on this path.
    /// Function-locals and `immutable`/`constant` state vars only — mutable storage may be
    /// rewritten between the check and the sink.
    safe_vars: HashSet<hir::VariableId>,
    /// Permits seen earlier on this path. Path-sensitive and killed on token/owner reassignment.
    permits: HashSet<PermitRecord>,
    hits: Vec<Span>,
}

/// Recursion budget for `_msgSender()`-style helper chains.
const HELPER_DEPTH: u8 = 3;

impl<'hir> Analyzer<'hir> {
    fn new(hir: &'hir hir::Hir<'hir>) -> Self {
        Self { hir, safe_vars: HashSet::new(), permits: HashSet::new(), hits: Vec::new() }
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

    /// Visits a body that may execute zero times or out-of-line (loops, try clauses):
    /// in-body kills survive, in-body additions don't.
    fn visit_isolated(&mut self, stmts: &'hir [hir::Stmt<'hir>]) {
        let baseline_safe = self.safe_vars.clone();
        let baseline_permits = self.permits.clone();
        for s in stmts {
            let _ = self.visit_stmt(s);
        }
        self.safe_vars.retain(|v| baseline_safe.contains(v));
        self.permits.retain(|p| baseline_permits.contains(p));
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
                self.add_facts(cond, false);
                let _ = self.visit_stmt(then);
                let then_exits = branch_always_exits(then);
                let after_then_safe = std::mem::replace(&mut self.safe_vars, baseline_safe);
                let after_then_permits = std::mem::replace(&mut self.permits, baseline_permits);

                let (after_else_safe, after_else_permits, else_exits) = match else_ {
                    Some(e) => {
                        let _ = self.visit_stmt(e);
                        (
                            std::mem::take(&mut self.safe_vars),
                            std::mem::take(&mut self.permits),
                            branch_always_exits(e),
                        )
                    }
                    None => {
                        self.add_facts(cond, true);
                        (
                            std::mem::take(&mut self.safe_vars),
                            std::mem::take(&mut self.permits),
                            false,
                        )
                    }
                };

                let (sv, pm) = match (then_exits, else_exits) {
                    (true, true) => (
                        after_then_safe.union(&after_else_safe).copied().collect(),
                        after_then_permits.union(&after_else_permits).copied().collect(),
                    ),
                    (true, false) => (after_else_safe, after_else_permits),
                    (false, true) => (after_then_safe, after_then_permits),
                    (false, false) => (
                        after_then_safe.intersection(&after_else_safe).copied().collect(),
                        after_then_permits.intersection(&after_else_permits).copied().collect(),
                    ),
                };
                self.safe_vars = sv;
                self.permits = pm;
                return ControlFlow::Continue(());
            }

            StmtKind::Loop(block, _) => {
                self.visit_isolated(block.stmts);
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
        match &expr.kind {
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                if let Some(cond) = args.exprs().next() {
                    self.add_facts(cond, false);
                }
            }
            ExprKind::Call(..) => {
                if let Some(p) = match_permit_call(expr) {
                    self.permits.insert(p);
                } else if let Some((from, token)) = match_sink(self.hir, expr)
                    && !self.is_safe(from)
                    && !self.permit_covers(token, from)
                {
                    self.hits.push(expr.span);
                }
            }
            // Kill permits on every write; only track safety for non-state address locals.
            ExprKind::Assign(lhs, _, rhs) => {
                if let Some(target) = underlying_var(lhs) {
                    self.kill_permits_for(target);
                    if !self.hir.variable(target).kind.is_state() && is_address(self.hir, target) {
                        self.assign(target, rhs);
                    }
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
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
    expr: &'hir hir::Expr<'hir>,
) -> Option<(&'hir hir::Expr<'hir>, Option<hir::VariableId>)> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let hir::CallArgsKind::Unnamed(args) = args.kind else { return None };
    let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
    let name = ident.name.as_str();

    if args.len() == 3 && (name == "transferFrom" || name == "safeTransferFrom") {
        let cid = receiver_contract_id(hir, recv)?;
        contract_has_function(
            hir,
            cid,
            "transferFrom",
            &["address", "address", "uint256"],
            &["bool"],
        )
        .then(|| (&args[0], underlying_var(recv)))
    } else if args.len() == 4 && name == "safeTransferFrom" {
        let cid = receiver_contract_id(hir, recv)?;
        (hir.contract(cid).kind == ContractKind::Library
            && library_has_safe_transfer_from(hir, cid))
        .then(|| (&args[1], underlying_var(&args[0])))
    } else {
        None
    }
}

/// Matches `token.permit(owner, address(this), value, deadline, v, r, s)` (EIP-2612 shape).
/// Only `spender == address(this)` permits are recorded; others can't suppress a sink.
fn match_permit_call(expr: &hir::Expr<'_>) -> Option<PermitRecord> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let hir::CallArgsKind::Unnamed(args) = args.kind else { return None };
    if args.len() != 7 {
        return None;
    }
    let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
    if ident.name.as_str() != "permit" || !is_address_self(&args[1]) {
        return None;
    }
    Some(PermitRecord { token: underlying_var(recv)?, owner: underlying_var(&args[0])? })
}

/// Hoists `require(modParam == msg.sender)` style guards from the modifier prefix (statements
/// before its single top-level `_;`), mapping modifier params back to caller args.
fn collect_modifier_safety(
    hir: &hir::Hir<'_>,
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

    let mut a = Analyzer::new(hir);
    for stmt in &body.stmts[..placeholder_idx] {
        let _ = a.visit_stmt(stmt);
    }
    for (mp, caller) in arg_map {
        if a.safe_vars.contains(&mp) {
            out_safe.insert(caller);
        }
    }
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
/// reference (e.g. a library), or an `IERC20(addr)` interface wrap.
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
        _ => None,
    }
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

/// 4-arg `safeTransferFrom(token, address, address, uint256)`. The `token` param can be
/// either a contract type (OpenZeppelin SafeERC20: `IERC20`) or an `address` (Solady
/// SafeTransferLib). Can't go through `contract_has_function` because the first form's
/// first param isn't elementary.
fn library_has_safe_transfer_from(hir: &hir::Hir<'_>, cid: hir::ContractId) -> bool {
    hir.contract_item_ids(cid).any(|item| {
        let Some(fid) = item.as_function() else { return false };
        let f = hir.function(fid);
        if f.parameters.len() != 4 || f.name.is_none_or(|n| n.name.as_str() != "safeTransferFrom") {
            return false;
        }
        let token_ok = matches!(
            hir.variable(f.parameters[0]).ty.kind,
            TypeKind::Custom(ItemId::Contract(_))
                | TypeKind::Elementary(ElementaryType::Address(_))
        );
        token_ok
            && is_address(hir, f.parameters[1])
            && is_address(hir, f.parameters[2])
            && is_elementary(hir, f.parameters[3], "uint256")
    })
}

fn is_address(hir: &hir::Hir<'_>, id: hir::VariableId) -> bool {
    matches!(hir.variable(id).ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
}

fn is_elementary(hir: &hir::Hir<'_>, id: hir::VariableId, abi: &str) -> bool {
    matches!(&hir.variable(id).ty.kind, TypeKind::Elementary(ty) if ty.to_abi_str() == abi)
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
