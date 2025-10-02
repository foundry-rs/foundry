use solar::sema::hir::{CallArgs, CallArgsKind, Expr, ExprKind, ItemId, Res};

use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, info::NamedStructFields},
};

declare_forge_lint!(
    NAMED_STRUCT_FIELDS,
    Severity::Info,
    "named-struct-fields",
    "prefer initializing structs with named fields"
);

impl<'hir> LateLintPass<'hir> for NamedStructFields {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        _hir: &'hir solar::sema::hir::Hir<'hir>,
        expr: &'hir solar::sema::hir::Expr<'hir>,
    ) {
        if let ExprKind::Call(
            Expr { kind: ExprKind::Ident([Res::Item(ItemId::Struct(_struct_id))]), .. },
            CallArgs { kind: CallArgsKind::Unnamed(_args), .. },
            _,
        ) = &expr.kind
        {
            ctx.emit(&NAMED_STRUCT_FIELDS, expr.span);
        }
    }
}
