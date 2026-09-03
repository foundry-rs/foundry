use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint, info::UnusedError},
};
use solar::{
    ast::ContractKind,
    interface::{Symbol, data_structures::Never, source_map::FileName},
    sema::{
        Gcx,
        hir::{self, Visit as _},
        ty::{Ty, TyKind},
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

declare_forge_lint!(UNUSED_ERROR, Severity::Info, "unused-error", "custom error is never used");

impl<'ast> ProjectLintPass<'ast> for UnusedError {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]) {
        if !ctx.is_lint_enabled(UNUSED_ERROR.id()) {
            return;
        }

        let gcx = ctx.gcx();
        let hir = &gcx.hir;

        // Map every input source's HIR `SourceId` to the corresponding `ProjectSource` index, so
        // only errors declared in user-provided files are reported. Uses are still collected
        // across the whole unit, so an error declared here and reverted in a dependency (or the
        // other way around) is attributed correctly.
        let input_source_idx: HashMap<hir::SourceId, usize> = hir
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

        let used = collect_used_errors(gcx);

        // Report input-file error declarations that no expression in the unit references.
        for error_id in hir.error_ids() {
            let error = hir.error(error_id);
            let Some(&src_idx) = input_source_idx.get(&error.source) else { continue };
            // Errors declared in interfaces and abstract contracts are ABI surface meant for
            // implementers and off-chain consumers, which may live outside the compiled sources.
            if let Some(contract_id) = error.contract
                && matches!(
                    hir.contract(contract_id).kind,
                    ContractKind::Interface | ContractKind::AbstractContract
                )
            {
                continue;
            }
            if !used.contains(&error_id) {
                ctx.emit(&sources[src_idx], &UNUSED_ERROR, error.span);
            }
        }
    }
}

/// Collects every [`hir::ErrorId`] referenced by an expression anywhere in the unit.
///
/// Resolved identifiers cover almost every use: the lowering resolves the full path of
/// `revert Lib.Err(...)` into a single `Ident`, and `require(cond, Err(...))` or `Err.selector`
/// reference the error through a resolved `Ident` as well. The one exception is a qualified
/// member access such as `Lib.Err.selector`: the inner `Err` segment carries no resolution in
/// the HIR, so it is resolved against the scope its base designates: the items of a contract,
/// or, for a module alias, Solar's resolved source scope, which binds exactly the names the
/// file declares and imports (aliases included) rather than the raw items of the transitively
/// imported files.
fn collect_used_errors(gcx: Gcx<'_>) -> HashSet<hir::ErrorId> {
    let hir = &gcx.hir;
    let mut used = HashSet::new();
    // Walk every source of the unit: functions, modifiers, and variable initializers.
    for source_id in hir.source_ids() {
        let mut collector = UsedErrorCollector { gcx, hir, current_source: source_id, used };
        let _ = collector.visit_nested_source(source_id);
        used = collector.used;
    }
    used
}

/// A named scope a qualified member can resolve against.
enum MemberScope {
    /// A contract or library: its declared items.
    Contract(hir::ContractId),
    /// A module alias: Solar's resolved scope for that source.
    Module(hir::SourceId),
}

struct UsedErrorCollector<'gcx> {
    gcx: Gcx<'gcx>,
    hir: &'gcx hir::Hir<'gcx>,
    /// The source being walked: module member lookups are made from its viewpoint.
    current_source: hir::SourceId,
    used: HashSet<hir::ErrorId>,
}

impl<'gcx> hir::Visit<'gcx> for UsedErrorCollector<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'gcx hir::Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            // Direct resolved reference: `revert Err()`, `revert Lib.Err()` (fully resolved by
            // the lowering), `require(cond, Err())`, `Err.selector`, ...
            hir::ExprKind::Ident(resolutions) => {
                // Symbols can be overloaded: consider every resolution.
                for res in *resolutions {
                    if let hir::Res::Item(hir::ItemId::Error(error_id)) = res {
                        self.used.insert(*error_id);
                    }
                }
            }
            // `Lib.Err.selector`: the `Err` member carries no resolution, so resolve it against
            // the scope its base designates.
            hir::ExprKind::Member(base, member) => {
                for scope in self.base_scopes(base) {
                    match scope {
                        MemberScope::Contract(contract_id) => {
                            self.mark_named_error(contract_id, member.name);
                        }
                        // In the resolved scope an import alias binds under its local name to
                        // the exact declaration: mark that error, not a same-name lookalike.
                        MemberScope::Module(source_id) => {
                            for member_ty in self.module_members_named(source_id, member.name) {
                                if let TyKind::Error(_, error_id) = member_ty.kind {
                                    self.used.insert(error_id);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

impl<'gcx> UsedErrorCollector<'gcx> {
    /// Marks as used the errors of the contract whose name matches `name`.
    fn mark_named_error(&mut self, contract_id: hir::ContractId, name: Symbol) {
        // Scopes cannot hold two same-name errors, but scan every item to stay conservative.
        for item_id in self.hir.contract(contract_id).items {
            if let hir::ItemId::Error(error_id) = item_id
                && self.hir.error(*error_id).name.name == name
            {
                self.used.insert(*error_id);
            }
        }
    }

    /// Returns the types of the members named `name` in the resolved scope of module
    /// `source_id`.
    ///
    /// `members_of` on a module type iterates Solar's resolved source scope: the file's own
    /// declarations plus the names its imports actually bind, under their local (alias) names.
    fn module_members_named(&self, source_id: hir::SourceId, name: Symbol) -> Vec<Ty<'gcx>> {
        let module_ty = self.gcx.type_of_res(hir::Res::Namespace(source_id));
        self.gcx
            .members_of(module_ty, self.current_source, None)
            .filter(|member| member.name == name)
            .map(|member| member.ty)
            .collect()
    }

    /// Returns the named scopes `expr` can designate: a contract or library through a resolved
    /// identifier, a module alias, or a member chain leading to one (`NS.Lib`, `NS.Inner`).
    fn base_scopes(&self, expr: &hir::Expr<'_>) -> Vec<MemberScope> {
        let mut scopes = Vec::new();
        match &expr.kind {
            hir::ExprKind::Ident(resolutions) => {
                for res in *resolutions {
                    match res {
                        hir::Res::Item(hir::ItemId::Contract(contract_id)) => {
                            scopes.push(MemberScope::Contract(*contract_id));
                        }
                        hir::Res::Namespace(source_id) => {
                            scopes.push(MemberScope::Module(*source_id));
                        }
                        _ => {}
                    }
                }
            }
            // A chained scope access (`NS.Lib`, `NS.Inner`): resolve the name in the scopes of
            // the base. Contracts do not nest named scopes, so only module bases descend.
            hir::ExprKind::Member(inner_base, name) => {
                for scope in self.base_scopes(inner_base) {
                    if let MemberScope::Module(source_id) = scope {
                        for member_ty in self.module_members_named(source_id, name.name) {
                            // A contract or library is a type-namespace item, so its member
                            // type comes wrapped as `Type(Contract(..))`; a nested module
                            // alias comes as a bare `Module(..)`.
                            let member_ty = match member_ty.kind {
                                TyKind::Type(inner) => inner,
                                _ => member_ty,
                            };
                            match member_ty.kind {
                                TyKind::Contract(contract_id) => {
                                    scopes.push(MemberScope::Contract(contract_id));
                                }
                                TyKind::Module(inner_source) => {
                                    scopes.push(MemberScope::Module(inner_source));
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        scopes
    }
}
