use super::UnwrappedModifierLogic;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{
        Severity, SolLint,
        analysis::{block_outcome, count_placeholders, for_each_lhs_var, referenced_item},
    },
};
use solar::{
    ast::{ContractKind, FunctionKind},
    interface::diagnostics::Applicability,
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind, Function, ItemId, Res, Stmt, StmtKind, Visit as _},
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(
    UNWRAPPED_MODIFIER_LOGIC,
    Severity::CodeSize,
    "unwrapped-modifier-logic",
    "wrap modifier logic to reduce code size"
);

impl<'gcx> LateLintPass<'gcx> for UnwrappedModifierLogic {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        let (FunctionKind::Modifier, Some(body), Some(name)) = (func.kind, func.body, func.name)
        else {
            return;
        };
        if block_outcome(body).can_skip_placeholder() {
            return;
        }
        // Only a single, top-level placeholder can be split around: extracting a placeholder
        // nested in an `if`/loop/`try` into a helper would change behavior.
        if count_placeholders(body.stmts) != 1 {
            return;
        }
        let Some(idx) = body.stmts.iter().position(|s| matches!(s.kind, StmtKind::Placeholder))
        else {
            return;
        };
        let (before, after) = (&body.stmts[..idx], &body.stmts[idx + 1..]);
        if let Some(suggestion) = snippet(ctx, &gcx.hir, func, name.as_str(), before, after) {
            ctx.emit_with_suggestion(
                &UNWRAPPED_MODIFIER_LOGIC,
                func.span.to(func.body_span),
                suggestion,
            );
        }
    }
}

/// A call to a non-builtin function or to a library function: the only statement cheap enough to
/// leave inline.
fn is_plain_call(hir: &hir::Hir<'_>, expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, ..) = &expr.kind else { return false };
    match &callee.kind {
        ExprKind::Ident(reses) => !reses.iter().any(|r| r.as_builtin().is_some()),
        ExprKind::Member(base, _) => matches!(referenced_item(base), Some(ItemId::Contract(id))
            if hir.contract(id).kind == ContractKind::Library),
        _ => false,
    }
}

/// Whether `stmts` should move into a helper: anything but a single plain call requires wrapping.
/// Inline assembly is left alone; its authors know how to manage code size and have a reason to
/// use it in a modifier.
fn requires_wrapping(hir: &hir::Hir<'_>, stmts: &[Stmt<'_>]) -> bool {
    let (mut calls, mut other) = (0, false);
    for stmt in stmts {
        match &stmt.kind {
            StmtKind::Placeholder => {}
            StmtKind::Expr(expr) if is_plain_call(hir, expr) => calls += 1,
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => return false,
            _ => other = true,
        }
    }
    other || calls > 1
}

