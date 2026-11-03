use super::DuplicateStorageNamespace;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use foundry_common::namespaced_storage::storage_namespace;
use solar::sema::{Gcx, hir};
use std::collections::BTreeMap;

declare_forge_lint!(
    DUPLICATE_STORAGE_NAMESPACE,
    Severity::Med,
    "duplicate-storage-namespace",
    "ERC-7201 namespace is declared more than once"
);

impl<'gcx> LateLintPass<'gcx> for DuplicateStorageNamespace {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract: &'gcx hir::Contract<'gcx>,
    ) {
        let Some((&id, bases)) = contract.linearized_bases.split_first() else { return };
        for (namespace, structs) in namespaced_structs(gcx, contract.linearized_bases) {
            if structs.len() < 2 {
                continue;
            }
            let first = gcx.item_canonical_name(structs[0]);
            // Bases come first, so every struct declared here after the first occurrence collides.
            for &strukt in &structs[1..] {
                let strukt = gcx.hir.strukt(strukt);
                if strukt.contract == Some(id) {
                    ctx.emit_with_msg(
                        &DUPLICATE_STORAGE_NAMESPACE,
                        strukt.name.span,
                        format!(
                            "ERC-7201 namespace `{namespace}` is already declared by `{first}`"
                        ),
                    );
                }
            }
            // Inherited declarations collide here only if no single base already combines them;
            // otherwise the collision is reported in that base.
            let inherited = &structs
                [..structs.partition_point(|&strukt| gcx.hir.strukt(strukt).contract != Some(id))];
            if let &[first, .., last] = inherited
                && !bases.iter().any(|&base| {
                    let hierarchy = gcx.hir.contract(base).linearized_bases;
                    inherited.iter().all(|&strukt| {
                        gcx.hir
                            .strukt(strukt)
                            .contract
                            .is_some_and(|declared_in| hierarchy.contains(&declared_in))
                    })
                })
            {
                ctx.emit_with_msg(
                    &DUPLICATE_STORAGE_NAMESPACE,
                    contract.name.span,
                    format!(
                        "ERC-7201 namespace `{namespace}` is declared by both `{}` and `{}`",
                        gcx.item_canonical_name(first),
                        gcx.item_canonical_name(last)
                    ),
                );
            }
        }
    }
}

/// Returns the namespaced structs declared in `contracts`, grouped by namespace, most base first.
///
/// Malformed ERC-7201 annotations are skipped: they are not duplicates, and the storage layout
/// decoder rejects them with a dedicated error.
fn namespaced_structs<'gcx>(
    gcx: Gcx<'gcx>,
    contracts: &[hir::ContractId],
) -> BTreeMap<&'gcx str, Vec<hir::StructId>> {
    let mut namespaces = BTreeMap::<_, Vec<_>>::new();
    for &contract in contracts.iter().rev() {
        for id in gcx.hir.contract(contract).items.iter().filter_map(|item| item.as_struct()) {
            if let Ok(Some(namespace)) = storage_namespace(gcx, gcx.hir.strukt(id)) {
                namespaces.entry(namespace).or_default().push(id);
            }
        }
    }
    namespaces
}
