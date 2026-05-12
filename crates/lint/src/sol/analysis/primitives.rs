//! Side-effect-free syntactic/semantic probes over solar HIR.

use solar::{
    interface::{Symbol, kw, sym},
    sema::hir::{
        self, ElementaryType, Expr, ExprKind, ItemId, Res, Stmt, StmtKind, TypeKind, VariableId,
    },
};

/// Strips parens, `payable(...)`, and single-arg type casts (e.g. `address(x)`).
pub fn peel_address_wraps<'a>(expr: &'a Expr<'a>) -> &'a Expr<'a> {
    let mut e = expr.peel_parens();
    loop {
        match &e.kind {
            ExprKind::Payable(inner) => e = inner.peel_parens(),
            ExprKind::Call(callee, args, _) if args.len() == 1 && is_address_cast(callee) => {
                match args.exprs().next() {
                    Some(arg) => e = arg.peel_parens(),
                    None => break,
                }
            }
            _ => break,
        }
    }
    e
}

/// `VariableId` for an expression that resolves to a variable (after peeling).
pub fn underlying_var(expr: &Expr<'_>) -> Option<VariableId> {
    match &peel_address_wraps(expr).kind {
        ExprKind::Ident(reses) => ident_var_ids(reses).next(),
        _ => None,
    }
}

/// Yields every `VariableId` an `Ident` resolves to (covers overloads).
pub fn ident_var_ids<'a>(reses: &'a [Res]) -> impl Iterator<Item = VariableId> + 'a {
    reses.iter().filter_map(|r| match r {
        Res::Item(ItemId::Variable(vid)) => Some(*vid),
        _ => None,
    })
}

/// True if `expr` references the named global builtin (`msg`, `tx`, `this`, ...).
pub fn is_builtin(expr: &Expr<'_>, name: Symbol) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Ident(reses)
        if reses.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == name)))
}

/// True if `expr` is `<base_name>.<member_name>` (e.g. `msg.sender`).
pub fn is_builtin_member(expr: &Expr<'_>, base_name: Symbol, member_name: Symbol) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Member(base, member)
        if member.name == member_name && is_builtin(base.peel_parens(), base_name))
}

/// `msg.sender`.
pub fn is_msg_sender(expr: &Expr<'_>) -> bool {
    is_builtin_member(expr, sym::msg, sym::sender)
}

/// `tx.origin`.
pub fn is_tx_origin(expr: &Expr<'_>) -> bool {
    is_builtin_member(expr, sym::tx, kw::Origin)
}

/// `address(this)`, `payable(this)`, or bare `this` (after peeling).
pub fn is_address_self(expr: &Expr<'_>) -> bool {
    is_builtin(peel_address_wraps(expr), sym::this)
}

/// True if `callee` is the `address` type used as a cast: the `address` in `address(x)`.
pub fn is_address_cast(callee: &Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ElementaryType::Address(_)), .. })
    )
}

/// True if `vid` is typed as `address`/`address payable`.
pub fn is_address_type(hir: &hir::Hir<'_>, vid: VariableId) -> bool {
    matches!(hir.variable(vid).ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
}

/// True if `callee` resolves to the builtin `require` or `assert`.
pub fn is_require_or_assert(callee: &Expr<'_>) -> bool {
    matches!(&callee.peel_parens().kind, ExprKind::Ident(reses)
        if reses.iter().any(|r| matches!(r,
            Res::Builtin(b) if b.name() == sym::require || b.name() == sym::assert)))
}

/// Receiver of `<expr>.{call,delegatecall,staticcall,transfer,send}`, including
/// the `.call{value: x}(...)` option form.
pub fn address_call_receiver<'a>(callee: &'a Expr<'a>) -> Option<&'a Expr<'a>> {
    // `addr.call{...}(..)` lowers as `Call(Member(receiver, "call"), ..)`.
    let inner = match &callee.kind {
        ExprKind::Call(inner, ..) => inner,
        _ => callee,
    };
    let target = if matches!(inner.kind, ExprKind::Member(..)) { inner } else { callee };
    if let ExprKind::Member(receiver, name) = &target.kind {
        let n = name.name;
        if n == kw::Call
            || n == kw::Delegatecall
            || n == kw::Staticcall
            || n == sym::transfer
            || n == sym::send
        {
            return Some(receiver);
        }
    }
    None
}

/// True when `stmt` unconditionally exits the enclosing function: `return`,
/// `revert`/`revert(...)`, `require(false, ...)`, `assert(false)`, or a
/// block/if where every terminal path exits.
pub fn branch_always_exits(stmt: &Stmt<'_>) -> bool {
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

fn is_exit_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return false };
    if is_builtin(callee, kw::Revert) {
        return true;
    }
    if is_require_or_assert(callee)
        && let Some(first) = args.exprs().next()
        && matches!(
            &first.peel_parens().kind,
            ExprKind::Lit(lit) if matches!(lit.kind, solar::ast::LitKind::Bool(false))
        )
    {
        return true;
    }
    false
}
