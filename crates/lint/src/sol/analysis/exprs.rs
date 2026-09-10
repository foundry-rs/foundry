//! Expression-shape probes over Solar HIR (and a few AST-level ones).

use solar::{
    ast::{self, LitKind, UnOpKind},
    interface::{Symbol, kw, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, CallArgs, ElementaryType, Expr, ExprKind, FunctionId, ItemId, Res, TypeKind,
            VariableId,
        },
        ty::{CallableParamSource, TyKind},
    },
};
use std::ops::ControlFlow;

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

/// `msg.sender`.
pub fn is_msg_sender(expr: &Expr<'_>) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Member(base, name)
        if name.name == sym::sender && is_builtin(base, sym::msg))
}

/// `msg.sender` or `tx.origin`.
pub fn is_sender_member(expr: &Expr<'_>) -> bool {
    is_msg_sender(expr)
        || matches!(&expr.peel_parens().kind, ExprKind::Member(base, name)
            if name.name == kw::Origin && is_builtin(base, sym::tx))
}

/// True if `callee` resolves to the builtin `require` or `assert`.
pub fn is_require_or_assert(callee: &Expr<'_>) -> bool {
    is_builtin(callee, sym::require) || is_builtin(callee, sym::assert)
}

/// `revert(...)`, `revert Err(...)`-style builtin revert call (any form).
pub fn is_revert_call(expr: &Expr<'_>) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Call(callee, ..) if is_builtin(callee, kw::Revert))
}

/// A literal zero/false or an elementary cast or arithmetic negation of one.
pub fn is_zero_value(expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => match &lit.kind {
            LitKind::Number(value) => value.is_zero(),
            LitKind::Address(value) => value.is_zero(),
            LitKind::Bool(value) => !value,
            _ => false,
        },
        ExprKind::Call(callee, args, _) if cast_type(callee).is_some() => {
            let mut exprs = args.exprs();
            exprs.len() == 1 && exprs.next().is_some_and(is_zero_value)
        }
        ExprKind::Payable(inner) => is_zero_value(inner),
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Neg => is_zero_value(inner),
        _ => false,
    }
}

/// `revert(...)`, `selfdestruct(...)`, `require(false, ...)` or `assert(false)`.
pub fn is_exit_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
    is_builtin(callee, kw::Revert)
        || builtins(callee).any(|b| b == Builtin::Selfdestruct)
        || (is_require_or_assert(callee) && args.exprs().next().is_some_and(is_literal_false))
}

/// The boolean literal `false`.
pub fn is_literal_false(expr: &Expr<'_>) -> bool {
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
pub fn underlying_var(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<VariableId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(_) => gcx.resolved_variable(expr),
        ExprKind::Call(callee, args, _) if is_address_like_cast(callee) => {
            args.exprs().next().and_then(|arg| underlying_var(gcx, arg))
        }
        ExprKind::Payable(inner) => underlying_var(gcx, inner),
        _ => None,
    }
}

/// The local (non-state) variable a bare identifier refers to.
pub fn lhs_local_var(gcx: Gcx<'_>, lhs: &Expr<'_>) -> Option<VariableId> {
    let ExprKind::Ident(_) = lhs.peel_parens().kind else { return None };
    gcx.resolved_variable(lhs).filter(|&v| !gcx.hir.variable(v).kind.is_state())
}

/// State variables written by an lvalue: peels index/slice/member/payable/unary/delete wrappers
/// and tuple destructuring. Duplicates are removed.
pub fn state_lhs_vars(gcx: Gcx<'_>, lhs: &Expr<'_>) -> Vec<VariableId> {
    let mut vars = Vec::new();
    for_each_lhs_var(gcx, lhs, &mut |v| {
        if gcx.hir.variable(v).kind.is_state() && !vars.contains(&v) {
            vars.push(v);
        }
    });
    vars
}

/// Calls `f` for each resolved variable at the root of an lvalue, peeling
/// index/slice/member/payable/unary/delete wrappers and tuple destructuring.
pub fn for_each_lhs_var(gcx: Gcx<'_>, expr: &Expr<'_>, f: &mut impl FnMut(VariableId)) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(_) => {
            if let Some(var) = gcx.resolved_variable(expr) {
                f(var);
            }
        }
        ExprKind::Index(base, _)
        | ExprKind::Slice(base, ..)
        | ExprKind::Member(base, _)
        | ExprKind::Payable(base)
        | ExprKind::Unary(_, base)
        | ExprKind::Delete(base) => for_each_lhs_var(gcx, base, f),
        ExprKind::Tuple(exprs) => exprs.iter().flatten().for_each(|e| for_each_lhs_var(gcx, e, f)),
        _ => {}
    }
}

