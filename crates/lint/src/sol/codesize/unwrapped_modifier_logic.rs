use super::UnwrappedModifierLogic;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint, low::incorrect_modifier},
};
use solar::{
    ast,
    sema::hir::{self, Res, Visit as _},
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
        _gcx: solar::sema::Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        // Only check modifiers with a body and a name
        let body = match (func.kind, &func.body, func.name) {
            (ast::FunctionKind::Modifier, Some(body), Some(_)) => body,
            _ => return,
        };

        if incorrect_modifier::block_outcome(*body).can_skip_placeholder() {
            return;
        }

        // Only handle modifiers with exactly one placeholder, *and* require it to be top-level.
        // Counting recursively (rather than just top-level statements) ensures a placeholder nested
        // inside an `if`/loop/etc. is never extracted into a helper function, which would produce
        // an invalid, behavior-changing rewrite.
        if count_placeholders(body.stmts) != 1 {
            return;
        }
        let Some(idx) =
            body.stmts.iter().position(|s| matches!(s.kind, hir::StmtKind::Placeholder))
        else {
            // The single placeholder is nested; splitting it out would be unsafe.
            return;
        };

        // Split statements into before and after the placeholder `_`.
        let stmts = body.stmts[..].as_ref();
        let (before, after) = (&stmts[..idx], &stmts[idx + 1..]);

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
                hir::StmtKind::Placeholder => {}
                hir::StmtKind::Expr(expr) => {
                    if !self.is_valid_expr(hir, expr) || has_valid_stmt {
                        res = true;
                    }
                    has_valid_stmt = true;
                }
                // Assembly may contain control flow or side effects this lint does not model.
                hir::StmtKind::AssemblyBlock(_)
                | hir::StmtKind::Switch(_)
                | hir::StmtKind::Err(_) => return false,
                _ => res = true,
            }
        }

        res
    }

    fn get_snippet<'hir>(
        &self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
        before: &'hir [hir::Stmt<'hir>],
        after: &'hir [hir::Stmt<'hir>],
    ) -> Option<Suggestion> {
        let wrap_before = !before.is_empty() && self.stmts_require_wrapping(hir, before);
        let wrap_after = !after.is_empty() && self.stmts_require_wrapping(hir, after);

        if !(wrap_before || wrap_after) {
            return None;
        }

        // A local variable declared before the placeholder and referenced after it makes any
        // rewrite unsafe: extracted helpers only receive the modifier's parameters, so moving
        // either side out of the modifier separates the declaration from its use.
        if has_shared_locals(hir, before, after)
            || (wrap_before && has_written_params_used_after(hir, func.parameters, before, after))
        {
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
        // Statements on a side that doesn't require wrapping are preserved verbatim in the new
        // modifier body, so the rewrite never drops them.
        let mut body_lines = Vec::new();
        if wrap_before {
            let suffix = if wrap_after { "Before" } else { "" };
            body_lines.push(format!("{body_indent}_{modifier_name}{suffix}({param_list});"));
        } else {
            for stmt in before {
                body_lines.push(format!("{body_indent}{}", ctx.span_to_snippet(stmt.span)?));
            }
        }
        body_lines.push(format!("{body_indent}_;"));
        if wrap_after {
            let suffix = if wrap_before { "After" } else { "" };
            body_lines.push(format!("{body_indent}_{modifier_name}{suffix}({param_list});"));
        } else {
            for stmt in after {
                body_lines.push(format!("{body_indent}{}", ctx.span_to_snippet(stmt.span)?));
            }
        }
        let body = body_lines.join("\n");

        let mod_indent = " ".repeat(ctx.get_span_indentation(func.span));
        let mut replacement =
            format!("modifier {modifier_name}({param_decls}) {{\n{body}\n{mod_indent}}}");

        let build_func = |stmts: &[hir::Stmt<'_>], suffix: &str| {
            let body_stmts = stmts
                .iter()
                .map(|s| ctx.span_to_snippet(s.span).map(|code| format!("\n{body_indent}{code}")))
                .collect::<Option<String>>()?;
            Some(format!(
                "\n\n{mod_indent}function _{modifier_name}{suffix}({param_decls}) internal {{{body_stmts}\n{mod_indent}}}"
            ))
        };

        if wrap_before {
            replacement.push_str(&build_func(before, if wrap_after { "Before" } else { "" })?);
        }
        if wrap_after {
            replacement.push_str(&build_func(after, if wrap_before { "After" } else { "" })?);
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

/// Visitor that breaks on the first reference to any of the tracked local variables.
struct SharedLocalFinder<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    locals: &'a [hir::VariableId],
}

impl<'hir> hir::Visit<'hir> for SharedLocalFinder<'_, 'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let hir::ExprKind::Ident(resolutions) = &expr.kind
            && resolutions.iter().any(
                |r| matches!(r, Res::Item(hir::ItemId::Variable(id)) if self.locals.contains(id)),
            )
        {
            return ControlFlow::Break(());
        }
        self.walk_expr(expr)
    }
}

