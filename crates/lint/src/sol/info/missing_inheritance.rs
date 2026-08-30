use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint, info::MissingInheritance},
};
use solar::{
    interface::{Span, source_map::FileName},
    sema::{
        Gcx,
        hir::{ContractId, ContractKind, FunctionKind, ItemId, SourceId},
    },
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
        let hir = &gcx.hir;

        // Map every input source's HIR `SourceId` to the corresponding `ProjectSource` index, so
        // we only analyze (and emit against) user-provided files.
        let input_source_idx: HashMap<SourceId, usize> = hir
            .sources_enumerated()
            .filter_map(|(sid, src)| {
                let path = match &src.file.name {
                    FileName::Real(p) => p,
                    _ => return None,
                };
                let idx = sources.iter().position(|s| &s.path == path)?;
                Some((sid, idx))
            })
            .collect();

        if input_source_idx.is_empty() {
            return;
        }

        // Targets are restricted to user input; candidates span the whole HIR so dependency
        // interfaces (e.g. OpenZeppelin's `IERC20`) are still matched.
        let mut candidates: Vec<(ContractId, BTreeSet<[u8; 4]>)> = Vec::new();
        let mut targets: Vec<ContractId> = Vec::new();
        let mut selectors_by_contract: HashMap<ContractId, BTreeSet<[u8; 4]>> = HashMap::new();

        for cid in hir.contract_ids() {
            let contract = hir.contract(cid);
            if contract.linearization_failed() {
                continue;
            }

            let selectors: BTreeSet<[u8; 4]> =
                gcx.interface_functions(cid).all().iter().map(|f| f.selector.0).collect();
            selectors_by_contract.insert(cid, selectors.clone());

            let in_input = input_source_idx.contains_key(&contract.source);

            match contract.kind {
                ContractKind::Library => {}
                ContractKind::Interface => {
                    if !selectors.is_empty() {
                        candidates.push((cid, selectors));
                    }
                }
                ContractKind::AbstractContract => {
                    if is_signature_only(gcx, cid) {
                        if !selectors.is_empty() {
                            candidates.push((cid, selectors));
                        }
                    } else if in_input {
                        targets.push(cid);
                    }
                }
                ContractKind::Contract => {
                    if in_input {
                        targets.push(cid);
                    }
                }
            }
        }

        if candidates.is_empty() || targets.is_empty() {
            return;
        }

        for tid in targets {
            let target = hir.contract(tid);
            let Some(target_selectors) = selectors_by_contract.get(&tid) else { continue };
            if target_selectors.is_empty() {
                continue;
            }

            // Collect intended interfaces for this target.
            let mut intended: Vec<(ContractId, &BTreeSet<[u8; 4]>)> = Vec::new();
            for (iid, isel) in &candidates {
                if *iid == tid {
                    continue;
                }
                // Skip if already inherited (transitively).
                if target.linearized_bases.contains(iid) {
                    continue;
                }
                // Target must implement every selector of the candidate.
                if !isel.is_subset(target_selectors) {
                    continue;
                }
                // Skip if some inherited base of the target already covers the candidate.
                let subsumed_by_base =
                    target.linearized_bases.iter().filter(|b| **b != tid).any(|b| {
                        match selectors_by_contract.get(b) {
                            Some(bsel) => isel.is_subset(bsel),
                            None => false,
                        }
                    });
                if subsumed_by_base {
                    continue;
                }
                intended.push((*iid, isel));
            }

            if intended.is_empty() {
                continue;
            }

            // Deterministic dedupe by maximal selector set:
            // sort by descending selector count, tie-break by (path, contract name, id), then
            // drop any candidate whose selector set is a subset/superset of a kept one.
            intended.sort_by(|(a_id, a_sel), (b_id, b_sel)| {
                b_sel
                    .len()
                    .cmp(&a_sel.len())
                    .then_with(|| sort_key(hir, *a_id).cmp(&sort_key(hir, *b_id)))
            });

            let mut kept: Vec<(ContractId, &BTreeSet<[u8; 4]>)> = Vec::new();
            'outer: for (iid, isel) in intended {
                for (_, ksel) in &kept {
                    if isel.is_subset(ksel) || ksel.is_subset(isel) {
                        continue 'outer;
                    }
                }
                kept.push((iid, isel));
            }

            // Emit one diagnostic per kept interface, against the source containing the target.
            let Some(&src_idx) = input_source_idx.get(&target.source) else { continue };
            let source = &sources[src_idx];
            for (iid, _) in kept {
                let interface = hir.contract(iid);
                let msg = format!(
                    "contract `{}` implements interface `{}`'s external API but does not explicitly inherit from it",
                    target.name.as_str(),
                    interface.name.as_str(),
                );
                ctx.emit_with_msg(source, &MISSING_INHERITANCE, target.name.span, msg);
            }
        }
    }
}

/// Returns `true` if `cid` is an "interface-like" abstract contract: signature-only and free of
/// state, constructors, and modifier bodies. Such contracts mirror the role of `interface` and
/// should be considered as candidate interfaces for the missing-inheritance check.
fn is_signature_only<'gcx>(gcx: Gcx<'gcx>, cid: ContractId) -> bool {
    let hir = &gcx.hir;
    let contract = hir.contract(cid);

    let mut has_function = false;
    for &item_id in contract.items {
        match item_id {
            ItemId::Variable(_) => return false,
            ItemId::Function(fid) => {
                let func = hir.function(fid);
                match func.kind {
                    FunctionKind::Constructor | FunctionKind::Receive | FunctionKind::Fallback => {
                        return false;
                    }
                    FunctionKind::Modifier => {
                        if func.body.is_some() {
                            return false;
                        }
                    }
                    FunctionKind::Function => {
                        if func.body.is_some() {
                            return false;
                        }
                        has_function = true;
                    }
                }
            }
            _ => {}
        }
    }

    has_function
}

/// Stable sort key for deterministic dedupe ordering across runs.
fn sort_key<'hir>(hir: &'hir solar::sema::hir::Hir<'hir>, cid: ContractId) -> (Span, &'hir str) {
    let c = hir.contract(cid);
    (c.name.span, c.name.as_str())
}
