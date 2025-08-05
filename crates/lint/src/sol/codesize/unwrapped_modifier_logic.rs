use super::UnwrappedModifierLogic;
use crate::{
    linter::{LateLintPass, LintContext, Snippet},
    sol::{Severity, SolLint},
};
use solar_ast::{self as ast, Span};
use solar_sema::hir::{self, Res};

declare_forge_lint!(
    UNWRAPPED_MODIFIER_LOGIC,
    Severity::Gas,
    "unwrapped-modifier-logic",
    "wrap modifier logic to reduce code size"
);

impl<'hir> LateLintPass<'hir> for UnwrappedModifierLogic {
    fn check_function(
        &mut self,
        ctx: &LintContext<'_>,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        // Only check modifiers with a body and a name
        let (body, name) = match (func.kind, &func.body, func.name) {
            (ast::FunctionKind::Modifier, Some(body), Some(name)) => (body, name),
            _ => return,
        };

        // Split statements into before and after the placeholder `_`.
        let stmts = body.stmts[..].as_ref();
        let (before, after) = stmts
            .iter()
            .position(|s| matches!(s.kind, hir::StmtKind::Placeholder))
            .map_or((stmts, &[][..]), |idx| (&stmts[..idx], &stmts[idx + 1..]));

        // Generate a fix snippet if the modifier logic should be wrapped.
        if let Some(snippet) = self.get_snippet(ctx, hir, func, before, after) {
            ctx.emit_with_fix(&UNWRAPPED_MODIFIER_LOGIC, name.span, snippet);
        }
    }
}

impl UnwrappedModifierLogic {
    /// Returns `true` if an expr is not a built-in ('require' or 'assert') call or a lib function.
    fn is_valid_expr(&self, hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
        if let hir::ExprKind::Call(func_expr, _, _) = &expr.kind {
            if let hir::ExprKind::Ident(resolutions) = &func_expr.kind {
                return !resolutions.iter().any(|r| matches!(r, Res::Builtin(_)));
            }

            if let hir::ExprKind::Member(base, _) = &func_expr.kind
                && let hir::ExprKind::Ident(resolutions) = &base.kind
            {
                return resolutions.iter().any(|r| {
                    matches!(r, Res::Item(hir::ItemId::Contract(id)) if hir.contract(*id).kind == ast::ContractKind::Library)
                });
            }
        }

        false
    }

    /// Checks if a block of statements is complex and should be wrapped in a helper function.
    ///
    /// This always is 'false' the modifier contains assembly. We assume that if devs know how to
    /// use assembly, they will also know how to reduce the codesize of their contracts and they
    /// have a good reason to use it on their modifiers.
    ///
    /// This is 'true' if the block contains:
    /// 1. Any statement that is not a placeholder or a valid expression.
    /// 2. More than one simple call expression.
    fn stmts_require_wrapping(&self, hir: &hir::Hir<'_>, stmts: &[hir::Stmt<'_>]) -> bool {
        let (mut res, mut has_valid_stmt) = (false, false);
        for stmt in stmts {
            match &stmt.kind {
                hir::StmtKind::Placeholder => continue,
                hir::StmtKind::Expr(expr) => {
                    if !self.is_valid_expr(hir, expr) || has_valid_stmt {
                        res = true;
                    }
                    has_valid_stmt = true;
                }
                // HIR doesn't support assembly yet:
                // <https://github.com/paradigmxyz/solar/blob/d25bf38a5accd11409318e023f701313d98b9e1e/crates/sema/src/hir/mod.rs#L977-L982>
                hir::StmtKind::Err(_) => return false,
                _ => res = true,
            }
        }

        res
    }

    fn get_snippet<'a>(
        &self,
        ctx: &LintContext<'_>,
        hir: &hir::Hir<'_>,
        func: &hir::Function<'_>,
        before: &'a [hir::Stmt<'a>],
        after: &'a [hir::Stmt<'a>],
    ) -> Option<Snippet> {
        let wrap_before = !before.is_empty() && self.stmts_require_wrapping(hir, before);
        let wrap_after = !after.is_empty() && self.stmts_require_wrapping(hir, after);

        if !(wrap_before || wrap_after) {
            return None;
        }

        let binding = func.name.unwrap();
        let modifier_name = binding.name.as_str();
        let mut param_list = vec![];
        let mut param_decls = vec![];

        for var_id in func.parameters {
            let var = hir.variable(*var_id);
            let ty = ctx
                .span_to_snippet(var.ty.span)
                .unwrap_or_else(|| "/* unknown type */".to_string());

            // solidity functions should always have named parameters
            if let Some(ident) = var.name {
                param_list.push(ident.to_string());
                param_decls.push(format!("{ty} {}", ident.to_string()));
            }
        }

        let param_list = param_list.join(", ");
        let param_decls = param_decls.join(", ");

        let body_indent = " ".repeat(ctx.get_span_indentation(
            before.first().or(after.first()).map(|stmt| stmt.span).unwrap_or(func.span),
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

        let mod_indent = " ".repeat(ctx.get_span_indentation(func.span));
        let mut replacement = format!(
            "{mod_indent}modifier {modifier_name}({param_decls}) {{\n{body}\n{mod_indent}}}"
        );

        let build_func = |stmts: &[hir::Stmt<'_>], suffix: &str| {
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
            span: Some(Span::new(func.span.lo(), func.body_span.hi())),
            add: replacement,
            trim_code: true,
        })
    }
}
