use super::FunctionSelectorCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::{data_structures::Never, kw, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, CallArgs, ContractId, ContractKind, Expr, ExprKind, ItemId, Stmt, StmtKind,
            TypeKind, Visit,
        },
        ty::{Ty, TyKind},
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(
    FUNCTION_SELECTOR_COLLISION,
    Severity::High,
    "function-selector-collision",
    "proxy and implementation functions have colliding selectors"
);

impl<'hir> LateLintPass<'hir> for FunctionSelectorCollision {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        proxy_id: ContractId,
    ) {
        let proxy = hir.contract(proxy_id);
        if proxy.kind != ContractKind::Contract || proxy.linearization_failed() {
            return;
        }
        let Some(fallback_id) = proxy.fallback else { return };
        let fallback = hir.function(fallback_id);
        let Some(body) = fallback.body else { return };

        let mut collector = DelegateTargetCollector {
            gcx,
            hir,
            fallback_input: unmodified_fallback_input(hir, fallback, body.stmts),
            targets: Vec::new(),
        };
        for stmt in body.stmts {
            let _ = collector.visit_stmt(stmt);
        }

        let proxy_functions = gcx.interface_functions(proxy_id);
        for implementation_id in collector.targets {
            if implementation_id == proxy_id {
                continue;
            }
            let implementation = hir.contract(implementation_id);
            if implementation.kind == ContractKind::Library || implementation.linearization_failed()
            {
                continue;
            }

            for proxy_function in proxy_functions.all() {
                for implementation_function in gcx.interface_functions(implementation_id).all() {
                    if proxy_function.selector != implementation_function.selector {
                        continue;
                    }
                    let proxy_signature = gcx.item_signature(proxy_function.id.into());
                    let implementation_signature =
                        gcx.item_signature(implementation_function.id.into());
                    if proxy_signature == implementation_signature {
                        continue;
                    }

                    let msg = format!(
                        "proxy function `{}.{proxy_signature}` collides with implementation function `{}.{implementation_signature}` at selector `{}`",
                        proxy.name.as_str(),
                        implementation.name.as_str(),
                        proxy_function.selector,
                    );
                    ctx.emit_with_msg(&FUNCTION_SELECTOR_COLLISION, proxy.name.span, msg);
                }
            }
        }
    }
}

fn unmodified_fallback_input<'hir>(
    hir: &'hir hir::Hir<'hir>,
    fallback: &'hir hir::Function<'hir>,
    stmts: &'hir [Stmt<'hir>],
) -> Option<hir::VariableId> {
    let input = fallback.parameters.first().copied()?;
    let mut collector = InputMutationCollector { hir, input, mutated: false };
    for stmt in stmts {
        let _ = collector.visit_stmt(stmt);
    }
    (!collector.mutated).then_some(input)
}

struct InputMutationCollector<'hir> {
    hir: &'hir hir::Hir<'hir>,
    input: hir::VariableId,
    mutated: bool,
}

impl<'hir> Visit<'hir> for InputMutationCollector<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Assign(lhs, _, _) = &expr.peel_parens().kind
            && lvalue_contains_var(lhs, self.input)
        {
            self.mutated = true;
        }
        self.walk_expr(expr)
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        if matches!(stmt.kind, StmtKind::AssemblyBlock(_)) {
            self.mutated = true;
            ControlFlow::Continue(())
        } else {
            self.walk_stmt(stmt)
        }
    }
}

fn lvalue_contains_var(expr: &Expr<'_>, target: hir::VariableId) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses
            .iter()
            .any(|res| matches!(res, hir::Res::Item(ItemId::Variable(id)) if *id == target)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().any(|expr| lvalue_contains_var(expr, target))
        }
        _ => false,
    }
}

struct DelegateTargetCollector<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    fallback_input: Option<hir::VariableId>,
    targets: Vec<ContractId>,
}

impl<'hir> Visit<'hir> for DelegateTargetCollector<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let Some(target) = delegated_contract(self.gcx, self.fallback_input, expr)
            && !self.targets.contains(&target)
        {
            self.targets.push(target);
        }
        self.walk_expr(expr)
    }
}

/// Returns the statically typed implementation contract for a proxy-style delegatecall.
fn delegated_contract<'hir>(
    gcx: Gcx<'hir>,
    fallback_input: Option<hir::VariableId>,
    expr: &'hir Expr<'hir>,
) -> Option<ContractId> {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return None };
    let ExprKind::Member(receiver, member) = &callee.peel_parens().kind else { return None };
    if member.name != kw::Delegatecall
        || gcx.builtin_callee(callee.id) != Some(Builtin::AddressDelegatecall)
        || !gcx.type_of_expr(receiver.peel_parens().id).is_some_and(ty_is_address)
        || !forwards_full_calldata(args, fallback_input)
    {
        return None;
    }
    typed_contract_behind_address_cast(gcx, receiver)
}

fn typed_contract_behind_address_cast<'hir>(
    gcx: Gcx<'hir>,
    expr: &'hir Expr<'hir>,
) -> Option<ContractId> {
    let expr = expr.peel_parens();
    if let Some(ty) = gcx.type_of_expr(expr.id)
        && let TyKind::Contract(id) = ty.peel_refs().kind
    {
        return Some(id);
    }
    match &expr.kind {
        ExprKind::Call(callee, args, _) if is_address_cast(callee) => {
            args.exprs().next().and_then(|arg| typed_contract_behind_address_cast(gcx, arg))
        }
        ExprKind::Payable(inner) => typed_contract_behind_address_cast(gcx, inner),
        _ => None,
    }
}

fn forwards_full_calldata(args: &CallArgs<'_>, fallback_input: Option<hir::VariableId>) -> bool {
    let Some(arg) = args.exprs().next() else { return false };
    if matches!(
        &arg.peel_parens().kind,
        ExprKind::Member(base, member)
            if member.name == sym::data && is_builtin_named(base, sym::msg)
    ) {
        return true;
    }
    let Some(fallback_input) = fallback_input else { return false };
    matches!(
        &arg.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(
                |res| matches!(res, hir::Res::Item(ItemId::Variable(id)) if *id == fallback_input),
            )
    )
}

fn is_builtin_named(expr: &Expr<'_>, name: solar::interface::Symbol) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| matches!(res, hir::Res::Builtin(b) if b.name() == name))
    )
}

fn is_address_cast(callee: &Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Type(hir::Type {
            kind: TypeKind::Elementary(hir::ElementaryType::Address(_)),
            ..
        })
    )
}

fn ty_is_address(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(hir::ElementaryType::Address(_)))
}
