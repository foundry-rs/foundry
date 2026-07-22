use super::ExternalFunction;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, DataLocation, UnOpKind, Visibility},
    sema::{
        Gcx,
        hir::{self, ContractId, ExprKind, StmtKind, VariableId, Visit as _},
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(
    EXTERNAL_FUNCTION,
    Severity::Gas,
    "external-function",
    "public function can be declared external"
);

impl<'hir> LateLintPass<'hir> for ExternalFunction {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        contract_id: ContractId,
    ) {
        if !ctx.is_lint_enabled(EXTERNAL_FUNCTION.id) {
            return;
        }

        let contract = hir.contract(contract_id);

        // Libraries have different `external` semantics (delegatecall vs inlining); interfaces
        // have no bodies.
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract) {
            return;
        }
        if contract.linearization_failed() {
            return;
        }

        for fid in contract.functions() {
            let func = hir.function(fid);

            // `is_ordinary()` excludes constructor / fallback / receive / modifier.
            if func.visibility != Visibility::Public || !func.is_ordinary() {
                continue;
            }
            // Solidity only allows widening visibility on override (`external` -> `public`),
            // never tightening. Flag the base chain instead.
            if func.override_ {
                continue;
            }
            // Abstract declarations must stay `public` so derived contracts can override them.
            let Some(body) = func.body else { continue };

            // Only flag when at least one parameter is a reference type currently in `memory`;
            // value-only signatures yield negligible savings.
            let has_memory_reference_param = func.parameters.iter().any(|&pid| {
                let p = hir.variable(pid);
                p.ty.kind.is_reference_type() && p.data_location == Some(DataLocation::Memory)
            });
            if !has_memory_reference_param {
                continue;
            }

            if body_escapes_params(gcx, hir, &body, func.parameters)
                || modifier_args_reference_params(gcx, func.modifiers, func.parameters)
            {
                continue;
            }

            let Some(name) = func.name else { continue };

            // A reference to this implementation or an override in a derivative counts as an
            // internal use of the virtual slot.
            if any_override_referenced(gcx, contract_id, fid) {
                continue;
            }

            ctx.emit(&EXTERNAL_FUNCTION, name.span);
        }
    }
}

fn any_override_referenced(
    gcx: Gcx<'_>,
    contract_id: ContractId,
    base_id: hir::FunctionId,
) -> bool {
    let hir = &gcx.hir;
    let base = hir.function(base_id);
    let Some(base_name) = base.name else { return false };
    let parameters = gcx.item_parameter_types(base_id);
    let references = gcx.function_reference_index();

    for (other_cid, other_contract) in hir.contracts_enumerated() {
        if other_cid != contract_id && !other_contract.linearized_bases.contains(&contract_id) {
            continue;
        }
        for fid in other_contract.functions() {
            if references.is_internally_referenced(fid) {
                let other = hir.function(fid);
                if let Some(other_name) = other.name
                    && other_name.name == base_name.name
                    && gcx.item_parameter_types(fid) == parameters
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Returns `true` if any param is written, aliased, or passed to a callee that could
/// mutate it via the internal-call memory-reference aliasing rule.
fn body_escapes_params<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    body: &hir::Block<'hir>,
    params: &[VariableId],
) -> bool {
    let mut finder = ParamEscapeFinder { gcx, hir, params };
    body.stmts.iter().any(|stmt| finder.visit_stmt(stmt).is_break())
}

/// Returns `true` if a modifier invocation binds one of `params` to a mutable `memory`
/// parameter. Value parameters cannot alias caller memory.
fn modifier_args_reference_params<'hir>(
    gcx: Gcx<'hir>,
    modifiers: &'hir [hir::Modifier<'hir>],
    params: &[VariableId],
) -> bool {
    modifiers.iter().any(|modifier| {
        let Some(callee) = modifier.id.as_function() else {
            return modifier.args.exprs().any(|arg| gcx.expr_root_is_any_variable(arg, params));
        };
        gcx.hir.function(callee).parameters.iter().enumerate().any(|(index, &parameter)| {
            parameter_is_mutable_memory_reference(&gcx.hir, parameter)
                && gcx
                    .modifier_arg(modifier, index)
                    .is_some_and(|arg| gcx.expr_root_is_any_variable(arg, params))
        })
    })
}

struct ParamEscapeFinder<'a, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    params: &'a [VariableId],
}

