use super::NamedStructFields;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use solar::{
    interface::diagnostics::Applicability,
    sema::{
        Gcx,
        hir::{CallArgs, CallArgsKind, Expr, ExprKind, ItemId, Res},
    },
};

declare_forge_lint!(
    NAMED_STRUCT_FIELDS,
    Severity::Info,
    "named-struct-fields",
    "prefer initializing structs with named fields"
);

impl<'gcx> LateLintPass<'gcx> for NamedStructFields {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx Expr<'gcx>) {
        let ExprKind::Call(
            Expr { kind: ExprKind::Ident([Res::Item(ItemId::Struct(struct_id))]), span, .. },
            CallArgs { kind: CallArgsKind::Unnamed(args), .. },
            _,
        ) = &expr.kind
        else {
            return;
        };
        // A fix needs one argument per field and every snippet available; otherwise the
        // diagnostic is emitted without a suggestion.
        let fields = gcx.hir.strukt(*struct_id).fields;
        let fix = (!fields.is_empty() && fields.len() == args.len()).then(|| {
            let assignments = fields
                .iter()
                .zip(*args)
                .map(|(field, arg)| {
                    Some(format!(
                        "{}: {}",
                        gcx.hir.variable(*field).name?,
                        ctx.span_to_snippet(arg.span)?
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(format!("{}({{ {} }})", ctx.span_to_snippet(*span)?, assignments.join(", ")))
        });
        match fix.flatten() {
            Some(fix) => ctx.emit_with_suggestion(
                &NAMED_STRUCT_FIELDS,
                expr.span,
                Suggestion::fix(fix, Applicability::MachineApplicable)
                    .with_desc("consider using named fields"),
            ),
            None => ctx.emit(&NAMED_STRUCT_FIELDS, expr.span),
        }
    }
}