/// Returns `true` if a local variable declared in the `before` segment is referenced in the
/// `after` segment.
///
/// Only top-level declarations need to be tracked: declarations nested inside blocks, loops, or
/// `try` clauses are scoped to them and cannot be referenced after the placeholder.
fn has_shared_locals<'hir>(
    hir: &'hir hir::Hir<'hir>,
    before: &'hir [hir::Stmt<'hir>],
    after: &'hir [hir::Stmt<'hir>],
) -> bool {
    let mut declared = Vec::new();
    for stmt in before {
        match &stmt.kind {
            hir::StmtKind::DeclSingle(id) => declared.push(*id),
            hir::StmtKind::DeclMulti(ids, _) => declared.extend(ids.iter().copied().flatten()),
            _ => {}
        }
    }
    if declared.is_empty() {
        return false;
    }

    let mut finder = SharedLocalFinder { hir, locals: &declared };
    after.iter().any(|stmt| finder.visit_stmt(stmt).is_break())
}

/// Returns `true` if a modifier parameter is written in the `before` segment and referenced in
/// the `after` segment.
fn has_written_params_used_after<'hir>(
    hir: &'hir hir::Hir<'hir>,
    params: &'hir [hir::VariableId],
    before: &'hir [hir::Stmt<'hir>],
    after: &'hir [hir::Stmt<'hir>],
) -> bool {
    let mut written = Vec::new();
    let mut finder = ParamWriteFinder { hir, params, written: &mut written };
    for stmt in before {
        let _ = finder.visit_stmt(stmt);
    }

    if written.is_empty() {
        return false;
    }

    let mut finder = SharedLocalFinder { hir, locals: &written };
    after.iter().any(|stmt| finder.visit_stmt(stmt).is_break())
}

/// Visitor that collects modifier parameters written by an expression.
struct ParamWriteFinder<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    params: &'a [hir::VariableId],
    written: &'a mut Vec<hir::VariableId>,
}

impl<'hir> hir::Visit<'hir> for ParamWriteFinder<'_, 'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            hir::ExprKind::Assign(lhs, _, _) | hir::ExprKind::Delete(lhs) => {
                collect_written_params(lhs, self.params, self.written);
            }
            hir::ExprKind::Unary(op, inner)
                if matches!(
                    op.kind,
                    ast::UnOpKind::PreInc
                        | ast::UnOpKind::PreDec
                        | ast::UnOpKind::PostInc
                        | ast::UnOpKind::PostDec
                ) =>
            {
                collect_written_params(inner, self.params, self.written);
            }
            _ => {}
        }

        self.walk_expr(expr)
    }
}

fn collect_written_params(
    expr: &hir::Expr<'_>,
    params: &[hir::VariableId],
    written: &mut Vec<hir::VariableId>,
) {
    match &expr.kind {
        hir::ExprKind::Ident(resolutions) => {
            for resolution in *resolutions {
                if let Res::Item(hir::ItemId::Variable(id)) = resolution
                    && params.contains(id)
                    && !written.contains(id)
                {
                    written.push(*id);
                }
            }
        }
        hir::ExprKind::Tuple(items) => {
            for item in items.iter().flatten() {
                collect_written_params(item, params, written);
            }
        }
        hir::ExprKind::Index(base, _)
        | hir::ExprKind::Slice(base, _, _)
        | hir::ExprKind::Member(base, _)
        | hir::ExprKind::YulMember(base, _) => collect_written_params(base, params, written),
        _ => {}
    }
}

/// Recursively counts placeholder (`_`) statements within a list of statements, descending into
/// nested blocks, conditionals, loops, `try`/`catch`, and Yul `switch` cases.
fn count_placeholders(stmts: &[hir::Stmt<'_>]) -> usize {
    stmts.iter().map(count_placeholders_in_stmt).sum()
}

fn count_placeholders_in_stmt(stmt: &hir::Stmt<'_>) -> usize {
    match &stmt.kind {
        hir::StmtKind::Placeholder => 1,
        hir::StmtKind::Block(block)
        | hir::StmtKind::UncheckedBlock(block)
        | hir::StmtKind::AssemblyBlock(block)
        | hir::StmtKind::Loop(block, _) => count_placeholders(block.stmts),
        hir::StmtKind::If(_, then_stmt, else_stmt) => {
            count_placeholders_in_stmt(then_stmt)
                + else_stmt.map_or(0, |s| count_placeholders_in_stmt(s))
        }
        hir::StmtKind::Try(try_stmt) => {
            try_stmt.clauses.iter().map(|clause| count_placeholders(clause.block.stmts)).sum()
        }
        hir::StmtKind::Switch(switch) => {
            switch.cases.iter().map(|case| count_placeholders(case.body.stmts)).sum()
        }
        _ => 0,
    }
}
