use super::ExternalFunction;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::is_builtin},
};
use solar::{
    ast::{ContractKind, DataLocation, Visibility},
    interface::{data_structures::Never, sym},
    sema::{
        Gcx,
        hir::{
            self, ContractId, Expr, ExprKind, FunctionId, Stmt, StmtKind, VariableId, Visit as _,
        },
        ty::TyKind,
    },
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    rc::Rc,
};

declare_forge_lint!(
    EXTERNAL_FUNCTION,
    Severity::Gas,
    "external-function",
    "`public` function can be declared `external`"
);

#[derive(Default)]
struct ProjectIndex {
    /// Functions referenced by name anywhere in the project: internal calls (`foo()`) and
    /// function pointers (`fn = foo;`).
    referenced: HashSet<FunctionId>,
    /// Contracts containing a `super` access, keyed by its resolved function.
    super_called: HashMap<FunctionId, HashSet<ContractId>>,
}

thread_local! {
    /// Project index keyed by the HIR address, which is stable for the whole lint run, so the
    /// index is built once instead of once per contract.
    static PROJECT_INDEX: RefCell<Option<(usize, Rc<ProjectIndex>)>> = const { RefCell::new(None) };
}

fn project_index(gcx: Gcx<'_>) -> Rc<ProjectIndex> {
    let key = std::ptr::from_ref(&gcx.hir) as usize;
    PROJECT_INDEX.with_borrow_mut(|slot| match slot {
        Some((cached_key, index)) if *cached_key == key => index.clone(),
        _ => slot.insert((key, Rc::new(build_project_index(gcx)))).1.clone(),
    })
}

fn build_project_index(gcx: Gcx<'_>) -> ProjectIndex {
    let hir = &gcx.hir;
    let mut builder = IndexBuilder { gcx, index: ProjectIndex::default(), contract: None };
    for func in hir.functions() {
        builder.contract = func.contract;
        let _ = builder.visit_function(func);
    }
    // State variable initializers run in the synthesized constructor.
    for var in hir.variables().filter(|var| var.is_state_variable()) {
        builder.contract = var.contract;
        let _ = builder.visit_var(var);
    }
    builder.index
}

struct IndexBuilder<'gcx> {
    gcx: Gcx<'gcx>,
    index: ProjectIndex,
    /// Contract being walked, to attribute `super.<name>` accesses to the caller.
    contract: Option<ContractId>,
}

impl<'gcx> hir::Visit<'gcx> for IndexBuilder<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            ExprKind::Ident(_) => self.index.referenced.extend(self.gcx.resolved_function(expr)),
            ExprKind::Member(base, _) if is_builtin(base, sym::super_) => {
                if let Some(cid) = self.contract
                    && let Some(fid) = self.gcx.resolved_function(expr)
                {
                    self.index.super_called.entry(fid).or_default().insert(cid);
                }
            }
            ExprKind::Member(base, _)
                if matches!(self.gcx.type_of_expr(base.id).map(|ty| ty.kind),
                    Some(TyKind::Type(ty)) if matches!(ty.kind, TyKind::Contract(_))) =>
            {
                self.index.referenced.extend(self.gcx.resolved_function(expr));
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

impl<'gcx> LateLintPass<'gcx> for ExternalFunction {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract_id: ContractId,
    ) {
        let contract = gcx.hir.contract(contract_id);
        // Libraries have different `external` semantics (delegatecall vs inlining); interfaces
        // have no bodies.
        if !ctx.is_lint_enabled(EXTERNAL_FUNCTION.id)
            || !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract)
            || contract.linearization_failed()
        {
            return;
        }
        let index = project_index(gcx);

        for fid in contract.functions() {
            let func = gcx.hir.function(fid);
            // Overrides can only widen visibility (`external` -> `public`), so the base chain is
            // flagged instead; abstract declarations must stay `public` to be overridable.
            let Some(name) = func.name else { continue };
            if func.visibility != Visibility::Public
                || !func.is_ordinary()
                || func.override_
                || func.body.is_none()
            {
                continue;
            }
            // Only reference parameters currently in `memory` yield meaningful savings.
            if !func.parameters.iter().any(|&p| is_memory_reference(gcx, p)) {
                continue;
            }
            let mut finder = ParamEscapeFinder { gcx, params: func.parameters };
            if finder.visit_function(func).is_break() {
                continue;
            }
            let super_called = index.super_called.iter().any(|(&target, callers)| {
                callers.iter().any(|&caller| {
                    gcx.hir.contracts_enumerated().any(|(cid, contract)| {
                        contract.linearized_bases.contains(&caller)
                            && gcx.resolve_super_function(cid, caller, target) == fid
                    })
                })
            });
            // A reference to this function's virtual target in a descendant also uses its slot.
            let override_referenced = gcx
                .hir
                .contracts_enumerated()
                .filter(|(cid, c)| *cid == contract_id || c.linearized_bases.contains(&contract_id))
                .any(|(cid, _)| index.referenced.contains(&gcx.resolve_virtual_function(cid, fid)));
            if !super_called && !override_referenced {
                ctx.emit(&EXTERNAL_FUNCTION, name.span);
            }
        }
    }
}

