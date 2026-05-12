//! Predicate decomposition and caller-guard recognition.

use super::primitives::{
    ident_var_ids, is_msg_sender, is_require_or_assert, is_tx_origin, peel_address_wraps,
};
use solar::{
    ast,
    sema::hir::{self, Expr, ExprKind, ItemId, Res, Stmt, StmtKind, VariableId},
};
use std::collections::HashSet;

/// Walks `pred` calling `cb(a, b)` for each equality fact it forces under the
/// given `truth`. Handles `==`/`!=`, `&&`/`||` (via De Morgan), and `!`.
/// Disjunctions are skipped — they don't establish a must-fact.
pub fn walk_eq_facts<'a, F>(pred: &'a Expr<'a>, truth: bool, cb: &mut F)
where
    F: FnMut(&'a Expr<'a>, &'a Expr<'a>),
{
    match &pred.peel_parens().kind {
        ExprKind::Binary(lhs, op, rhs) => {
            let (eq_op, and_op) = if truth {
                (ast::BinOpKind::Eq, ast::BinOpKind::And)
            } else {
                (ast::BinOpKind::Ne, ast::BinOpKind::Or)
            };
            if op.kind == and_op {
                walk_eq_facts(lhs, truth, cb);
                walk_eq_facts(rhs, truth, cb);
            } else if op.kind == eq_op {
                cb(lhs, rhs);
            }
        }
        ExprKind::Unary(op, inner) if matches!(op.kind, ast::UnOpKind::Not) => {
            walk_eq_facts(inner, !truth, cb);
        }
        _ => {}
    }
}

/// True if `pred` mentions the caller anywhere — `msg.sender`, `_msgSender()`,
/// `tx.origin`, or any local recorded in `aliases`.
pub fn pred_constrains_caller(
    hir: &hir::Hir<'_>,
    aliases: &HashSet<VariableId>,
    pred: &Expr<'_>,
) -> bool {
    let e = pred.peel_parens();
    if expr_is_caller_source(hir, aliases, e) {
        return true;
    }
    let mut found = false;
    walk_subexprs(e, &mut |sub| found |= pred_constrains_caller(hir, aliases, sub));
    found
}

/// True if `expr` directly refers to the caller: literal `msg.sender`/`tx.origin`,
/// no-arg `_msgSender()`, or an `Ident` referencing an alias.
pub fn expr_is_caller_source(
    hir: &hir::Hir<'_>,
    aliases: &HashSet<VariableId>,
    expr: &Expr<'_>,
) -> bool {
    let e = peel_address_wraps(expr);
    if is_msg_sender(e) || is_tx_origin(e) {
        return true;
    }
    if let ExprKind::Call(callee, args, _) = &e.kind
        && args.is_empty()
        && callee_is_named(hir, callee, "_msgSender")
    {
        return true;
    }
    if let ExprKind::Ident(reses) = &e.kind {
        return ident_var_ids(reses).any(|v| aliases.contains(&v));
    }
    false
}

/// True if `stmt` is a single-statement caller authorization guard:
/// `require/assert(<caller pred>)`, `if (<caller pred>) revert/return`, or a
/// bare `_checkOwner()` / `_checkRole(...)` call.
pub fn stmt_is_caller_guard(
    hir: &hir::Hir<'_>,
    aliases: &HashSet<VariableId>,
    stmt: &Stmt<'_>,
) -> bool {
    match &stmt.kind {
        StmtKind::Expr(e) => match &e.kind {
            ExprKind::Call(callee, args, _) => {
                (is_require_or_assert(callee)
                    && args.exprs().next().is_some_and(|c| pred_constrains_caller(hir, aliases, c)))
                    || callee_is_named_in(hir, callee, &["_checkOwner", "_checkRole"])
            }
            _ => false,
        },
        StmtKind::If(cond, then, _) => {
            pred_constrains_caller(hir, aliases, cond) && exits_in_then(then)
        }
        _ => false,
    }
}

fn exits_in_then(stmt: &Stmt<'_>) -> bool {
    let is_exit = |s: &Stmt<'_>| matches!(s.kind, StmtKind::Revert(_) | StmtKind::Return(_));
    is_exit(stmt)
        || matches!(&stmt.kind,
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b) if b.stmts.iter().any(is_exit))
}

fn callee_is_named(hir: &hir::Hir<'_>, callee: &Expr<'_>, name: &str) -> bool {
    callee_is_named_in(hir, callee, &[name])
}

fn callee_is_named_in(hir: &hir::Hir<'_>, callee: &Expr<'_>, names: &[&str]) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Member(_, name) => names.contains(&name.as_str()),
        ExprKind::Ident(reses) => reses.iter().any(|r| {
            matches!(r, Res::Item(ItemId::Function(fid))
                if hir.function(*fid).name.is_some_and(|n| names.contains(&n.as_str())))
        }),
        _ => false,
    }
}

fn walk_subexprs<'a, F: FnMut(&'a Expr<'a>)>(expr: &'a Expr<'a>, f: &mut F) {
    match &expr.kind {
        ExprKind::Array(elems) => elems.iter().for_each(f),
        ExprKind::Assign(l, _, r) | ExprKind::Binary(l, _, r) => {
            f(l);
            f(r);
        }
        ExprKind::Call(c, a, n) => {
            f(c);
            a.exprs().for_each(&mut *f);
            if let Some(named) = n {
                named.iter().for_each(|arg| f(&arg.value));
            }
        }
        ExprKind::Delete(e)
        | ExprKind::Unary(_, e)
        | ExprKind::Payable(e)
        | ExprKind::Member(e, _) => f(e),
        ExprKind::Index(b, idx) => {
            f(b);
            if let Some(i) = idx {
                f(i);
            }
        }
        ExprKind::Slice(b, l, r) => {
            f(b);
            if let Some(l) = l {
                f(l);
            }
            if let Some(r) = r {
                f(r);
            }
        }
        ExprKind::Ternary(c, t, fl) => {
            f(c);
            f(t);
            f(fl);
        }
        ExprKind::Tuple(elems) => elems.iter().copied().flatten().for_each(f),
        _ => {}
    }
}
