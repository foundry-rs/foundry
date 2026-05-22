use super::AssertStateChange;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{DataLocation, ElementaryType, UnOpKind},
    interface::{Span, Symbol, kw, sym},
    sema::{
        Hir,
        hir::{ContractId, Expr, ExprKind, FunctionId, ItemId, Res, Type, TypeKind},
    },
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

declare_forge_lint!(
    ASSERT_STATE_CHANGE,
    Severity::Med,
    "assert-state-change",
    "assert() should not contain state-modifying expressions"
);

thread_local! {
    static CURRENT_CONTRACT: RefCell<Option<ContractId>> = const { RefCell::new(None) };
}

impl<'hir> LateLintPass<'hir> for AssertStateChange {
    fn check_nested_contract(&mut self, _ctx: &LintContext, _hir: &'hir Hir<'hir>, id: ContractId) {
        set_current_contract(Some(id));
    }

    fn check_function(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir Hir<'hir>,
        func: &'hir solar::sema::hir::Function<'hir>,
    ) {
        set_current_contract(func.contract);
    }

    fn check_expr(&mut self, ctx: &LintContext, hir: &'hir Hir<'hir>, expr: &'hir Expr<'hir>) {
        let ExprKind::Call(callee, args, _) = &expr.kind else { return };
        if !is_assert(callee) {
            return;
        }

        let current_contract = current_contract();
        for arg in args.exprs() {
            if let Some(span) = find_state_change(hir, current_contract, arg) {
                ctx.emit_with_msg(
                    &ASSERT_STATE_CHANGE,
                    span,
                    "assert() argument contains a state-modifying expression; \
                     assert() is for invariants, hoist the mutation before the assert, \
                     or use require() for validation",
                );
            }
        }
    }
}

fn set_current_contract(id: Option<ContractId>) {
    CURRENT_CONTRACT.with(|cell| *cell.borrow_mut() = id);
}

fn current_contract() -> Option<ContractId> {
    CURRENT_CONTRACT.with(|cell| *cell.borrow())
}

fn is_assert(callee: &Expr<'_>) -> bool {
    let ExprKind::Ident(reses) = &callee.kind else { return false };
    reses.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == sym::assert))
}

