//! Match contract function shapes (ABI signatures, receiver contract type).

use solar::sema::hir::{self, ContractId, Expr, ExprKind, ItemId, Res, TypeKind, VariableId};

/// True if `id`'s elementary type matches the given ABI string.
pub fn is_elementary(hir: &hir::Hir<'_>, id: VariableId, abi: &str) -> bool {
    matches!(&hir.variable(id).ty.kind, TypeKind::Elementary(ty) if ty.to_abi_str() == abi)
}

/// Static contract type of a method-call receiver: a contract-typed variable,
/// a direct contract reference (libraries), or an `IFoo(addr)` interface wrap.
pub fn receiver_contract_id(hir: &hir::Hir<'_>, recv: &Expr<'_>) -> Option<ContractId> {
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
