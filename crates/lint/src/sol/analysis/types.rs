//! Type probes using Solar's type-checker results.

use solar::sema::{
    Gcx, Ty,
    hir::{self, ContractId, Expr, TypeKind, VariableId},
    ty::TyKind,
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

/// Static contract type of a method-call receiver or direct contract/library reference.
pub fn receiver_contract_id<'gcx>(gcx: Gcx<'gcx>, recv: &Expr<'gcx>) -> Option<ContractId> {
    gcx.type_of_expr(recv.peel_parens().id).and_then(ty_contract_id)
}

/// The only element of `iter`, or `None` when it has zero or several.
pub fn unique<T>(mut iter: impl Iterator<Item = T>) -> Option<T> {
    let first = iter.next()?;
    iter.next().is_none().then_some(first)
}
