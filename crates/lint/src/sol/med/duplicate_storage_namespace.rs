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
            // Report a collision once, in the first contract whose hierarchy contains it.
            if structs.len() < 2
                || bases.iter().any(|&base| {
                    namespaced_structs(gcx, gcx.hir.contract(base).linearized_bases)
                        .get(namespace)
                        .is_some_and(|structs| structs.len() > 1)
                })
            {
                continue;
            }
            let first = qualified_name(gcx, structs[0]);
            let (span, msg) = match structs
                .iter()
                .rev()
                .find(|&&s| gcx.hir.strukt(s).contract == Some(id))
            {
                Some(&declared_here) => (
                    gcx.hir.strukt(declared_here).name.span,
                    format!("ERC-7201 namespace `{namespace}` is already declared by `{first}`"),
                ),
                None => (
                    contract.name.span,
                    format!(
                        "ERC-7201 namespace `{namespace}` is declared by both `{first}` and `{}`",
                        qualified_name(gcx, structs[1])
                    ),
                ),
            };
            ctx.emit_with_msg(&DUPLICATE_STORAGE_NAMESPACE, span, msg);
        }
    }
}

/// Returns the namespaced structs declared in `contracts`, grouped by namespace, most base first.
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

fn qualified_name(gcx: Gcx<'_>, id: hir::StructId) -> String {
    let strukt = gcx.hir.strukt(id);
    match strukt.contract {
        Some(contract) => format!("{}.{}", gcx.hir.contract(contract).name, strukt.name),
        None => strukt.name.to_string(),
    }
}
