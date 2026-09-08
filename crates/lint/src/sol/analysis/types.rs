//! Type probes: static types of HIR expressions and variables.

use super::{is_address_cast, is_this_or_super, referenced_item};
use solar::{
    ast::{DataLocation, LitKind, StrKind, TypeSize, UnOpKind},
    interface::Symbol,
    sema::{
        Gcx, Ty,
        hir::{
            self, BinOpKind, CallArgs, ContractId, Expr, ExprKind, ItemId, Res, TypeKind,
            VariableId,
        },
        ty::TyKind,
    },
};

/// True if `vid` is typed as `address`/`address payable`.
pub fn is_address_type(hir: &hir::Hir<'_>, vid: VariableId) -> bool {
    matches!(hir.variable(vid).ty.kind, TypeKind::Elementary(hir::ElementaryType::Address(_)))
}

/// True if `id`'s elementary type matches the given ABI string.
pub fn is_elementary(hir: &hir::Hir<'_>, id: VariableId, abi: &str) -> bool {
    matches!(&hir.variable(id).ty.kind, TypeKind::Elementary(ty) if ty.to_abi_str() == abi)
}

/// `address` / `address payable` after peeling references.
pub fn ty_is_address(ty: Ty<'_>) -> bool {
    ty.peel_refs().is_address()
}

/// The contract a type denotes, through references and `type(C)`.
pub fn ty_contract_id(ty: Ty<'_>) -> Option<ContractId> {
    match ty.peel_refs().kind {
        TyKind::Contract(id) => Some(id),
        TyKind::Type(ty) => ty_contract_id(ty),
        _ => None,
    }
}

/// True when `expr`'s type-checked static type is `address` / `address payable`.
pub fn expr_is_address<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> bool {
    gcx.type_of_expr(expr.peel_parens().id).is_some_and(ty_is_address)
}

/// `address`-typed expression, or an address cast / `payable(..)` wrap whose type is unknown.
pub fn is_address_like<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Payable(_) => true,
        ExprKind::Call(callee, ..) if is_address_cast(callee) => true,
        _ => expr_ty(gcx, expr).is_some_and(ty_is_address),
    }
}

/// Static contract type of a method-call receiver: a contract-typed expression, a direct
/// contract/library reference, or an `IFoo(addr)` cast.
pub fn receiver_contract_id<'gcx>(gcx: Gcx<'gcx>, recv: &Expr<'gcx>) -> Option<ContractId> {
    expr_ty(gcx, recv).and_then(ty_contract_id).or_else(|| direct_contract_id(recv))
}

fn direct_contract_id(expr: &Expr<'_>) -> Option<ContractId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Contract(cid)) => Some(*cid),
            _ => None,
        }),
        ExprKind::Call(callee, ..) => direct_contract_id(callee),
        _ => None,
    }
}

/// Data location of a variable, defaulting state variables to storage.
pub fn variable_data_location(hir: &hir::Hir<'_>, var_id: VariableId) -> Option<DataLocation> {
    let var = hir.variable(var_id);
    var.data_location.or_else(|| var.kind.is_state().then_some(DataLocation::Storage))
}

