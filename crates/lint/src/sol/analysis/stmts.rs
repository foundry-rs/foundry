//! Statement-shape probes over Solar HIR.

use super::is_exit_call;
use solar::sema::hir::{LoopSource, Stmt, StmtKind};

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

/// Number of `_` placeholders in `stmts`, recursing into nested control flow.
pub fn count_placeholders(stmts: &[Stmt<'_>]) -> usize {
    stmts.iter().map(count_placeholders_in_stmt).sum()
}

fn count_placeholders_in_stmt(stmt: &Stmt<'_>) -> usize {
    match &stmt.kind {
        StmtKind::Placeholder => 1,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) | StmtKind::Loop(b, _) => {
            count_placeholders(b.stmts)
        }
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
pub fn stmts_before_placeholder<'a, 'hir>(
    stmts: &'a [Stmt<'hir>],
    out: &mut Vec<&'a Stmt<'hir>>,
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
pub fn do_while_user_stmts<'a, 'hir>(stmts: &'a [Stmt<'hir>]) -> &'a [Stmt<'hir>] {
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
