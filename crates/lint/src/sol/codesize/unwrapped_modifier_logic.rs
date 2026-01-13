use super::UnwrappedModifierLogic;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    data_structures::{Never, map::FxHashSet},
    sema::hir::{self, Res, Visit},
};
use std::ops::ControlFlow;

declare_forge_lint!(
    UNWRAPPED_MODIFIER_LOGIC,
    Severity::CodeSize,
    "unwrapped-modifier-logic",
    "wrap modifier logic to reduce code size"
);

impl<'hir> LateLintPass<'hir> for UnwrappedModifierLogic {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        // Only check modifiers with a body and a name
        let body = match (func.kind, &func.body, func.name) {
            (ast::FunctionKind::Modifier, Some(body), Some(_)) => body,
            _ => return,
        };

        // Split statements into before and after the placeholder `_`.
        let stmts = body.stmts[..].as_ref();
        let (before, after) = stmts
            .iter()
            .position(|s| matches!(s.kind, hir::StmtKind::Placeholder))
            .map_or((stmts, &[][..]), |idx| (&stmts[..idx], &stmts[idx + 1..]));

        // Generate a fix suggestion if the modifier logic should be wrapped.
        if let Some(suggestion) = self.get_snippet(ctx, hir, func, before, after) {
            ctx.emit_with_suggestion(
                &UNWRAPPED_MODIFIER_LOGIC,
                func.span.to(func.body_span),
                suggestion,
            );
        }
    }
}

/// Visitor that collects used variable IDs from expressions.
struct UsedVarCollector<'hir> {
    hir: &'hir hir::Hir<'hir>,
    vars: FxHashSet<hir::VariableId>,
}

impl<'hir> hir::Visit<'hir> for UsedVarCollector<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let hir::ExprKind::Ident(resolutions) = &expr.kind {
            for res in *resolutions {
                if let Res::Item(hir::ItemId::Variable(var_id)) = res {
                    self.vars.insert(*var_id);
                }
            }
        }
        self.walk_expr(expr)
    }
}

impl UnwrappedModifierLogic {
    /// Checks if statements require wrapping into a helper function.
    /// Returns `false` if assembly is detected (HIR represents it as `Err`).
    fn requires_wrapping(
        &self,
        hir: &hir::Hir<'_>,
        stmts: &[hir::Stmt<'_>],
        allow_one_decl: bool,
    ) -> bool {
        let (mut has_trivial_call, mut has_decl) = (false, false);
        for stmt in stmts {
            match &stmt.kind {
                hir::StmtKind::Placeholder => {}
                hir::StmtKind::Expr(expr) => {
                    if !self.is_trivial_call(hir, expr) || has_trivial_call || has_decl {
                        return true;
                    }
                    has_trivial_call = true;
                }
                // HIR doesn't support assembly yet:
                // <https://github.com/paradigmxyz/solar/blob/d25bf38a5accd11409318e023f701313d98b9e1e/crates/sema/src/hir/mod.rs#L977-L982>
                hir::StmtKind::Err(_) => return false,
                hir::StmtKind::DeclSingle(_) | hir::StmtKind::DeclMulti(_, _) if allow_one_decl => {
                    if has_trivial_call || has_decl {
                        return true;
                    }
                    has_decl = true;
                }
                _ => return true,
            }
        }
        false
    }

    /// Collects top-level declared variable IDs from statements.
    fn collect_declared_vars(hir: &hir::Hir<'_>, stmts: &[hir::Stmt<'_>]) -> Vec<hir::VariableId> {
        let is_stmt_var =
            |id: &hir::VariableId| matches!(hir.variable(*id).kind, hir::VarKind::Statement);
        let mut vars = Vec::new();
        for stmt in stmts {
            match &stmt.kind {
                hir::StmtKind::DeclSingle(id) if is_stmt_var(id) => vars.push(*id),
                hir::StmtKind::DeclMulti(ids, _) => {
                    vars.extend(ids.iter().flatten().filter(|id| is_stmt_var(id)).copied())
                }
                _ => {}
            }
        }
        vars
    }

