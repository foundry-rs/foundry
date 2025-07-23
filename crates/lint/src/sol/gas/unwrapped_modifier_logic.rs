use super::UnwrappedModifierLogic;
use crate::{
    linter::{EarlyLintPass, LintContext, Snippet},
    sol::{Severity, SolLint},
};
use solar_ast::{ExprKind, ItemFunction, Span, Stmt, StmtKind};

declare_forge_lint!(
    UNWRAPPED_MODIFIER_LOGIC,
    Severity::Gas,
    "unwrapped-modifier-logic",
    "wrap modifier logic to reduce code size"
);

impl<'ast> EarlyLintPass<'ast> for UnwrappedModifierLogic {
    fn check_item_function(&mut self, ctx: &LintContext<'_>, func: &'ast ItemFunction<'ast>) {
        // Only check modifiers with a body and name.
        let (body, name) = match (func.body.as_ref(), func.header.name) {
            (Some(body), Some(name)) if func.kind.is_modifier() => (body, name),
            _ => return,
        };

        // Split statements into before and after the placeholder `_`.
        let stmts = &body.stmts[..];
        let (before, after) = stmts
            .iter()
            .position(|s| matches!(s.kind, StmtKind::Placeholder))
            .map_or((stmts, &[][..]), |idx| (&stmts[..idx], &stmts[idx + 1..]));

        // Generate a fix snippet if the modifier logic should be wrapped.
        if let Some(snippet) = self.get_snippet(ctx, func, before, after) {
            ctx.emit_with_fix(&UNWRAPPED_MODIFIER_LOGIC, name.span, snippet);
        }
    }
}

impl UnwrappedModifierLogic {
    // TODO: Support library member calls like `Lib.foo` (throws false positives).
    fn is_valid_expr(&self, expr: &solar_ast::Expr<'_>) -> bool {
        if let ExprKind::Call(func_expr, _) = &expr.kind
            && let ExprKind::Ident(ident) = &func_expr.kind
        {
            return !matches!(ident.name.as_str(), "require" | "assert");
        }
        false
    }

    fn is_valid_stmt(&self, stmt: &Stmt<'_>) -> bool {
        match &stmt.kind {
            StmtKind::Expr(expr) => self.is_valid_expr(expr),
            StmtKind::Placeholder => true,
            _ => false,
        }
    }

    fn check_stmts(&self, stmts: &[Stmt<'_>]) -> bool {
        let mut total_valid = 0;
        for stmt in stmts {
            if !self.is_valid_stmt(stmt) {
                return true;
            }
            if let StmtKind::Expr(expr) = &stmt.kind
                && self.is_valid_expr(expr)
            {
                total_valid += 1;
            }
        }
        total_valid > 1
    }

    fn get_snippet<'a>(
        &self,
        ctx: &LintContext<'_>,
        func: &ItemFunction<'_>,
        before: &'a [Stmt<'a>],
        after: &'a [Stmt<'a>],
    ) -> Option<Snippet> {
        let wrap_before = !before.is_empty() && self.check_stmts(before);
        let wrap_after = !after.is_empty() && self.check_stmts(after);

        if !(wrap_before || wrap_after) {
            return None;
        }

        let header_name = func.header.name.unwrap();
        let modifier_name = header_name.name.as_str();
        let params = &func.header.parameters;

        let param_list = params
            .vars
            .iter()
            .filter_map(|v| v.name.as_ref().map(|n| n.name.to_string()))
            .collect::<Vec<_>>()
            .join(", ");

        let param_decls = params
            .vars
            .iter()
            .map(|v| {
                let name = v.name.as_ref().map(|n| n.name.as_str()).unwrap_or("");
                let ty = ctx
                    .span_to_snippet(v.ty.span)
                    .unwrap_or_else(|| "/* unknown type */".to_string());
                format!("{ty} {name}")
            })
            .collect::<Vec<_>>()
            .join(", ");

        let body_indent = " ".repeat(ctx.get_ind_for_span(
            before.first().or(after.first()).map(|stmt| stmt.span).unwrap_or(func.header.span),
        ));
        let body = match (wrap_before, wrap_after) {
            (true, true) => format!(
                "{body_indent}_{modifier_name}Before({param_list});\n{body_indent}_;\n{body_indent}_{modifier_name}After({param_list});"
            ),
            (true, false) => {
                format!("{body_indent}_{modifier_name}({param_list});\n{body_indent}_;")
            }
            (false, true) => {
                format!("{body_indent}_;\n{body_indent}_{modifier_name}({param_list});")
            }
            _ => unreachable!(),
        };

        let mod_indent = " ".repeat(ctx.get_ind_for_span(func.header.span));
        let mut replacement = format!(
            "{mod_indent}modifier {modifier_name}({param_decls}) {{\n{body}\n{mod_indent}}}"
        );

        let build_func = |stmts: &[Stmt<'_>], suffix: &str| {
            let body_stmts = stmts
                .iter()
                .filter_map(|s| ctx.span_to_snippet(s.span))
                .map(|code| format!("\n{body_indent}{code}"))
                .collect::<String>();
            format!(
                "\n\n{mod_indent}function _{modifier_name}{suffix}({param_decls}) internal {{{body_stmts}\n{mod_indent}}}"
            )
        };

        if wrap_before {
            replacement.push_str(&build_func(before, if wrap_after { "Before" } else { "" }));
        }
        if wrap_after {
            replacement.push_str(&build_func(after, if wrap_before { "After" } else { "" }));
        }

        Some(Snippet::Diff {
            desc: Some("wrap modifier logic to reduce code size"),
            span: Some(Span::new(func.header.span.lo(), func.body_span.hi())),
            add: replacement,
            trim_code: true,
        })
    }
}
