//! Inspect modifier bodies and map modifier params back to caller args.

use super::primitives::underlying_var;
use solar::sema::hir::{self, FunctionKind, ItemId, Modifier, Stmt, StmtKind, VariableId};
use std::collections::HashMap;

/// Counts placeholder (`_;`) statements anywhere in `stmts`. Lints typically
/// refuse to reason about modifiers whose count isn't `1`.
pub fn count_placeholders(stmts: &[Stmt<'_>]) -> usize {
    fn count_in(stmt: &Stmt<'_>) -> usize {
        match &stmt.kind {
            StmtKind::Placeholder => 1,
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => count_placeholders(b.stmts),
            StmtKind::If(_, t, e) => count_in(t) + e.as_ref().map_or(0, |s| count_in(s)),
            StmtKind::Loop(b, _) => count_placeholders(b.stmts),
            StmtKind::Try(t) => t.clauses.iter().map(|c| count_placeholders(c.block.stmts)).sum(),
            _ => 0,
        }
    }
    stmts.iter().map(count_in).sum()
}

/// Calls `on_stmt` for every statement before the single top-level `_;` of
/// `invocation`'s modifier body. Returns `false` (and skips the callback)
/// when the modifier has no body or doesn't have exactly one top-level `_;`.
pub fn scan_modifier_prefix<F>(
    hir: &hir::Hir<'_>,
    invocation: &Modifier<'_>,
    mut on_stmt: F,
) -> bool
where
    F: FnMut(&Stmt<'_>),
{
    let ItemId::Function(fid) = invocation.id else { return false };
    let modifier = hir.function(fid);
    if !matches!(modifier.kind, FunctionKind::Modifier) {
        return false;
    }
    let Some(body) = modifier.body else { return false };
    if count_placeholders(body.stmts) != 1 {
        return false;
    }
    let Some(idx) = body.stmts.iter().position(|s| matches!(s.kind, StmtKind::Placeholder)) else {
        return false;
    };
    for s in &body.stmts[..idx] {
        on_stmt(s);
    }
    true
}

/// Maps each modifier parameter to the caller-side variable id, when the
/// invocation's argument is a bare identifier (or trivially-peeled cast).
pub fn modifier_param_to_caller(
    hir: &hir::Hir<'_>,
    invocation: &Modifier<'_>,
) -> HashMap<VariableId, VariableId> {
    let ItemId::Function(fid) = invocation.id else { return HashMap::new() };
    let modifier = hir.function(fid);
    let mut out = HashMap::new();
    for (i, arg) in invocation.args.exprs().enumerate() {
        if let (Some(&mp), Some(caller)) = (modifier.parameters.get(i), underlying_var(arg)) {
            out.insert(mp, caller);
        }
    }
    out
}