/// Breaks when a parameter is written, aliased, passed to a callee or modifier that could mutate
/// it through the internal-call memory-reference aliasing rule.
struct ParamEscapeFinder<'a, 'gcx> {
    gcx: Gcx<'gcx>,
    params: &'a [VariableId],
}

impl ParamEscapeFinder<'_, '_> {
    fn is_param(&self, expr: &Expr<'_>) -> bool {
        root_var_is(self.gcx, expr, &|v| self.params.contains(&v))
    }
}

impl<'gcx> hir::Visit<'gcx> for ParamEscapeFinder<'_, 'gcx> {
    type BreakValue = ();

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_modifier(&mut self, modifier: &'gcx hir::Modifier<'gcx>) -> ControlFlow<()> {
        if modifier.args.exprs().any(|arg| self.is_param(arg)) {
            return ControlFlow::Break(());
        }
        self.walk_modifier(modifier)
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<()> {
        if let StmtKind::DeclSingle(vid) = &stmt.kind
            && let var = self.gcx.hir.variable(*vid)
            && is_memory_reference(self.gcx, *vid)
            && var.initializer.is_some_and(|init| self.is_param(init))
        {
            return ControlFlow::Break(());
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<()> {
        let escapes = match &expr.kind {
            ExprKind::Assign(lhs, op, rhs) => {
                self.is_param(lhs)
                    || (op.is_none()
                        && root_var_is(self.gcx, lhs, &|v| {
                            let var = self.gcx.hir.variable(v);
                            var.is_local_variable() && is_memory_reference(self.gcx, v)
                        })
                        && self.is_param(rhs))
            }
            ExprKind::Delete(inner) => self.is_param(inner),
            ExprKind::Unary(op, inner) => op.kind.has_side_effects() && self.is_param(inner),
            ExprKind::Call(callee, args, opts) => {
                !self
                    .gcx
                    .type_of_expr(callee.id)
                    .is_some_and(|ty| matches!(ty.kind, TyKind::Type(_)))
                    && (args.exprs().any(|arg| self.is_param(arg))
                        || opts.is_some_and(|opts| {
                            opts.args.iter().any(|opt| self.is_param(&opt.value))
                        })
                        || matches!(&callee.peel_parens().kind, ExprKind::Member(receiver, _)
                            if self.is_param(receiver)))
            }
            _ => false,
        };
        if escapes {
            return ControlFlow::Break(());
        }
        self.walk_expr(expr)
    }
}

fn is_memory_reference(gcx: Gcx<'_>, var: VariableId) -> bool {
    gcx.type_of_item(var.into()).loc() == Some(DataLocation::Memory)
}

/// Whether the variable at the root of `expr` (through parens, members, indexes and slices)
/// satisfies `pred`.
fn root_var_is(gcx: Gcx<'_>, expr: &Expr<'_>, pred: &impl Fn(VariableId) -> bool) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(_) => gcx.resolved_variable(expr).is_some_and(pred),
        ExprKind::Member(base, _)
        | ExprKind::Payable(base)
        | ExprKind::Index(base, _)
        | ExprKind::Slice(base, ..) => root_var_is(gcx, base, pred),
        _ => false,
    }
}
