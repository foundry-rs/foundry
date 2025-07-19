use super::UnwrappedModifierLogic;
use crate::{
    linter::{EarlyLintPass, LintContext, Snippet},
    sol::{Severity, SolLint},
};
use solar_ast::{ExprKind, ItemFunction, Stmt, StmtKind};
use solar_interface::BytePos;

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
        if let Some(snippet) = self.get_snippet(ctx, func, stmts, before, after) {
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
        func: &ItemFunction<'_>,
        full_body: &[Stmt<'_>],
    ) -> (String, String, solar_ast::Span) {
        let (first, _) = match (full_body.first(), full_body.last()) {
            (Some(f), Some(l)) => (f, l),
            _ => {
                let name_span = func.header.name.unwrap().span;
                return ("        ".to_string(), "    ".to_string(), name_span);
            }
        };

        let source_map = ctx.session().source_map();
        let loc_info = source_map.lookup_char_pos(first.span.lo());
        let line_start = first.span.lo() - BytePos(loc_info.col.to_usize() as u32);

        let body_indent = source_map
            .span_to_snippet(solar_ast::Span::new(line_start, first.span.lo()))
            .unwrap_or_else(|_| "        ".to_string());

        // Get modifier indent
        let name_span = func.header.name.unwrap().span;
        let pos = source_map.lookup_char_pos(name_span.lo());
        let mod_line_start = name_span.lo() - BytePos(pos.col.to_usize() as u32);

        let mod_indent = source_map
            .span_to_snippet(solar_ast::Span::new(mod_line_start, name_span.lo()))
            .ok()
            .and_then(|s| s.rfind("modifier").map(|p| mod_line_start + BytePos(p as u32)))
            .and_then(|start| {
                source_map.span_to_snippet(solar_ast::Span::new(mod_line_start, start)).ok()
            })
            .unwrap_or_else(|| "    ".to_string());

        // Get full function span
        let start = name_span.lo()
            - BytePos(source_map.lookup_char_pos(name_span.lo()).col.to_usize() as u32);
        let span = func
            .body
            .as_ref()
            .map(|b| solar_ast::Span::new(start, b.span.hi()))
            .unwrap_or(name_span);

        (body_indent, mod_indent, span)
    }

    fn get_snippet<'a>(
        &self,
        ctx: &LintContext<'_>,
        func: &ItemFunction<'_>,
        full_body: &'a [Stmt<'a>],
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

        let (body_indent, mod_indent, span) = self.get_indent_and_span(ctx, func, full_body);

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
            span: Some(span),
            add: replacement,
        })
    }
}
