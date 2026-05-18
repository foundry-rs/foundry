//! Match contract function shapes (ABI signatures, receiver contract type).

use solar::sema::hir::{self, ContractId, Expr, ExprKind, ItemId, Res, TypeKind, VariableId};

/// True if `id`'s elementary type matches the given ABI string.
pub fn is_elementary(hir: &hir::Hir<'_>, id: VariableId, abi: &str) -> bool {
    matches!(&hir.variable(id).ty.kind, TypeKind::Elementary(ty) if ty.to_abi_str() == abi)
}

/// Static contract type of a method-call receiver: a contract-typed variable
/// or an `IFoo(addr)` interface wrap.
pub fn receiver_contract_id(hir: &hir::Hir<'_>, recv: &Expr<'_>) -> Option<ContractId> {
    match &recv.kind {
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) => {
            if let TypeKind::Custom(ItemId::Contract(cid)) = hir.variable(*id).ty.kind {
                Some(cid)
            } else {
                None
            }
        }
        ExprKind::Call(
            Expr { kind: ExprKind::Ident([Res::Item(ItemId::Contract(cid))]), .. },
            ..,
        ) => Some(*cid),
        _ => None,
    }
}
