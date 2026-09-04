//! Expression-shape probes over Solar HIR (and a few AST-level ones).

use solar::{
    ast::{self, LitKind},
    interface::{Symbol, kw, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, CallArgs, ElementaryType, Expr, ExprKind, FunctionId, ItemId, Res, TypeKind,
            VariableId,
        },
    },
};

/// True if `expr` (through parens) is an identifier resolving to the given builtin name.
pub fn is_builtin(expr: &Expr<'_>, name: Symbol) -> bool {
    builtins(expr).any(|b| b.name() == name)
}

/// Iterator over the builtins `expr` (through parens) resolves to.
pub fn builtins<'a>(expr: &'a Expr<'a>) -> impl Iterator<Item = Builtin> + 'a {
    let reses: &[Res] = match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses,
        _ => &[],
    };
    reses.iter().filter_map(Res::as_builtin)
}

/// `this` or `super`.
pub fn is_this_or_super(expr: &Expr<'_>) -> bool {
    is_builtin(expr, sym::this) || is_builtin(expr, sym::super_)
}

/// `msg.sender`.
pub fn is_msg_sender(expr: &Expr<'_>) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Member(base, name)
        if name.name == sym::sender && is_builtin(base, sym::msg))
}

/// True if `callee` resolves to the builtin `require` or `assert`.
pub fn is_require_or_assert(callee: &Expr<'_>) -> bool {
    is_builtin(callee, sym::require) || is_builtin(callee, sym::assert)
}

/// `revert(...)`, `revert Err(...)`-style builtin revert call (any form).
pub fn is_revert_call(expr: &Expr<'_>) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Call(callee, ..) if is_builtin(callee, kw::Revert))
}

/// `revert(...)`, `selfdestruct(...)`, `require(false, ...)` or `assert(false)`.
pub fn is_exit_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
    is_builtin(callee, kw::Revert)
        || builtins(callee).any(|b| b == Builtin::Selfdestruct)
        || (is_require_or_assert(callee) && args.exprs().next().is_some_and(is_literal_false))
}

fn is_literal_false(expr: &Expr<'_>) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Lit(lit) if matches!(lit.kind, LitKind::Bool(false)))
}

/// The integer literal `0`.
pub fn is_literal_zero(expr: &Expr<'_>) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Lit(lit)
        if matches!(&lit.kind, LitKind::Number(n) if n.is_zero()))
}

/// `address(...)` / `address payable(...)` cast head.
pub fn is_address_cast(callee: &Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ElementaryType::Address(_)), .. })
    )
}

/// `IFoo(...)` contract / interface cast head.
pub fn is_contract_cast(callee: &Expr<'_>) -> bool {
    matches!(&callee.peel_parens().kind, ExprKind::Ident(reses)
        if !reses.is_empty() && reses.iter().all(|r| matches!(r, Res::Item(ItemId::Contract(_)))))
}

/// `address(...)` or `IFoo(...)` cast head.
pub fn is_address_like_cast(callee: &Expr<'_>) -> bool {
    is_address_cast(callee) || is_contract_cast(callee)
}

/// `address(this)`, `payable(this)`, `IFoo(this)`, `IFoo(address(this))`, or bare `this`.
pub fn is_address_self(expr: &Expr<'_>) -> bool {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Payable(inner) => is_address_self(inner),
        ExprKind::Call(callee, args, _) if is_address_like_cast(callee) => {
            args.exprs().next().is_some_and(is_address_self)
        }
        _ => is_builtin(expr, sym::this),
    }
}

/// The variable a bare identifier refers to, looking through parens, `payable(...)` and
/// address-like casts (`address(x)`, `IFoo(x)`).
pub fn underlying_var(expr: &Expr<'_>) -> Option<VariableId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(Res::as_variable),
        ExprKind::Call(callee, args, _) if is_address_like_cast(callee) => {
            args.exprs().next().and_then(underlying_var)
        }
        ExprKind::Payable(inner) => underlying_var(inner),
        _ => None,
    }
}

/// The local (non-state) variable a bare identifier refers to.
pub fn lhs_local_var(hir: &hir::Hir<'_>, lhs: &Expr<'_>) -> Option<VariableId> {
    let ExprKind::Ident(reses) = &lhs.peel_parens().kind else { return None };
    reses.iter().filter_map(Res::as_variable).find(|v| !hir.variable(*v).kind.is_state())
}