impl<'hir> hir::Visit<'hir> for ParamEscapeFinder<'_, 'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        if let StmtKind::DeclSingle(vid) = &stmt.kind {
            let var = self.hir.variable(*vid);
            if let Some(init) = var.initializer
                && var.ty.kind.is_reference_type()
                && var.data_location == Some(DataLocation::Memory)
                && self.gcx.expr_root_is_any_variable(init, self.params)
            {
                return ControlFlow::Break(());
            }
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            ExprKind::Assign(lhs, op, rhs) => {
                if self.gcx.expr_root_is_any_variable(lhs, self.params) {
                    return ControlFlow::Break(());
                }
                if op.is_none()
                    && lhs_is_local_memory_reference(self.gcx, self.hir, lhs)
                    && self.gcx.expr_root_is_any_variable(rhs, self.params)
                {
                    return ControlFlow::Break(());
                }
            }
            ExprKind::Delete(inner) if self.gcx.expr_root_is_any_variable(inner, self.params) => {
                return ControlFlow::Break(());
            }
            ExprKind::Unary(op, inner)
                if matches!(
                    op.kind,
                    UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                ) && self.gcx.expr_root_is_any_variable(inner, self.params) =>
            {
                return ControlFlow::Break(());
            }
            ExprKind::Call(callee, args, opts)
                if self.gcx.call_info(expr).is_some_and(|info| {
                    info.is_direct_internal() || info.is_indirect_internal()
                }) =>
            {
                let info = self.gcx.call_info(expr).unwrap();
                if info.is_direct_internal()
                    && let Some(callee) = info.function()
                {
                    for (index, &parameter) in
                        self.hir.function(callee).parameters.iter().enumerate()
                    {
                        if parameter_is_mutable_memory_reference(self.hir, parameter)
                            && self.gcx.call_arg_for_param(expr, index).is_some_and(|arg| {
                                self.gcx.expr_root_is_any_variable(arg, self.params)
                            })
                        {
                            return ControlFlow::Break(());
                        }
                    }
                    return self.walk_expr(expr);
                }

                // An indirect internal call has no selected declaration to prove that its
                // parameters are non-aliasing, so retain the conservative fallback.
                for arg in args.exprs() {
                    if self.gcx.expr_root_is_any_variable(arg, self.params) {
                        return ControlFlow::Break(());
                    }
                }
                if let Some(opts) = opts {
                    for opt in opts.args {
                        if self.gcx.expr_root_is_any_variable(&opt.value, self.params) {
                            return ControlFlow::Break(());
                        }
                    }
                }
                if let ExprKind::Member(receiver, _) = &callee.peel_parens().kind
                    && self.gcx.expr_root_is_any_variable(receiver, self.params)
                {
                    return ControlFlow::Break(());
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

fn parameter_is_mutable_memory_reference(hir: &hir::Hir<'_>, parameter: VariableId) -> bool {
    let parameter = hir.variable(parameter);
    parameter.ty.kind.is_reference_type() && parameter.data_location == Some(DataLocation::Memory)
}

/// Returns `true` if the root of `lhs` resolves to a local variable with reference type
/// in `memory`.
fn lhs_is_local_memory_reference<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    lhs: &'hir hir::Expr<'hir>,
) -> bool {
    gcx.expr_root_variable(lhs).is_some_and(|variable| {
        let variable = hir.variable(variable);
        variable.is_local_variable()
            && variable.ty.kind.is_reference_type()
            && variable.data_location == Some(DataLocation::Memory)
    })
}