fn snippet<'gcx>(
    ctx: &LintContext,
    hir: &'gcx hir::Hir<'gcx>,
    func: &'gcx Function<'gcx>,
    name: &str,
    before: &'gcx [Stmt<'gcx>],
    after: &'gcx [Stmt<'gcx>],
) -> Option<Suggestion> {
    let (wrap_before, wrap_after) = (requires_wrapping(hir, before), requires_wrapping(hir, after));
    if !(wrap_before || wrap_after) {
        return None;
    }

    // Extracted helpers only receive the modifier's parameters, so a local declared before the
    // placeholder and used after it, or a parameter mutated before it and read after it (the
    // helper would only mutate its by-value copy), makes the rewrite unsafe. Only top-level
    // declarations matter: nested ones are scoped to their block.
    let mut shared = Vec::new();
    for stmt in before {
        match &stmt.kind {
            StmtKind::DeclSingle(id) => shared.push(*id),
            StmtKind::DeclMulti(ids, _) => shared.extend(ids.iter().flatten()),
            _ => {}
        }
    }
    if wrap_before {
        any_expr(hir, before, |expr| {
            let lvalue = match &expr.kind {
                ExprKind::Assign(lhs, ..) | ExprKind::Delete(lhs) => Some(lhs),
                ExprKind::Unary(op, inner) if op.kind.has_side_effects() => Some(inner),
                _ => None,
            };
            if let Some(lvalue) = lvalue {
                for_each_lhs_var(lvalue, &mut |v| {
                    if func.parameters.contains(&v) && !shared.contains(&v) {
                        shared.push(v);
                    }
                });
            }
            false
        });
    }
    if any_expr(hir, after, |expr| {
        matches!(&expr.kind, ExprKind::Ident(reses)
            if reses.iter().filter_map(Res::as_variable).any(|v| shared.contains(&v)))
    }) {
        return None;
    }

    let (mut param_list, mut param_decls) = (Vec::new(), Vec::new());
    for &var_id in func.parameters {
        let var = hir.variable(var_id);
        // Unnamed parameters cannot be forwarded to the helper.
        let Some(ident) = var.name else { continue };
        let ty = ctx.span_to_snippet(var.ty.span).unwrap_or_else(|| "/* unknown type */".into());
        param_list.push(ident.to_string());
        param_decls.push(format!("{ty} {ident}"));
    }
    let (param_list, param_decls) = (param_list.join(", "), param_decls.join(", "));
    let body_indent = " ".repeat(
        ctx.get_span_indentation(before.first().or(after.first()).map_or(func.span, |s| s.span)),
    );
    let mod_indent = " ".repeat(ctx.get_span_indentation(func.span));
    let (before_suffix, after_suffix) =
        if wrap_before && wrap_after { ("Before", "After") } else { ("", "") };

    // A side that needs wrapping becomes a helper call plus the helper definition; the other side
    // is preserved verbatim so the rewrite never drops statements.
    let side = |stmts: &[Stmt<'_>], wrap: bool, suffix: &str| -> Option<(Vec<String>, String)> {
        if !wrap {
            let lines = stmts
                .iter()
                .map(|s| Some(format!("{body_indent}{}", ctx.span_to_snippet(s.span)?)))
                .collect::<Option<_>>()?;
            return Some((lines, String::new()));
        }
        let body = stmts
            .iter()
            .map(|s| Some(format!("\n{body_indent}{}", ctx.span_to_snippet(s.span)?)))
            .collect::<Option<String>>()?;
        Some((
            vec![format!("{body_indent}_{name}{suffix}({param_list});")],
            format!(
                "\n\n{mod_indent}function _{name}{suffix}({param_decls}) internal {{{body}\n{mod_indent}}}"
            ),
        ))
    };
    let (before_lines, before_helper) = side(before, wrap_before, before_suffix)?;
    let (after_lines, after_helper) = side(after, wrap_after, after_suffix)?;
    let body = before_lines
        .into_iter()
        .chain([format!("{body_indent}_;")])
        .chain(after_lines)
        .collect::<Vec<_>>()
        .join("\n");
    let replacement = format!(
        "modifier {name}({param_decls}) {{\n{body}\n{mod_indent}}}{before_helper}{after_helper}"
    );
    Some(
        Suggestion::fix(replacement, Applicability::MachineApplicable)
            .with_desc("wrap modifier logic to reduce code size"),
    )
}

/// Visits every expression under `stmts`, stopping as soon as `f` returns `true`.
fn any_expr<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    stmts: &'gcx [Stmt<'gcx>],
    f: impl FnMut(&'gcx Expr<'gcx>) -> bool,
) -> bool {
    let mut finder = ExprFinder { hir, f };
    stmts.iter().any(|stmt| finder.visit_stmt(stmt).is_break())
}

struct ExprFinder<'gcx, F> {
    hir: &'gcx hir::Hir<'gcx>,
    f: F,
}

impl<'gcx, F: FnMut(&'gcx Expr<'gcx>) -> bool> hir::Visit<'gcx> for ExprFinder<'gcx, F> {
    type BreakValue = ();

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<()> {
        if (self.f)(expr) {
            return ControlFlow::Break(());
        }
        self.walk_expr(expr)
    }
}