/// Recursively searches `expr` for the first sub-expression that modifies state.
/// Returns its span so the diagnostic points at exactly where the mutation occurs.
fn find_state_change<'hir>(
    hir: &Hir<'hir>,
    current_contract: Option<ContractId>,
    expr: &'hir Expr<'hir>,
) -> Option<Span> {
    match &expr.kind {
        // x = y, x += y, etc., only when the lvalue targets a state variable
        ExprKind::Assign(lhs, _, rhs) => {
            if lvalue_is_state_var(hir, lhs) {
                return Some(expr.span);
            }
            find_state_change(hir, current_contract, lhs)
                .or_else(|| find_state_change(hir, current_contract, rhs))
        }

        // delete x, only when x is a state variable
        ExprKind::Delete(inner) => {
            if lvalue_is_state_var(hir, inner) {
                return Some(expr.span);
            }
            find_state_change(hir, current_contract, inner)
        }

        // ++x, x++, --x, x--, only when x is a state variable
        ExprKind::Unary(op, inner)
            if matches!(
                op.kind,
                UnOpKind::PreInc | UnOpKind::PostInc | UnOpKind::PreDec | UnOpKind::PostDec
            ) =>
        {
            if lvalue_is_state_var(hir, inner) {
                return Some(expr.span);
            }
            find_state_change(hir, current_contract, inner)
        }

        ExprKind::Call(callee, args, named_args) => {
            // arr.push(...) / arr.pop() on a storage array/bytes are mutations.
            // Positive type check (`is_dynamic_array_or_bytes`) avoids FPs on interface/contract
            // methods named push/pop, even when the receiver is not a simple Ident (e.g.
            // `stateStruct.queue.push(x)` where `queue: IQueue`).
            if let ExprKind::Member(base, method) = &callee.kind
                && (method.name == sym::push || method.name.as_str() == "pop")
                && is_dynamic_array_or_bytes(hir, current_contract, base)
                && lvalue_is_state_var(hir, base)
            {
                return Some(expr.span);
            }

            // Low-level address calls (.call/.delegatecall/.send/.transfer) are always mutating.
            // Only apply this name-based heuristic when the receiver is syntactically address-like.
            // Using a positive address check rather than "not a known contract" avoids FPs on
            // non-Ident receivers (function-call results, member chains, `this`) whose contract
            // type `contract_id_of` cannot resolve syntactically.
            if let ExprKind::Member(base, method) = &callee.kind {
                let n = method.name;
                if (n == kw::Call || n == kw::Delegatecall || n == sym::send || n == sym::transfer)
                    && is_address_like(hir, current_contract, base)
                {
                    return Some(expr.span);
                }
            }

            // Resolvable contract member calls: check mutates_state() via HIR.
            // We collect all overloads with the same name and arity, then flag when
            // any candidate mutates state. Using `any` avoids FNs where a mutating
            // overload coexists with a view overload of the same arity.
            let candidates = resolve_member_overloads(hir, current_contract, callee, args.len());
            if !candidates.is_empty()
                && candidates.iter().any(|&fid| hir.function(fid).mutates_state())
            {
                return Some(expr.span);
            }

            if candidates.is_empty()
                && let ExprKind::Member(base, method) = &callee.kind
                && lvalue_is_state_var(hir, base)
                && let Some(recv_ty) = receiver_type(hir, current_contract, base)
            {
                let lib_candidates =
                    resolve_library_extension(hir, method.name, args.len(), recv_ty);
                if !lib_candidates.is_empty()
                    && lib_candidates.iter().all(|&fid| hir.function(fid).mutates_state())
                {
                    return Some(expr.span);
                }
            }

            // Bare-identifier internal function calls: same any-mutates policy as member calls,
            // since Solar does not resolve which specific overload was selected.
            let reses = match &callee.peel_parens().kind {
                ExprKind::Ident(r) => *r,
                _ => &[],
            };
            let fn_reses: Vec<FunctionId> = reses
                .iter()
                .filter_map(|res| {
                    if let Res::Item(ItemId::Function(fid)) = res { Some(*fid) } else { None }
                })
                .filter(|&fid| hir.function(fid).parameters.len() == args.len())
                .collect();
            if !fn_reses.is_empty() && fn_reses.iter().any(|&fid| hir.function(fid).mutates_state())
            {
                return Some(expr.span);
            }

            // Recurse into callee, positional args, and named args
            find_state_change(hir, current_contract, callee)
                .or_else(|| args.exprs().find_map(|a| find_state_change(hir, current_contract, a)))
                .or_else(|| {
                    named_args
                        .iter()
                        .flat_map(|na| na.iter())
                        .find_map(|na| find_state_change(hir, current_contract, &na.value))
                })
        }

        ExprKind::Unary(_, inner) | ExprKind::Member(inner, _) | ExprKind::Payable(inner) => {
            find_state_change(hir, current_contract, inner)
        }
        ExprKind::Binary(lhs, _, rhs) => find_state_change(hir, current_contract, lhs)
            .or_else(|| find_state_change(hir, current_contract, rhs)),
        ExprKind::Ternary(cond, t, f) => find_state_change(hir, current_contract, cond)
            .or_else(|| find_state_change(hir, current_contract, t))
            .or_else(|| find_state_change(hir, current_contract, f)),
        ExprKind::Index(base, idx) => find_state_change(hir, current_contract, base)
            .or_else(|| idx.and_then(|i| find_state_change(hir, current_contract, i))),
        ExprKind::Slice(base, start, end) => find_state_change(hir, current_contract, base)
            .or_else(|| start.and_then(|s| find_state_change(hir, current_contract, s)))
            .or_else(|| end.and_then(|e| find_state_change(hir, current_contract, e))),
        ExprKind::Array(exprs) => {
            exprs.iter().find_map(|e| find_state_change(hir, current_contract, e))
        }
        ExprKind::Tuple(exprs) => exprs
            .iter()
            .copied()
            .flatten()
            .find_map(|e| find_state_change(hir, current_contract, e)),
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => None,
    }
}

