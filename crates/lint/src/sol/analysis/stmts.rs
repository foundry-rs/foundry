//! Statement-shape probes over Solar HIR.

use super::is_exit_call;
use solar::{
    ast::FunctionKind,
    sema::hir::{self, Expr, FunctionId, LoopSource, Stmt, StmtKind, Visit},
};
use std::ops::ControlFlow;

/// Runs `f` on every statement (pre-order, nested ones included) until it breaks.
struct StmtVisitor<'gcx, F> {
    hir: &'gcx hir::Hir<'gcx>,
    f: F,
}

impl<'gcx, F: FnMut(&'gcx Stmt<'gcx>) -> ControlFlow<()>> Visit<'gcx> for StmtVisitor<'gcx, F> {
    type BreakValue = ();

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<()> {
        (self.f)(stmt)?;
        self.walk_stmt(stmt)
    }
}

/// Runs `f` on every statement of `stmts` and their nested statements (pre-order) until it breaks.
pub fn visit_stmts<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    stmts: impl IntoIterator<Item = &'gcx Stmt<'gcx>>,
    f: impl FnMut(&'gcx Stmt<'gcx>) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let mut visitor = StmtVisitor { hir, f };
    stmts.into_iter().try_for_each(|stmt| visitor.visit_stmt(stmt))
}

/// The expression directly owned by `stmt` (nested statements excluded).
pub fn stmt_expr<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    stmt: &'gcx Stmt<'gcx>,
) -> Option<&'gcx Expr<'gcx>> {
    match stmt.kind {
        StmtKind::DeclSingle(var_id) => hir.variable(var_id).initializer,
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Expr(expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr)
        | StmtKind::Return(Some(expr))
        | StmtKind::If(expr, ..) => Some(expr),
        StmtKind::Try(try_stmt) => Some(&try_stmt.expr),
        _ => None,
    }
}

/// True when executing `stmt` provably prevents control from continuing past it: `return`,
/// `revert`, `selfdestruct`, `require(false, ..)` / `assert(false)`, a block containing any such
/// statement, an `if` whose both arms exit, a `try` whose every clause exits, or a `do-while`
/// whose body exits without `break`/`continue`.
pub fn branch_always_exits(stmt: &Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Revert(_) => true,
        StmtKind::Expr(expr) => is_exit_call(expr),
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => b.stmts.iter().any(branch_always_exits),
        StmtKind::If(_, t, Some(e)) => branch_always_exits(t) && branch_always_exits(e),
        StmtKind::Loop(block, LoopSource::DoWhile) => {
            let user = do_while_user_stmts(block.stmts);
            !stmts_break_or_continue(user) && user.iter().any(branch_always_exits)
        }
        StmtKind::Try(t) => {
            !t.clauses.is_empty()
                && t.clauses.iter().all(|c| c.block.stmts.iter().any(branch_always_exits))
        }
        _ => false,
    }
}

/// The `for` update statement of a loop, which runs after every iteration.
pub const fn loop_update<'gcx>(source: LoopSource<'gcx>) -> Option<&'gcx Stmt<'gcx>> {
    match source {
        LoopSource::For { update } => update,
        LoopSource::While | LoopSource::DoWhile => None,
    }
}

/// The statements of one loop iteration: the body followed by the `for` update, if any.
pub fn loop_stmts<'gcx>(
    block: hir::Block<'gcx>,
    source: LoopSource<'gcx>,
) -> impl Iterator<Item = &'gcx Stmt<'gcx>> + Clone {
    block.stmts.iter().chain(loop_update(source))
}

/// Number of `_` placeholders in `stmts`, recursing into nested control flow.
pub fn count_placeholders(stmts: &[Stmt<'_>]) -> usize {
    stmts.iter().map(count_placeholders_in_stmt).sum()
}

fn count_placeholders_in_stmt(stmt: &Stmt<'_>) -> usize {
    match &stmt.kind {
        StmtKind::Placeholder => 1,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => count_placeholders(b.stmts),
        StmtKind::Loop(b, source) => loop_stmts(*b, *source).map(count_placeholders_in_stmt).sum(),
        StmtKind::If(_, t, e) => {
            count_placeholders_in_stmt(t) + e.as_ref().map_or(0, |e| count_placeholders_in_stmt(e))
        }
        StmtKind::Try(t) => t.clauses.iter().map(|c| count_placeholders(c.block.stmts)).sum(),
        _ => 0,
    }
}

/// Collects the statements executed before the first placeholder of a modifier body, following
/// nested blocks. Returns `None` when the placeholder is not reached unconditionally (e.g. it is
/// inside an `if`, loop or `try`).
pub fn stmts_before_placeholder<'a, 'gcx>(
    stmts: &'a [Stmt<'gcx>],
    out: &mut Vec<&'a Stmt<'gcx>>,
) -> Option<()> {
    for (i, stmt) in stmts.iter().enumerate() {
        match &stmt.kind {
            StmtKind::Placeholder => {
                out.extend(&stmts[..i]);
                return Some(());
            }
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b) if count_placeholders(b.stmts) > 0 => {
                out.extend(&stmts[..i]);
                return stmts_before_placeholder(b.stmts, out);
            }
            _ if count_placeholders_in_stmt(stmt) > 0 => return None,
            _ => {}
        }
    }
    None
}

/// Strips the trailing `if (cond) break;` that lowers `do { ... } while (cond);`.
pub fn do_while_user_stmts<'a, 'gcx>(stmts: &'a [Stmt<'gcx>]) -> &'a [Stmt<'gcx>] {
    match stmts.split_last() {
        Some((last, rest)) if is_loop_termination_if(last) => rest,
        _ => stmts,
    }
}

/// `if (...) break;` as synthesized by the `do-while` lowering.
pub fn is_loop_termination_if(stmt: &Stmt<'_>) -> bool {
    let StmtKind::If(_, t, e) = &stmt.kind else { return false };
    is_break_stmt(t) || e.as_ref().is_some_and(|e| is_break_stmt(e))
}

/// `break`, possibly wrapped in single-statement blocks.
pub fn is_break_stmt(stmt: &Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Break => true,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => {
            b.stmts.len() == 1 && is_break_stmt(&b.stmts[0])
        }
        _ => false,
    }
}

/// `break`/`continue` targeting the current loop (nested loops shadow them).
pub fn stmts_break_or_continue(stmts: &[Stmt<'_>]) -> bool {
    stmts.iter().any(|stmt| match &stmt.kind {
        StmtKind::Break | StmtKind::Continue => true,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => stmts_break_or_continue(b.stmts),
        StmtKind::If(_, t, e) => {
            stmts_break_or_continue(std::slice::from_ref(*t))
                || e.is_some_and(|e| stmts_break_or_continue(std::slice::from_ref(e)))
        }
        StmtKind::Try(t) => t.clauses.iter().any(|c| stmts_break_or_continue(c.block.stmts)),
        _ => false,
    })
}

/// The statements a modifier runs before its unique `_;`, when that placeholder is reached
/// unconditionally. `None` for non-modifiers, bodiless modifiers and conditional placeholders.
pub fn modifier_prefix<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    fid: FunctionId,
) -> Option<Vec<&'gcx Stmt<'gcx>>> {
    let modifier = hir.function(fid);
    let body = modifier.body.filter(|_| modifier.kind == FunctionKind::Modifier)?;
    if count_placeholders(body.stmts) != 1 {
        return None;
    }
    let mut prefix = Vec::new();
    stmts_before_placeholder(body.stmts, &mut prefix)?;
    Some(prefix)
}