/// State variables written by an lvalue: peels index/slice/member/payable/unary/delete wrappers
/// and tuple destructuring. Duplicates are removed.
pub fn state_lhs_vars(hir: &hir::Hir<'_>, lhs: &Expr<'_>) -> Vec<VariableId> {
    let mut vars = Vec::new();
    for_each_lhs_var(lhs, &mut |v| {
        if hir.variable(v).kind.is_state() && !vars.contains(&v) {
            vars.push(v);
        }
    });
    vars
}

/// Calls `f` for every variable resolution at the root of an lvalue, peeling
/// index/slice/member/payable/unary/delete wrappers and tuple destructuring.
pub fn for_each_lhs_var(expr: &Expr<'_>, f: &mut impl FnMut(VariableId)) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().filter_map(Res::as_variable).for_each(f),
        ExprKind::Index(base, _)
        | ExprKind::Slice(base, ..)
        | ExprKind::Member(base, _)
        | ExprKind::Payable(base)
        | ExprKind::Unary(_, base)
        | ExprKind::Delete(base) => for_each_lhs_var(base, f),
        ExprKind::Tuple(exprs) => exprs.iter().flatten().for_each(|e| for_each_lhs_var(e, f)),
        _ => {}
    }
}

/// The elements of a tuple expression (through parens).
pub fn tuple_elems<'hir>(expr: &'hir Expr<'hir>) -> Option<&'hir [Option<&'hir Expr<'hir>>]> {
    match &expr.peel_parens().kind {
        ExprKind::Tuple(elems) => Some(elems),
        _ => None,
    }
}

/// The argument bound to `param` of `function` in `args`, positional or named.
pub fn arg_for_param<'hir>(
    hir: &hir::Hir<'hir>,
    function: &hir::Function<'hir>,
    param: VariableId,
    args: &CallArgs<'hir>,
) -> Option<&'hir Expr<'hir>> {
    let idx = function.parameters.iter().position(|p| *p == param)?;
    let names: Vec<_> =
        function.parameters.iter().map(|p| hir.variable(*p).name.map(|n| n.name)).collect();
    args.argument_for_parameter(idx, Some(&names))
}

/// The single function the type checker resolved `expr` to (overloads, overrides, `super.`,
/// `using for` and import aliases already accounted for).
pub fn resolved_function(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<FunctionId> {
    gcx.type_of_expr(expr.peel_parens().id)?.function_id()
}

/// All functions a bare identifier callee may resolve to (syntactic, overload-agnostic).
pub fn function_ids<'a>(callee: &'a Expr<'a>) -> impl Iterator<Item = FunctionId> + 'a {
    let reses: &[Res] = match &callee.peel_parens().kind {
        ExprKind::Ident(reses) => reses,
        _ => &[],
    };
    reses.iter().filter_map(Res::as_function)
}

/// The item a bare identifier refers to, if any.
pub fn referenced_item(expr: &Expr<'_>) -> Option<ItemId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(id), ..]) => Some(*id),
        _ => None,
    }
}

/// Receiver of `<expr>.{call,delegatecall,transfer,send}` (value-bearing sinks), including the
/// `.call{value: x}(...)` option form.
pub fn address_call_receiver<'a>(callee: &'a Expr<'a>) -> Option<&'a Expr<'a>> {
    let inner = match &callee.kind {
        ExprKind::Call(inner, ..) if matches!(inner.kind, ExprKind::Member(..)) => inner,
        _ => callee,
    };
    let ExprKind::Member(receiver, name) = &inner.kind else { return None };
    matches!(name.name, kw::Call | kw::Delegatecall | sym::transfer | sym::send).then_some(receiver)
}

/// True if a HIR call carries an explicit `gas:` option.
pub fn is_call_with_gas_limit(expr: &Expr<'_>) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Call(_, _, Some(opts))
        if opts.args.iter().any(|opt| opt.name.name == kw::Gas))
}

/// AST-level: `target.call(...)`, `.delegatecall(...)`, `.staticcall(...)`, with or without
/// `{value: x}` options.
pub const fn is_low_level_call(expr: &ast::Expr<'_>) -> bool {
    if let ast::ExprKind::Call(call_expr, _) = &expr.kind {
        let callee = match &call_expr.kind {
            ast::ExprKind::CallOptions(inner, _) => inner,
            _ => call_expr,
        };
        if let ast::ExprKind::Member(_, member) = &callee.kind {
            return matches!(member.name, kw::Call | kw::Delegatecall | kw::Staticcall);
        }
    }
    false
}
