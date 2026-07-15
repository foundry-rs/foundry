use super::FunctionSelectorCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::{data_structures::Never, kw, sym},
    sema::{
        Gcx,
        hir::{self, CallArgs, ContractId, ContractKind, Expr, ExprKind, TypeKind, Visit},
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
        let Some(body) = hir.function(fallback_id).body else { return };

        let mut collector = DelegateTargetCollector { gcx, hir, targets: Vec::new() };
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

struct DelegateTargetCollector<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    targets: Vec<ContractId>,
}

impl<'hir> Visit<'hir> for DelegateTargetCollector<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let Some(target) = delegated_contract(self.gcx, expr)
            && !self.targets.contains(&target)
        {
            self.targets.push(target);
        }
        self.walk_expr(expr)
    }
}

/// Returns the statically typed implementation contract for a proxy-style delegatecall.
fn delegated_contract<'hir>(gcx: Gcx<'hir>, expr: &'hir Expr<'hir>) -> Option<ContractId> {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return None };
    let ExprKind::Member(receiver, member) = &callee.peel_parens().kind else { return None };
    if member.name != kw::Delegatecall
        || !gcx.type_of_expr(receiver.peel_parens().id).is_some_and(ty_is_address)
        || !forwards_msg_data(args)
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

fn forwards_msg_data(args: &CallArgs<'_>) -> bool {
    args.exprs().next().is_some_and(|arg| {
        matches!(
            &arg.peel_parens().kind,
            ExprKind::Member(base, member)
                if member.name == sym::data && is_builtin_named(base, sym::msg)
        )
    })
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
