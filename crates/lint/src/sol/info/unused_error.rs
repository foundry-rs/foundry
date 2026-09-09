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

        // Only errors declared in user-provided files are reported, while uses are collected
        // across the whole unit, so an error declared here and reverted in a dependency (or the
        // other way around) is attributed correctly.
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

        let mut collector = UsedErrorCollector { gcx, current_source: None, used: HashSet::new() };
        for source_id in gcx.hir.source_ids() {
            collector.current_source = Some(source_id);
            let _ = collector.visit_nested_source(source_id);
        }

        for error_id in gcx.hir.error_ids() {
            let error = gcx.hir.error(error_id);
            let Some(&src_idx) = input_source_idx.get(&error.source) else { continue };
            // Errors declared in interfaces and abstract contracts are ABI surface meant for
            // implementers and off-chain consumers, which may live outside the compiled sources.
            let abi_surface = error.contract.is_some_and(|id| {
                matches!(
                    gcx.hir.contract(id).kind,
                    ContractKind::Interface | ContractKind::AbstractContract
                )
            });
            if !abi_surface && !collector.used.contains(&error_id) {
                ctx.emit(&sources[src_idx], &UNUSED_ERROR, error.span);
            }
        }
    }
}

/// A named scope a qualified member can resolve against.
enum MemberScope {
    /// A contract or library: its declared items.
    Contract(hir::ContractId),
    /// A module alias: Solar's resolved scope for that source.
    Module(hir::SourceId),
}

/// Collects every error referenced by an expression anywhere in the unit.
///
/// Resolved identifiers cover almost every use: the lowering resolves the full path of
/// `revert Lib.Err(...)` into a single `Ident`, and `require(cond, Err(...))` or `Err.selector`
/// reference the error through a resolved `Ident` as well. The one exception is a qualified
/// member access such as `Lib.Err.selector`: the inner `Err` segment carries no resolution in
/// the HIR, so it is resolved against the scope its base designates: the items of a contract,
/// or, for a module alias, Solar's resolved source scope, which binds exactly the names the
/// file declares and imports (aliases included).
struct UsedErrorCollector<'gcx> {
    gcx: Gcx<'gcx>,
    /// The source being walked: module member lookups are made from its viewpoint.
    current_source: Option<hir::SourceId>,
    used: HashSet<hir::ErrorId>,
}

impl<'gcx> hir::Visit<'gcx> for UsedErrorCollector<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx hir::Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        let hir = self.hir();
        match &expr.kind {
            // Symbols can be overloaded: consider every resolution.
            hir::ExprKind::Ident(resolutions) => {
                self.used.extend(resolutions.iter().filter_map(|res| match res {
                    hir::Res::Item(hir::ItemId::Error(error_id)) => Some(*error_id),
                    _ => None,
                }));
            }
            hir::ExprKind::Member(base, member) => {
                for scope in self.base_scopes(base) {
                    match scope {
                        MemberScope::Contract(contract_id) => {
                            self.used.extend(hir.contract(contract_id).items.iter().filter_map(
                                |item| match item {
                                    hir::ItemId::Error(id)
                                        if hir.error(*id).name.name == member.name =>
                                    {
                                        Some(*id)
                                    }
                                    _ => None,
                                },
                            ));
                        }
                        // In the resolved scope an import alias binds under its local name to
                        // the exact declaration: mark that error, not a same-name lookalike.
                        MemberScope::Module(source_id) => {
                            for ty in self.module_members_named(source_id, member.name) {
                                if let TyKind::Error(_, error_id) = ty.kind {
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
    /// The types of the members named `name` in the resolved scope of module `source_id`: the
    /// file's own declarations plus the names its imports bind, under their local names.
    fn module_members_named(&self, source_id: hir::SourceId, name: Symbol) -> Vec<Ty<'gcx>> {
        let module_ty = self.gcx.type_of_res(hir::Res::Namespace(source_id));
        let Some(current_source) = self.current_source else { return Vec::new() };
        self.gcx
            .members_of(module_ty, current_source, None)
            .filter_map(|member| (member.name == name).then_some(member.ty))
            .collect()
    }

    /// The named scopes `expr` can designate: a contract or library through a resolved
    /// identifier, a module alias, or a member chain leading to one (`NS.Lib`, `NS.Inner`).
    fn base_scopes(&self, expr: &hir::Expr<'_>) -> Vec<MemberScope> {
        match &expr.kind {
            hir::ExprKind::Ident(resolutions) => resolutions
                .iter()
                .filter_map(|res| match res {
                    hir::Res::Item(hir::ItemId::Contract(id)) => Some(MemberScope::Contract(*id)),
                    hir::Res::Namespace(id) => Some(MemberScope::Module(*id)),
                    _ => None,
                })
                .collect(),
            // Contracts do not nest named scopes, so only module bases descend. A contract is a
            // type-namespace item, so its member type comes wrapped as `Type(Contract(..))`; a
            // nested module alias comes as a bare `Module(..)`.
            hir::ExprKind::Member(inner_base, name) => self
                .base_scopes(inner_base)
                .into_iter()
                .filter_map(|scope| match scope {
                    MemberScope::Module(source_id) => Some(source_id),
                    MemberScope::Contract(_) => None,
                })
                .flat_map(|source_id| self.module_members_named(source_id, name.name))
                .filter_map(|ty| {
                    let ty = match ty.kind {
                        TyKind::Type(inner) => inner,
                        _ => ty,
                    };
                    match ty.kind {
                        TyKind::Contract(id) => Some(MemberScope::Contract(id)),
                        TyKind::Module(id) => Some(MemberScope::Module(id)),
                        _ => None,
                    }
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}
