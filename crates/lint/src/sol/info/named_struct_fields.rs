use solar::sema::hir::{CallArgs, CallArgsKind, Expr, ExprKind, ItemId, Res};

use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
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
        hir: &'hir solar::sema::hir::Hir<'hir>,
        expr: &'hir solar::sema::hir::Expr<'hir>,
    ) {
        let ExprKind::Call(
            Expr { kind: ExprKind::Ident([Res::Item(ItemId::Struct(struct_id))]), span, .. },
            CallArgs { kind: CallArgsKind::Unnamed(args), .. },
            _,
        ) = &expr.kind
        else {
            return;
        };

        let strukt = hir.strukt(*struct_id);
        let fields = &strukt.fields;

        // Basic sanity conditions for a consistent auto-fix
        if fields.len() != args.len() || fields.is_empty() {
            // Emit without suggestion
            ctx.emit(&NAMED_STRUCT_FIELDS, expr.span);
            return;
        }

        // Get struct name snippet and emit without suggestion if we can't get it
        let Some(struct_name_snippet) = ctx.span_to_snippet(*span) else {
            // Emit without suggestion if we can't get the struct name snippet
            ctx.emit(&NAMED_STRUCT_FIELDS, expr.span);
            return;
        };

        // Collect field names and corresponding argument source snippets
        let mut field_assignments = Vec::new();
        for (field_id, arg) in fields.iter().zip(args.iter()) {
            let field = hir.variable(*field_id);

            let Some((arg_snippet, field_name)) =
                ctx.span_to_snippet(arg.span).zip(field.name.map(|n| n.to_string()))
            else {
                // Emit without suggestion if we can't get argument snippet
                ctx.emit(&NAMED_STRUCT_FIELDS, expr.span);
                return;
            };

            field_assignments.push(format!("{field_name}: {arg_snippet}"));
        }

        ctx.emit_with_suggestion(
            &NAMED_STRUCT_FIELDS,
            expr.span,
            Suggestion::fix(
                format!("{}({{ {} }})", struct_name_snippet, field_assignments.join(", ")),
                solar::interface::diagnostics::Applicability::MachineApplicable,
            )
            .with_desc("consider using named fields"),
        );
    }
}