/// Returns all overloads of the called member function that match the call's argument count.
/// Matching by arity narrows overload candidates; the caller flags the call if any candidate
/// mutates state, since Solar does not resolve which specific overload was selected.
fn resolve_member_overloads<'hir>(
    hir: &Hir<'hir>,
    current_contract: Option<ContractId>,
    callee: &'hir Expr<'hir>,
    arg_count: usize,
) -> Vec<FunctionId> {
    let ExprKind::Member(base, method) = &callee.peel_parens().kind else { return vec![] };
    let Some(cid) = contract_id_of(hir, current_contract, base) else { return vec![] };
    hir.contract_item_ids(cid)
        .filter_map(|item| {
            let fid = item.as_function()?;
            let f = hir.function(fid);
            (f.name.is_some_and(|n| n.name == method.name) && f.parameters.len() == arg_count)
                .then_some(fid)
        })
        .collect()
}

/// Extracts the contract ID from an expression whose static type is a contract or interface.
fn contract_id_of<'hir>(
    hir: &Hir<'hir>,
    current_contract: Option<ContractId>,
    expr: &'hir Expr<'hir>,
) -> Option<ContractId> {
    if is_this_or_super(expr) {
        return current_contract;
    }
    // `IToken(addr).foo()`, explicit interface cast; the callee Ident resolves to the contract
    // itself rather than a function, so receiver_type's Call arm would not match it.
    if let ExprKind::Call(
        Expr { kind: ExprKind::Ident([Res::Item(ItemId::Contract(cid))]), .. },
        ..,
    ) = &expr.peel_parens().kind
    {
        return Some(*cid);
    }
    let ty = receiver_type(hir, current_contract, expr)?;
    if let TypeKind::Custom(ItemId::Contract(cid)) = ty.kind { Some(cid) } else { None }
}

fn is_this_or_super(expr: &Expr<'_>) -> bool {
    let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return false };
    reses
        .iter()
        .any(|r| matches!(r, Res::Builtin(b) if b.name() == sym::this || b.name() == sym::super_))
}

/// Finds library functions in the HIR that could be a `using for` extension matching the given
/// method name, call arity, **and** receiver type. A library extension function has
/// `arg_count + 1` parameters (the extra one being the receiver passed implicitly) with the
/// first parameter in storage, and that first parameter's type must structurally match the
/// receiver's static type, otherwise an unrelated library with a same-named function would
/// false-positive on a contract/interface call.
///
/// Solar does not yet embed resolution info on `ExprKind::Member` for extension methods, so this
/// is a best-effort fallback. The per-name lookup table is memoized per HIR (see
/// `library_extensions_by_name`) to avoid a full `function_ids()` scan on every eligible call.
fn resolve_library_extension<'hir>(
    hir: &Hir<'hir>,
    method_name: Symbol,
    arg_count: usize,
    receiver_ty: &Type<'hir>,
) -> Vec<FunctionId> {
    let expected_params = arg_count + 1; // +1 for the implicit storage receiver
    let by_name = library_extensions_by_name(hir);
    let Some(fids) = by_name.get(&method_name) else { return Vec::new() };
    fids.iter()
        .copied()
        .filter(|&fid| {
            let f = hir.function(fid);
            if f.parameters.len() != expected_params {
                return false;
            }
            // First param must be a storage reference of a type matching the receiver.
            let Some(first) = f.parameters.first().copied().map(|id| hir.variable(id)) else {
                return false;
            };
            if first.data_location != Some(DataLocation::Storage) {
                return false;
            }
            types_compatible(&first.ty, receiver_ty)
        })
        .collect()
}