/// The elements of a tuple expression (through parens).
pub fn tuple_elems<'gcx>(expr: &'gcx Expr<'gcx>) -> Option<&'gcx [Option<&'gcx Expr<'gcx>>]> {
    match &expr.peel_parens().kind {
        ExprKind::Tuple(elems) => Some(elems),
        _ => None,
    }
}

/// The argument bound to `param` of `function_id` in `args`, positional or named.
pub fn arg_for_param<'gcx>(
    gcx: Gcx<'gcx>,
    function_id: FunctionId,
    param: VariableId,
    args: &CallArgs<'gcx>,
) -> Option<&'gcx Expr<'gcx>> {
    let function = gcx.hir.function(function_id);
    let idx = function.parameters.iter().position(|p| *p == param)?;
    let names = gcx.callable_param_names(CallableParamSource::Function {
        id: function_id,
        skips_receiver: false,
    });
    args.argument_for_parameter(idx, Some(&names))
}

/// The function an internal call made from within `contract_id` dispatches to: a virtual call
/// resolves to the most derived override, `super.f` to the next base implementation, and a
/// qualified `Base.f` to that declaration exactly. `None` for external and unresolved callees.
pub fn dispatched_function(
    gcx: Gcx<'_>,
    contract_id: hir::ContractId,
    callee: &Expr<'_>,
) -> Option<FunctionId> {
    let callee = callee.peel_parens();
    let function_id = gcx.resolved_callee(callee.id)?.res.as_function()?;
    match &callee.kind {
        ExprKind::Member(base, _) => {
            let solar::sema::ty::TyKind::Type(ty) = gcx.type_of_expr(base.id)?.kind else {
                return None;
            };
            match ty.kind {
                solar::sema::ty::TyKind::Contract(_) => Some(function_id),
                solar::sema::ty::TyKind::Super(defining) => {
                    Some(gcx.resolve_super_function(contract_id, defining, function_id))
                }
                _ => None,
            }
        }
        ExprKind::Ident(_) => Some(gcx.resolve_virtual_function(contract_id, function_id)),
        _ => None,
    }
}

/// The item a bare identifier refers to, if any.
pub fn referenced_item(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<ItemId> {
    let ExprKind::Ident(_) = expr.peel_parens().kind else { return None };
    match gcx.resolved_expr(expr)? {
        Res::Item(id) => Some(id),
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

/// The lvalue written by an assignment, `delete` or increment/decrement expression.
pub const fn write_target<'gcx>(expr: &'gcx Expr<'gcx>) -> Option<&'gcx Expr<'gcx>> {
    match &expr.kind {
        ExprKind::Assign(target, ..) | ExprKind::Delete(target) => Some(target),
        ExprKind::Unary(op, target) if op.kind.has_side_effects() => Some(target),
        _ => None,
    }
}

/// The functions reachable through the runtime dispatch of a most-derived contract: its
/// interface functions plus the inherited `fallback`/`receive`, if any.
pub fn runtime_entry_points(gcx: Gcx<'_>, contract_id: hir::ContractId) -> Vec<FunctionId> {
    let bases = gcx.hir.contract(contract_id).linearized_bases;
    let mut entries: Vec<_> =
        gcx.interface_functions(contract_id).all().iter().map(|f| f.id).collect();
    entries.extend(bases.iter().find_map(|&cid| gcx.hir.contract(cid).fallback));
    entries.extend(bases.iter().find_map(|&cid| gcx.hir.contract(cid).receive));
    entries
}

/// The elementary type an explicit cast head `T(...)` converts to.
pub fn cast_type(callee: &Expr<'_>) -> Option<ElementaryType> {
    match &callee.peel_parens().kind {
        ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) => Some(*ty),
        _ => None,
    }
}

/// `address` / `address payable` or a contract/interface type.
pub const fn var_is_address_like(var: &hir::Variable<'_>) -> bool {
    matches!(
        var.ty.kind,
        TypeKind::Elementary(ElementaryType::Address(_)) | TypeKind::Custom(ItemId::Contract(_))
    )
}

/// AST-level boolean literal, through parens.
pub fn ast_bool_literal(expr: &ast::Expr<'_>) -> Option<bool> {
    match &expr.peel_parens().kind {
        ast::ExprKind::Lit(ast::Lit { kind: LitKind::Bool(value), .. }, _) => Some(*value),
        _ => None,
    }
}

/// Calls `f` on every direct sub-expression of `expr`, in evaluation order.
pub fn for_each_child<'gcx>(expr: &'gcx Expr<'gcx>, f: &mut impl FnMut(&'gcx Expr<'gcx>)) {
    match &expr.kind {
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            f(lhs);
            f(rhs);
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => f(inner),
        ExprKind::Call(callee, args, opts) => {
            f(callee);
            opts.iter().flat_map(|opts| opts.args).for_each(|opt| f(&opt.value));
            args.exprs().for_each(f);
        }
        ExprKind::Index(base, index) => {
            f(base);
            index.iter().copied().for_each(f);
        }
        ExprKind::Slice(base, start, end) => {
            f(base);
            [*start, *end].into_iter().flatten().for_each(f);
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            f(cond);
            f(true_expr);
            f(false_expr);
        }
        ExprKind::Array(exprs) => exprs.iter().for_each(f),
        ExprKind::Tuple(exprs) => exprs.iter().flatten().copied().for_each(f),
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::YulMember(..)
        | ExprKind::Err(_) => {}
    }
}

