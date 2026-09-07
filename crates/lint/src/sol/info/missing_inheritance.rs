use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint, info::MissingInheritance},
};
use solar::{
    interface::source_map::FileName,
    sema::hir::{ContractId, ContractKind, FunctionKind, Hir, ItemId},
};
use std::collections::{BTreeSet, HashMap};

declare_forge_lint!(
    MISSING_INHERITANCE,
    Severity::Info,
    "missing-inheritance",
    "contract implements an interface's external API but does not explicitly inherit from it"
);

impl<'ast> ProjectLintPass<'ast> for MissingInheritance {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]) {
        if !ctx.is_lint_enabled(MISSING_INHERITANCE.id()) {
            return;
        }
        let gcx = ctx.gcx();

        // Only user-provided files are analyzed (and emitted against).
        let input_source_idx: HashMap<_, _> = gcx
            .hir
            .sources_enumerated()
            .filter_map(|(sid, src)| {
                let FileName::Real(path) = &src.file.name else { return None };
                Some((sid, sources.iter().position(|s| &s.path == path)?))
            })
            .collect();
        if input_source_idx.is_empty() {
            return;
        }

        // Targets are restricted to user input; candidates span the whole HIR so dependency
        // interfaces (e.g. OpenZeppelin's `IERC20`) are still matched.
        let mut selectors = HashMap::<ContractId, BTreeSet<[u8; 4]>>::new();
        let mut candidates = Vec::new();
        let mut targets = Vec::new();
        for cid in gcx.hir.contract_ids() {
            let contract = gcx.hir.contract(cid);
            if contract.linearization_failed() {
                continue;
            }
            let sels: BTreeSet<_> =
                gcx.interface_functions(cid).all().iter().map(|f| f.selector.0).collect();
            let interface_like = match contract.kind {
                ContractKind::Interface => true,
                ContractKind::AbstractContract => is_signature_only(&gcx.hir, cid),
                ContractKind::Contract | ContractKind::Library => false,
            };
            if interface_like {
                if !sels.is_empty() {
                    candidates.push(cid);
                }
            } else if !contract.kind.is_library() && input_source_idx.contains_key(&contract.source)
            {
                targets.push(cid);
            }
            selectors.insert(cid, sels);
        }

        // Stable sort key for deterministic dedupe ordering across runs.
        let sort_key = |cid| {
            let name = &gcx.hir.contract(cid).name;
            (name.span, name.as_str())
        };
        for tid in targets {
            let target = gcx.hir.contract(tid);
            let target_sels = &selectors[&tid];
            if target_sels.is_empty() {
                continue;
            }
            // The target must implement every selector of the candidate, without already
            // inheriting it (transitively) or an inherited base covering the candidate.
            let mut intended: Vec<ContractId> = candidates
                .iter()
                .copied()
                .filter(|&iid| {
                    let isel = &selectors[&iid];
                    iid != tid
                        && !target.linearized_bases.contains(&iid)
                        && isel.is_subset(target_sels)
                        && !target.linearized_bases.iter().any(|b| {
                            *b != tid && selectors.get(b).is_some_and(|bsel| isel.is_subset(bsel))
                        })
                })
                .collect();
            // Deterministic dedupe by maximal selector set: sort by descending selector count,
            // tie-break by (span, name), then drop any candidate whose selector set is a
            // subset/superset of a kept one.
            intended.sort_by(|&a, &b| {
                selectors[&b]
                    .len()
                    .cmp(&selectors[&a].len())
                    .then_with(|| sort_key(a).cmp(&sort_key(b)))
            });
            let mut kept: Vec<ContractId> = Vec::new();
            for iid in intended {
                let isel = &selectors[&iid];
                if !kept.iter().any(|kid| {
                    let ksel = &selectors[kid];
                    isel.is_subset(ksel) || ksel.is_subset(isel)
                }) {
                    kept.push(iid);
                }
            }

            let Some(&src_idx) = input_source_idx.get(&target.source) else { continue };
            for iid in kept {
                let msg = format!(
                    "contract `{}` implements interface `{}`'s external API but does not explicitly inherit from it",
                    target.name.as_str(),
                    gcx.hir.contract(iid).name.as_str(),
                );
                ctx.emit_with_msg(&sources[src_idx], &MISSING_INHERITANCE, target.name.span, msg);
            }
        }
    }
}

/// True if `cid` is an "interface-like" abstract contract: signature-only and free of state,
/// constructors, and modifier bodies. Such contracts mirror the role of `interface` and are
/// candidate interfaces for the missing-inheritance check.
fn is_signature_only(hir: &Hir<'_>, cid: ContractId) -> bool {
    let mut has_function = false;
    for &item_id in hir.contract(cid).items {
        match item_id {
            ItemId::Variable(_) => return false,
            ItemId::Function(fid) => {
                let func = hir.function(fid);
                match func.kind {
                    FunctionKind::Function if func.body.is_none() => has_function = true,
                    FunctionKind::Modifier if func.body.is_none() => {}
                    _ => return false,
                }
            }
            _ => {}
        }
    }
    has_function
}