/// Memoized per-HIR map of library function names to candidate `FunctionId`s. Building the map
/// requires a full `hir.function_ids()` scan; without memoization that scan would run on every
/// eligible call site in the program and scale poorly.
///
/// Identity is keyed on the `Hir<'_>` raw pointer. A given lint run sees a single HIR with a
/// stable address, so pointer comparison is safe; we never deref the pointer beyond identity
/// checking. The cache is `thread_local`, so concurrent project lint workers each maintain
/// their own.
fn library_extensions_by_name(hir: &Hir<'_>) -> Rc<HashMap<Symbol, Vec<FunctionId>>> {
    type Cache = (usize, Rc<HashMap<Symbol, Vec<FunctionId>>>);
    thread_local! {
        static CACHE: RefCell<Option<Cache>> = const { RefCell::new(None) };
    }
    let key = hir as *const Hir<'_> as usize;
    CACHE.with(|cell| {
        if let Some((cached_key, map)) = &*cell.borrow()
            && *cached_key == key
        {
            return map.clone();
        }
        let mut map: HashMap<Symbol, Vec<FunctionId>> = HashMap::new();
        for fid in hir.function_ids() {
            let f = hir.function(fid);
            let Some(cid) = f.contract else { continue };
            if !hir.contract(cid).kind.is_library() {
                continue;
            }
            let Some(name) = f.name else { continue };
            map.entry(name.name).or_default().push(fid);
        }
        let rc = Rc::new(map);
        *cell.borrow_mut() = Some((key, rc.clone()));
        rc
    })
}

/// Returns the static type of a receiver expression, when derivable from HIR alone.
///
/// Handles:
///   * plain `Ident` resolving to a variable;
///   * `base[idx]` for array and mapping types;
///   * `base.field` where `base` has a struct type (looks up the field in the HIR);
///   * `f(...)` where `f` resolves to a single unambiguous function with exactly one return.
///
/// Returns `None` conservatively for anything else, biasing toward false negatives over false
/// positives.
fn receiver_type<'hir>(
    hir: &'hir Hir<'hir>,
    current_contract: Option<ContractId>,
    expr: &'hir Expr<'hir>,
) -> Option<&'hir Type<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) => Some(&hir.variable(*id).ty),
        ExprKind::Index(base, _) => {
            let base_ty = receiver_type(hir, current_contract, base)?;
            match &base_ty.kind {
                TypeKind::Array(arr) => Some(&arr.element),
                TypeKind::Mapping(map) => Some(&map.value),
                _ => None,
            }
        }
        // `cfg.token` where `cfg` is a struct variable, look up the named field's type.
        ExprKind::Member(base, field) => {
            let base_ty = receiver_type(hir, current_contract, base)?;
            let TypeKind::Custom(ItemId::Struct(sid)) = base_ty.kind else { return None };
            hir.strukt(sid).fields.iter().find_map(|&fid| {
                let v = hir.variable(fid);
                v.name.is_some_and(|n| n.name == field.name).then_some(&v.ty)
            })
        }
        // `getToken()` or `factory.token()`, single-return free/member function call.
        ExprKind::Call(callee, args, _) => {
            let fids: Vec<FunctionId> = match &callee.peel_parens().kind {
                ExprKind::Ident(reses) => reses
                    .iter()
                    .filter_map(|r| {
                        if let Res::Item(ItemId::Function(fid)) = r { Some(*fid) } else { None }
                    })
                    .filter(|&fid| hir.function(fid).parameters.len() == args.len())
                    .collect(),
                ExprKind::Member(..) => {
                    resolve_member_overloads(hir, current_contract, callee, args.len())
                }
                _ => Vec::new(),
            };
            single_return_type(hir, fids)
        }
        _ => None,
    }
}

fn single_return_type<'hir>(
    hir: &'hir Hir<'hir>,
    fids: Vec<FunctionId>,
) -> Option<&'hir Type<'hir>> {
    let mut tys = fids.iter().filter_map(|&fid| {
        let f = hir.function(fid);
        (f.returns.len() == 1).then(|| &hir.variable(f.returns[0]).ty)
    });
    let first = tys.next()?;
    tys.all(|ty| types_compatible(first, ty)).then_some(first)
}

