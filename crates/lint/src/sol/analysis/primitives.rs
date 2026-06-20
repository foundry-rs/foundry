//! Side-effect-free syntactic/semantic probes over solar HIR.

use solar::{
    ast::LitKind,
    interface::{Symbol, kw, sym},
    sema::hir::{self, ElementaryType, Expr, ExprKind, Res, Stmt, StmtKind, TypeKind, VariableId},
};

/// True if `expr` references the named global builtin (`msg`, `tx`, `this`, ...).
fn is_builtin(expr: &Expr<'_>, name: Symbol) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Ident(reses)
        if reses.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == name)))
}

/// True if `vid` is typed as `address`/`address payable`.
pub fn is_address_type(hir: &hir::Hir<'_>, vid: VariableId) -> bool {
    matches!(hir.variable(vid).ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
}

/// True if `callee` resolves to the builtin `require` or `assert`.
pub fn is_require_or_assert(callee: &Expr<'_>) -> bool {
    matches!(&callee.kind, ExprKind::Ident(reses)
        if reses.iter().any(|r| matches!(r,
            Res::Builtin(b) if b.name() == sym::require || b.name() == sym::assert)))
}

/// Receiver of `<expr>.{call,delegatecall,transfer,send}`, including the
/// `.call{value: x}(...)` option form.
pub fn address_call_receiver<'a>(callee: &'a Expr<'a>) -> Option<&'a Expr<'a>> {
    // `addr.call{...}(..)` lowers as `Call(Member(receiver, "call"), ..)`.
    let inner = match &callee.kind {
        ExprKind::Call(inner, ..) => inner,
        _ => callee,
    };
    let target = if matches!(inner.kind, ExprKind::Member(..)) { inner } else { callee };
    if let ExprKind::Member(receiver, name) = &target.kind {
        let n = name.name;
        if n == kw::Call || n == kw::Delegatecall || n == sym::transfer || n == sym::send {
            return Some(receiver);
        }
    }
    None
}

/// True when executing `stmt` provably prevents control from continuing past
/// it: a `return`, `revert`/`revert(...)`, `require(false, ...)`,
/// `assert(false)`, a block containing any such statement (any subsequent
/// statements are unreachable), or an `if` whose both arms exit.
pub fn branch_always_exits(stmt: &Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Revert(_) => true,
        StmtKind::Expr(expr) => is_exit_call(expr),
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => b.stmts.iter().any(branch_always_exits),
        StmtKind::If(_, t, Some(e)) => branch_always_exits(t) && branch_always_exits(e),
        _ => false,
    }
}

fn is_exit_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return false };
    if is_builtin(callee, kw::Revert) {
        return true;
    }
    if is_require_or_assert(callee)
        && let Some(first) = args.exprs().next()
        && matches!(
            &first.peel_parens().kind,
            ExprKind::Lit(lit) if matches!(lit.kind, LitKind::Bool(false))
        )
    {
        return true;
    }
    false
}
