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
        // Skip non-modifiers and empty modifiers.
        if !func.kind.is_modifier() || func.body.is_none() {
            return;
        }

        // Get the body of the modifier.
        let body = func.body.as_ref().unwrap();

        // Find the placeholder statement position.
        let placeholder_idx = body.iter().position(|s| matches!(s.kind, StmtKind::Placeholder));

        // Split statements before and after placeholder.
        let (before, after) = if let Some(idx) = placeholder_idx {
            (&body[..idx], &body[idx + 1..])
        } else {
            (&body[..], &[][..])
        };

        // Check if statements need wrapping.
        let needs_lint = |stmts: &[Stmt<'_>]| {
            let valid_count = stmts.iter().filter(|s| is_valid_stmt(s)).count();
            let has_invalid = stmts.iter().any(|s| !is_valid_stmt(s));
            has_invalid || valid_count > 1
        };

        // Emit lint if either section needs wrapping.
        if (needs_lint(before) || needs_lint(after))
            && let Some(name) = func.header.name
        {
            self.emit_lint_with_fix(ctx, &name, &func.header.parameters, body, before, after);
        }
    }
}

impl UnwrappedModifierLogic {
    fn emit_lint_with_fix<'a>(
        &self,
        ctx: &LintContext<'_>,
        name: &solar_ast::Ident,
        params: &solar_ast::ParameterList<'_>,
        full_body: &'a [Stmt<'a>],
        before: &'a [Stmt<'a>],
        after: &'a [Stmt<'a>],
    ) {
        if let Some(snippet) =
            self.generate_fix(name.name.as_str(), params, full_body, before, after)
        {
            ctx.emit_with_fix(&UNWRAPPED_MODIFIER_LOGIC, name.span, snippet);
        } else {
            ctx.emit(&UNWRAPPED_MODIFIER_LOGIC, name.span);
        }
    }

    fn generate_fix<'a>(
        &self,
        modifier_name: &str,
        params: &solar_ast::ParameterList<'_>,
        full_body: &'a [Stmt<'a>],
        before: &'a [Stmt<'a>],
        after: &'a [Stmt<'a>],
    ) -> Option<Snippet> {
        // Build parameter list for function calls.
        let param_list = format!(
            "({})",
            params
                .vars
                .iter()
                .filter_map(|v| v.name.as_ref())
                .map(|n| n.name.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );

        // Check which sections need wrapping.
        let wrap_before = !before.is_empty() && needs_wrapping(before);
        let wrap_after = !after.is_empty() && needs_wrapping(after);

        // Generate replacement based on what needs wrapping.
        let replacement = match (wrap_before, wrap_after) {
            // If both sections need wrapping, wrap both appending `Before` and `After` to the
            // modifier name.
            (true, true) => format!(
                "_{}Before{};\n_;\n_{}After{};",
                modifier_name, param_list, modifier_name, param_list
            ),

            // If only one before section needs wrapping, wrap that section.
            (true, false) => {
                format!("_{}{};\n_;", modifier_name, param_list)
            }

            // If only one after section needs wrapping, wrap that section.
            (false, true) => format!("_;\n_{}{};", modifier_name, param_list),

            // If no sections need wrapping, return None.
            (false, false) => return None,
        };

        // Return the replacement snippet.
        Some(Snippet::Diff {
            desc: Some("wrap modifier logic to reduce code size"),
            span: Some(full_body.first()?.span.to(full_body.last()?.span)),
            add: replacement,
        })
    }
}

fn needs_wrapping(stmts: &[Stmt<'_>]) -> bool {
    let valid_count = stmts.iter().filter(|s| is_valid_stmt(s)).count();
    let has_invalid = stmts.iter().any(|s| !is_valid_stmt(s));
    has_invalid || valid_count > 1
}

fn is_valid_stmt(stmt: &Stmt<'_>) -> bool {
    match &stmt.kind {
        // If the statement is an expression, emit if not valid.
        StmtKind::Expr(expr) => is_valid_expr(expr),

        // If the statement is a placeholder, skip.
        StmtKind::Placeholder => true,

        // Disallow all other statements.
        _ => false,
    }
}

// TODO: Support library member calls like `Lib.foo` (throws false positives).
fn is_valid_expr(expr: &solar_ast::Expr<'_>) -> bool {
    // If the expression is a call, continue.
    if let ExprKind::Call(func_expr, _) = &expr.kind
        && let ExprKind::Ident(ident) = &func_expr.kind
    {
        // If the call is a built-in control flow function, emit.
        return !matches!(ident.name.as_str(), "require" | "assert");
    }

    // Disallow all other expressions.
    false
}
