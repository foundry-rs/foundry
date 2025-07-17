use super::UnwrappedModifierLogic;
use crate::{
    linter::{EarlyLintPass, LintContext, Snippet},
    sol::{Severity, SolLint},
};
use solar_ast::{ExprKind, ItemFunction, Stmt, StmtKind};

declare_forge_lint!(
    UNWRAPPED_MODIFIER_LOGIC,
    Severity::Gas,
    "unwrapped-modifier-logic",
    "wrap modifier logic to reduce code size"
);

impl<'ast> EarlyLintPass<'ast> for UnwrappedModifierLogic {
    fn check_item_function(&mut self, ctx: &LintContext<'_>, func: &'ast ItemFunction<'ast>) {
        // Only check modifiers with a body and name.
        if !func.kind.is_modifier() || func.body.is_none() || func.header.name.is_none() {
            return;
        }

        let name = func.header.name.unwrap();
        let stmts = &func.body.as_ref().unwrap().stmts[..];

        // Split statements into before and after the placeholder `_`.
        let (before, after) = stmts
            .iter()
            .position(|s| matches!(s.kind, StmtKind::Placeholder))
            .map_or((stmts, &[][..]), |idx| (&stmts[..idx], &stmts[idx + 1..]));

        // Generate a fix snippet if the modifier logic should be wrapped.
        if let Some(snippet) =
            self.get_snippet(ctx, &name, &func.header.parameters, stmts, before, after)
        {
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

    fn get_indent_and_span(
        &self,
        ctx: &LintContext<'_>,
        full_body: &[Stmt<'_>],
    ) -> (String, Option<solar_ast::Span>) {
        let (first, last) = match (full_body.first(), full_body.last()) {
            (Some(f), Some(l)) => (f, l),
            _ => return ("        ".to_string(), None),
        };

        let source_map = ctx.session().source_map();
        let loc_info = source_map.lookup_char_pos(first.span.lo());
        let line_start = first.span.lo() - solar_interface::BytePos(loc_info.col.to_usize() as u32);

        match source_map.span_to_snippet(solar_ast::Span::new(line_start, first.span.lo())) {
            Ok(indent) => (indent, Some(solar_ast::Span::new(line_start, last.span.hi()))),
            Err(_) => ("        ".to_string(), None),
        }
    }

    fn get_snippet<'a>(
        &self,
        ctx: &LintContext<'_>,
        name: &solar_ast::Ident,
        params: &solar_ast::ParameterList<'_>,
        full_body: &'a [Stmt<'a>],
        before: &'a [Stmt<'a>],
        after: &'a [Stmt<'a>],
    ) -> Option<Snippet> {
        // Check if before/after blocks should be wrapped.
        let wrap_before = !before.is_empty() && self.check_stmts(before);
        let wrap_after = !after.is_empty() && self.check_stmts(after);

        if !wrap_before && !wrap_after {
            return None;
        }

        // Get modifier name.
        let modifier_name = name.name.as_str();

        // Get modifier parameters.
        let param_list = params
            .vars
            .iter()
            .filter_map(|v| v.name.as_ref())
            .map(|n| n.name.to_string())
            .collect::<Vec<_>>()
            .join(", ");

        // Get indentation and span.
        let (indent, span) = self.get_indent_and_span(ctx, full_body);

        // Generate replacement code.
        let fix = match (wrap_before, wrap_after) {
            (true, true) => format!(
                "{indent}_{modifier_name}Before({param_list});\n{indent}_;\n{indent}_{modifier_name}After({param_list});"
            ),
            (true, false) => format!("{indent}_{modifier_name}({param_list});\n{indent}_;"),
            (false, true) => format!("{indent}_;\n{indent}_{modifier_name}({param_list});"),
            (false, false) => unreachable!(),
        };

        Some(Snippet::Diff {
            desc: Some("wrap modifier logic to reduce code size"),
            span,
            add: fix,
        })
    }
}