    /// Collects all variables referenced in a statement block.
    fn collect_used_vars(
        hir: &hir::Hir<'_>,
        stmts: &[hir::Stmt<'_>],
    ) -> FxHashSet<hir::VariableId> {
        let mut collector = UsedVarCollector { hir, vars: FxHashSet::default() };
        for stmt in stmts {
            let _ = collector.visit_stmt(stmt);
        }
        collector.vars
    }

    /// Finds variables declared in "before" that are used in "after".
    fn collect_shared_locals(
        hir: &hir::Hir<'_>,
        before: &[hir::Stmt<'_>],
        after: &[hir::Stmt<'_>],
    ) -> Vec<hir::VariableId> {
        if after.is_empty() || before.is_empty() {
            return Vec::new();
        }
        let declared_before = Self::collect_declared_vars(hir, before);
        if declared_before.is_empty() {
            return Vec::new();
        }
        let used_after = Self::collect_used_vars(hir, after);
        declared_before.into_iter().filter(|id| used_after.contains(id)).collect()
    }

    /// Returns `true` if the expression is a "trivial" call that doesn't require wrapping.
    fn is_trivial_call(&self, hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
        let hir::ExprKind::Call(func_expr, _, _) = &expr.kind else {
            return false;
        };

        match &func_expr.kind {
            // Direct function call: trivial if not a builtin
            hir::ExprKind::Ident(resolutions) => {
                !resolutions.iter().any(|r| matches!(r, Res::Builtin(_)))
            }
            // Member call: trivial if calling a library function
            hir::ExprKind::Member(base, _) => {
                if let hir::ExprKind::Ident(resolutions) = &base.kind {
                    resolutions.iter().any(|r| {
                        matches!(r, Res::Item(hir::ItemId::Contract(id))
                            if hir.contract(*id).kind == ast::ContractKind::Library)
                    })
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Extracts (type, name, decl) strings for a variable.
    fn extract_var_info(
        ctx: &LintContext,
        hir: &hir::Hir<'_>,
        var_id: hir::VariableId,
    ) -> Option<(String, String, String)> {
        let var = hir.variable(var_id);
        let ty = ctx.span_to_snippet(var.ty.span)?;
        let name = var.name?.to_string();
        Some((ty.clone(), name.clone(), format!("{ty} {name}")))
    }

    fn get_snippet<'hir>(
        &self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        func: &hir::Function<'_>,
        before: &'hir [hir::Stmt<'hir>],
        after: &'hir [hir::Stmt<'hir>],
    ) -> Option<Suggestion> {
        let wrap_before = !before.is_empty() && self.requires_wrapping(hir, before, true);
        let wrap_after = !after.is_empty() && self.requires_wrapping(hir, after, false);

        if !(wrap_before || wrap_after) {
            return None;
        }

        let binding = func.name.unwrap();
        let modifier_name = binding.name.as_str();
        let mut param_names = vec![];
        let mut param_decls = vec![];
        for var_id in func.parameters {
            if let Some((_, name, decl)) = Self::extract_var_info(ctx, hir, *var_id) {
                param_names.push(name);
                param_decls.push(decl);
            }
        }

        // Extract type and name info for shared locals
        let shared_locals = Self::collect_shared_locals(hir, before, after);
        let (mut shared_types, mut shared_names, mut shared_decls) = (vec![], vec![], vec![]);
        for var_id in &shared_locals {
            if let Some((ty, name, decl)) = Self::extract_var_info(ctx, hir, *var_id) {
                shared_types.push(ty);
                shared_names.push(name);
                shared_decls.push(decl);
            }
        }

        let body_indent = " ".repeat(ctx.get_span_indentation(
            before.first().or(after.first()).map(|stmt| stmt.span).unwrap_or(func.span),
        ));

        // Build format strings for different shared variable counts
        let (assignment, returns_decl, return_stmt) = match shared_locals.len() {
            0 => (String::new(), String::new(), String::new()),
            1 => (
                format!("{} {} = ", shared_types[0], shared_names[0]),
                format!(" returns ({})", shared_types[0]),
                format!("\n{body_indent}return {};", shared_names[0]),
            ),
            _ => (
                format!("({}) = ", shared_decls.join(", ")),
                format!(" returns ({})", shared_types.join(", ")),
                format!("\n{body_indent}return ({});", shared_names.join(", ")),
            ),
        };

        let param_names = param_names.join(", ");
        let param_decls = param_decls.join(", ");

        let after_args = if shared_locals.is_empty() {
            param_names.clone()
        } else if param_names.is_empty() {
            shared_names.join(", ")
        } else {
            format!("{}, {}", param_names, shared_names.join(", "))
        };

        let body = match (wrap_before, wrap_after) {
            (true, true) => format!(
                "{body_indent}{assignment}_{modifier_name}Before({param_names});\n{body_indent}_;\n{body_indent}_{modifier_name}After({after_args});"
            ),
            (true, false) => {
                // Before is wrapped, after isn't complex enough to wrap - keep after inline
                let after_stmts = after
                    .iter()
                    .filter_map(|s| ctx.span_to_snippet(s.span))
                    .map(|code| format!("\n{body_indent}{code}"))
                    .collect::<String>();
                format!(
                    "{body_indent}{assignment}_{modifier_name}({param_names});\n{body_indent}_;{after_stmts}"
                )
            }
            (false, true) => {
                // Before isn't wrapped, so include its statements inline
                let before_stmts = before
                    .iter()
                    .filter_map(|s| ctx.span_to_snippet(s.span))
                    .map(|code| format!("{body_indent}{code}\n"))
                    .collect::<String>();
                format!(
                    "{before_stmts}{body_indent}_;\n{body_indent}_{modifier_name}({after_args});"
                )
            }
            _ => unreachable!(),
        };

        let mod_indent = " ".repeat(ctx.get_span_indentation(func.span));
        let mut replacement =
            format!("modifier {modifier_name}({param_decls}) {{\n{body}\n{mod_indent}}}");

        let build_func = |stmts: &[hir::Stmt<'_>], suffix: &str, is_before: bool| {
            let body_stmts = stmts
                .iter()
                .filter_map(|s| ctx.span_to_snippet(s.span))
                .map(|code| format!("\n{body_indent}{code}"))
                .collect::<String>();

            let extra_params = if !is_before && !shared_decls.is_empty() {
                if param_decls.is_empty() {
                    shared_decls.join(", ")
                } else {
                    format!("{}, {}", param_decls, shared_decls.join(", "))
                }
            } else {
                param_decls.clone()
            };

            let returns = if is_before && !returns_decl.is_empty() { &returns_decl } else { "" };
            let ret_stmt = if is_before && !return_stmt.is_empty() { &return_stmt } else { "" };
            format!(
                "\n\n{mod_indent}function _{modifier_name}{suffix}({extra_params}) internal{returns} {{{body_stmts}{ret_stmt}\n{mod_indent}}}"
            )
        };

        if wrap_before {
            replacement.push_str(&build_func(before, if wrap_after { "Before" } else { "" }, true));
        }
        if wrap_after {
            replacement.push_str(&build_func(after, if wrap_before { "After" } else { "" }, false));
        }

        Some(
            Suggestion::fix(
                replacement,
                ast::interface::diagnostics::Applicability::MachineApplicable,
            )
            .with_desc("wrap modifier logic to reduce code size"),
        )
    }
}