/// True if `pred` holds for `expr` or any of its sub-expressions.
pub fn any_subexpr(expr: &Expr<'_>, mut pred: impl FnMut(&Expr<'_>) -> bool) -> bool {
    expr.visit(&mut |e| if pred(e) { ControlFlow::Break(()) } else { ControlFlow::Continue(()) })
        .is_break()
}

/// True if evaluating `expr` performs an assignment, `delete` or increment/decrement.
pub fn has_side_effect(expr: &Expr<'_>) -> bool {
    any_subexpr(expr, |e| match &e.kind {
        ExprKind::Assign(..) | ExprKind::Delete(_) => true,
        ExprKind::Unary(op, _) => op.kind.has_side_effects(),
        _ => false,
    })
}

/// True when `callee` names a zero-parameter function whose body returns an expression matching
/// `pred`.
pub fn callee_no_arg_returns<'gcx>(
    gcx: Gcx<'gcx>,
    callee: &'gcx Expr<'gcx>,
    mut pred: impl FnMut(&'gcx Expr<'gcx>) -> bool,
) -> bool {
    gcx.type_of_expr(callee.peel_parens().id).is_some_and(
        |ty| matches!(ty.kind, TyKind::Fn(f) if f.is_internal() || f.is_delegate_call()),
    ) && gcx
        .resolved_function(callee)
        .is_some_and(|fid| function_no_arg_returns(gcx, fid, &mut pred))
}

/// True when `fid` takes no parameters and its body is `return e;` or `namedRet = e;` (optionally
/// followed by a bare `return;`) with `pred(e)`.
pub fn function_no_arg_returns<'gcx>(
    gcx: Gcx<'gcx>,
    fid: FunctionId,
    pred: &mut impl FnMut(&'gcx Expr<'gcx>) -> bool,
) -> bool {
    let f = gcx.hir.function(fid);
    let Some(body) = f.body else { return false };
    let stmts = match body.stmts {
        [rest @ .., last] if matches!(last.kind, hir::StmtKind::Return(None)) => rest,
        stmts => stmts,
    };
    let [stmt] = stmts else { return false };
    f.parameters.is_empty()
        && match &stmt.kind {
            hir::StmtKind::Return(Some(e)) => pred(e),
            hir::StmtKind::Expr(e) => {
                matches!(&e.peel_parens().kind, ExprKind::Assign(lhs, None, rhs)
                if f.returns.len() == 1 && underlying_var(gcx, lhs) == Some(f.returns[0]) && pred(rhs))
            }
            _ => false,
        }
}

/// Package-root directory names of the OpenZeppelin distributions (npm scope and git submodules).
pub const OPENZEPPELIN_ROOTS: &[&str] =
    &["@openzeppelin", "openzeppelin-contracts", "openzeppelin-contracts-upgradeable"];

/// True if the source file of `source_id` lives under one of the given package-root directory
/// names (matched as whole, case-insensitive path components).
pub fn source_in_package(hir: &hir::Hir<'_>, source_id: hir::SourceId, roots: &[&str]) -> bool {
    let solar::interface::source_map::FileName::Real(path) = &hir.source(source_id).file.name
    else {
        return false;
    };
    path.components().any(|component| {
        matches!(component, std::path::Component::Normal(name)
            if roots.iter().any(|root| name.eq_ignore_ascii_case(root)))
    })
}