/// Static type of `expr`. Uses the type checker's result when available and otherwise
/// reconstructs the type structurally from the HIR (locations of storage-rooted lvalues are
/// preserved). `this`/`super` and unresolvable expressions yield `None`.
pub fn expr_ty<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> Option<Ty<'gcx>> {
    let expr = expr.peel_parens();
    if !is_this_or_super(expr)
        && let Some(ty) = gcx.type_of_expr(expr.id)
    {
        return Some(ty);
    }
    match &expr.kind {
        ExprKind::Call(callee, args, _) => match expr_ty(gcx, callee)?.kind {
            TyKind::Fn(func) => Some(match func.returns {
                [] => gcx.types.unit,
                [ret] => *ret,
                returns => gcx.mk_ty_tuple(returns),
            }),
            TyKind::Type(to) => Some(explicit_cast_ty(gcx, to, args)),
            _ => None,
        },
        ExprKind::Ident(reses) => {
            let res = unique(reses.iter().filter(|res| !matches!(res, Res::Err(_))).copied())
                .or_else(|| {
                    unique(
                        reses.iter().filter_map(|r| r.as_variable().map(|v| Res::Item(v.into()))),
                    )
                })?;
            if is_this_or_super(expr) {
                return None;
            }
            let ty = gcx.type_of_res(res);
            Some(match res.as_variable() {
                Some(v) => ty.with_loc_if_ref_opt(gcx, variable_data_location(&gcx.hir, v)),
                None => ty,
            })
        }
        ExprKind::Index(lhs, index) => {
            let lhs_ty = expr_ty(gcx, lhs)?;
            if let Some(index) = index
                && !expr_ty(gcx, index)?.convert_implicit_to(gcx.types.uint(256), gcx)
            {
                return None;
            }
            let loc = lhs_ty.loc().or_else(|| {
                matches!(lhs_ty.kind, TyKind::Mapping(..)).then_some(DataLocation::Storage)
            });
            match lhs_ty.peel_refs().kind {
                TyKind::Mapping(_, value) => Some(value.with_loc_if_ref_opt(gcx, loc)),
                _ => lhs_ty.base_type(gcx),
            }
        }
        ExprKind::Lit(lit) => Some(match &lit.kind {
            LitKind::Str(StrKind::Hex, s, _) => {
                let size = TypeSize::try_new_fb_bytes(s.as_byte_str().len().min(32) as u8)?;
                gcx.types.fixed_bytes(size.bytes())
            }
            LitKind::Str(_, s, _) => gcx.mk_ty_string_literal(s.as_byte_str()),
            LitKind::Number(int) => gcx.mk_ty_int_literal(false, int.bit_len() as _)?,
            LitKind::Rational(_) | LitKind::Err(_) => return None,
            LitKind::Address(_) => gcx.types.address,
            LitKind::Bool(_) => gcx.types.bool,
        }),
        ExprKind::Member(base, member) => member_ty(gcx, base, member.name),
        ExprKind::New(ty) | ExprKind::Type(ty) | ExprKind::TypeCall(ty) => {
            Some(gcx.mk_ty(TyKind::Type(gcx.type_of_hir_ty(ty))))
        }
        ExprKind::Payable(inner) => expr_ty(gcx, inner)?
            .convert_explicit_to(gcx.types.address_payable, gcx)
            .then_some(gcx.types.address_payable),
        ExprKind::Slice(lhs, ..) => {
            let lhs_ty = expr_ty(gcx, lhs)?;
            lhs_ty.is_sliceable().then(|| gcx.mk_ty(TyKind::Slice(lhs_ty)))
        }
        ExprKind::Tuple(exprs) => {
            let tys = exprs.iter().map(|e| expr_ty(gcx, (*e)?)).collect::<Option<Vec<_>>>()?;
            Some(gcx.mk_ty_tuple(gcx.mk_tys(&tys)))
        }
        ExprKind::Ternary(_, t, f) => expr_ty(gcx, t)?.common_type(expr_ty(gcx, f)?, gcx),
        ExprKind::Unary(op, _) if op.kind == UnOpKind::Not => Some(gcx.types.bool),
        ExprKind::Unary(_, inner) => expr_ty(gcx, inner),
        ExprKind::Binary(_, op, _)
            if matches!(
                op.kind,
                BinOpKind::Lt
                    | BinOpKind::Le
                    | BinOpKind::Gt
                    | BinOpKind::Ge
                    | BinOpKind::Eq
                    | BinOpKind::Ne
                    | BinOpKind::And
                    | BinOpKind::Or
            ) =>
        {
            Some(gcx.types.bool)
        }
        _ => None,
    }
}

fn explicit_cast_ty<'gcx>(gcx: Gcx<'gcx>, to: Ty<'gcx>, args: &CallArgs<'gcx>) -> Ty<'gcx> {
    match args.exprs().next().and_then(|arg| expr_ty(gcx, arg)) {
        Some(from) => from.try_convert_explicit_to(to, gcx).unwrap_or(to),
        None => to,
    }
}

/// Type of `base.<member>`, resolved through Solar's member tables (unique match only).
pub fn member_ty<'gcx>(gcx: Gcx<'gcx>, base: &Expr<'gcx>, member: Symbol) -> Option<Ty<'gcx>> {
    if is_this_or_super(base) {
        return None;
    }
    let base_ty = expr_ty(gcx, base)?;
    let item = referenced_item(base);
    let source = item
        .map(|id| gcx.hir.item(id).source())
        .unwrap_or_else(|| gcx.hir.sources_enumerated().next().expect("HIR has a source").0);
    let contract = item.and_then(|id| gcx.hir.item(id).contract());
    unique(gcx.members_of(base_ty, source, contract).filter(|m| m.name == member).map(|m| m.ty))
}

/// The only element of `iter`, or `None` when it has zero or several.
pub fn unique<T>(mut iter: impl Iterator<Item = T>) -> Option<T> {
    let first = iter.next()?;
    iter.next().is_none().then_some(first)
}