/// Structural type equality for the cases that can appear as a `using for` receiver. We compare
/// `TypeKind` shapes directly because `Type` does not implement `PartialEq` and we want to be
/// explicit about which constructors are considered equal.
fn types_compatible(a: &Type<'_>, b: &Type<'_>) -> bool {
    match (&a.kind, &b.kind) {
        (TypeKind::Elementary(x), TypeKind::Elementary(y)) => x == y,
        (TypeKind::Custom(x), TypeKind::Custom(y)) => x == y,
        (TypeKind::Array(x), TypeKind::Array(y)) => types_compatible(&x.element, &y.element),
        (TypeKind::Mapping(x), TypeKind::Mapping(y)) => {
            types_compatible(&x.key, &y.key) && types_compatible(&x.value, &y.value)
        }
        _ => false,
    }
}

/// Returns `true` when `expr` is a dynamic array or `bytes`
fn is_dynamic_array_or_bytes(
    hir: &Hir<'_>,
    current_contract: Option<ContractId>,
    expr: &Expr<'_>,
) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) => matches!(
            hir.variable(*id).ty.kind,
            TypeKind::Array(_) | TypeKind::Elementary(ElementaryType::Bytes)
        ),
        ExprKind::Index(_, _) => matches!(
            receiver_type(hir, current_contract, expr).map(|t| &t.kind),
            Some(TypeKind::Array(_) | TypeKind::Elementary(ElementaryType::Bytes))
        ),
        _ => false,
    }
}

/// Returns `true` when `expr` is address-like, as far as can be determined from HIR alone.
///
/// Recognized address-like shapes:
///   * `payable(x)` and `address(x)` / `address payable(x)` casts;
///   * a plain `Ident` resolving to a variable of `address` type;
///   * indexing into a state variable whose element type is `address`
///     (`addresses[i].transfer(...)`);
///   * struct fields and function-call results with static `address` type;
///   * builtin address members (`msg.sender`, `tx.origin`, `block.coinbase`);
///   * a single-element tuple wrapping an address-like expression.
fn is_address_like<'hir>(
    hir: &Hir<'hir>,
    current_contract: Option<ContractId>,
    expr: &'hir Expr<'hir>,
) -> bool {
    if matches!(
        receiver_type(hir, current_contract, expr).map(|t| &t.kind),
        Some(TypeKind::Elementary(ElementaryType::Address(_)))
    ) {
        return true;
    }

    match &expr.peel_parens().kind {
        ExprKind::Payable(_) => true,
        // `address(x)` / `address payable(x)` casts parse as `Call(Type(Address), [x])`.
        ExprKind::Call(callee, _, _) => matches!(
            &callee.peel_parens().kind,
            ExprKind::Type(Type { kind: TypeKind::Elementary(ElementaryType::Address(_)), .. })
        ),
        // `msg.sender`, `tx.origin`, `block.coinbase`.
        ExprKind::Member(base, member) => is_address_builtin_member(base, member.name),
        ExprKind::Tuple(exprs) => {
            let mut iter = exprs.iter().flatten();
            match (iter.next(), iter.next()) {
                (Some(inner), None) => is_address_like(hir, current_contract, inner),
                _ => false,
            }
        }
        _ => false,
    }
}

fn is_address_builtin_member(base: &Expr<'_>, member: Symbol) -> bool {
    let ExprKind::Ident(reses) = &base.peel_parens().kind else { return false };
    reses.iter().any(|res| {
        let Res::Builtin(builtin) = res else { return false };
        matches!(
            (builtin.name(), member),
            (sym::msg, sym::sender) | (sym::tx, kw::Origin) | (sym::block, kw::Coinbase)
        )
    })
}

/// Returns `true` if the lvalue expression ultimately targets a storage variable.
/// Peels through index, slice, member, and payable wrappers to find the root identifier.
/// Locals declared `storage` are aliases into contract storage and count as state mutations.
fn lvalue_is_state_var(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) => {
            let v = hir.variable(*id);
            v.is_state_variable() || v.data_location == Some(DataLocation::Storage)
        }
        ExprKind::Index(base, _)
        | ExprKind::Slice(base, _, _)
        | ExprKind::Member(base, _)
        | ExprKind::Payable(base) => lvalue_is_state_var(hir, base),
        ExprKind::Tuple(exprs) => exprs.iter().flatten().any(|e| lvalue_is_state_var(hir, e)),
        _ => false,
    }
}
