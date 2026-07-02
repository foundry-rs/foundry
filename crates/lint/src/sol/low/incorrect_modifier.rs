use super::IncorrectModifier;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    sema::{
        Gcx, Hir,
        builtins::Builtin,
        hir::{Block, Expr, ExprKind, Function, LoopSource, Res, Stmt, StmtKind},
    },
};

declare_forge_lint!(
    INCORRECT_MODIFIER,
    Severity::Low,
    "incorrect-modifier",
    "modifier can finish without executing the modified function"
);

impl<'hir> LateLintPass<'hir> for IncorrectModifier {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        _hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let (ast::FunctionKind::Modifier, Some(body)) = (func.kind, func.body) else {
            return;
        };

        if block_outcome(body).can_skip_placeholder() {
            ctx.emit(&INCORRECT_MODIFIER, func.span);
        }
    }
}

/// Summary of how control flow can leave a statement or block *without* having executed the
/// placeholder (`_`) or reverted.
///
/// Each flag tracks whether there is at least one such path. If every path reaches `_` or reverts,
/// all flags are `false` ([`Outcome::COVERED`]).
#[derive(Clone, Copy)]
pub(crate) struct Outcome {
    /// Control can reach the end of the construct normally and continue to the next statement.
    falls_through: bool,
    /// Control can exit the modifier via `return` before reaching `_`.
    returns: bool,
    /// Control can exit the enclosing loop via `break` before reaching `_`.
    breaks: bool,
    /// Control can jump to the enclosing loop's next iteration via `continue` before reaching `_`.
    continues: bool,
}

impl Outcome {
    /// Every path reaches `_` or reverts.
    const COVERED: Self =
        Self { falls_through: false, returns: false, breaks: false, continues: false };
    const FALLTHROUGH: Self = Self { falls_through: true, ..Self::COVERED };
    const RETURNS: Self = Self { returns: true, ..Self::COVERED };
    const BREAKS: Self = Self { breaks: true, ..Self::COVERED };
    const CONTINUES: Self = Self { continues: true, ..Self::COVERED };

    /// Whether the modifier body can finish without executing `_`. Only fall-through and `return`
    /// reach the modifier's end; `break`/`continue` are always consumed by an enclosing loop.
    pub(crate) const fn can_skip_placeholder(self) -> bool {
        self.falls_through || self.returns
    }

    const fn merge(self, other: Self) -> Self {
        Self {
            falls_through: self.falls_through || other.falls_through,
            returns: self.returns || other.returns,
            breaks: self.breaks || other.breaks,
            continues: self.continues || other.continues,
        }
    }
}

pub(crate) fn block_outcome(block: Block<'_>) -> Outcome {
    let mut outcome = Outcome::FALLTHROUGH;
    for stmt in block.stmts {
        // Once a statement cannot fall through, the rest of the block is unreachable.
        if !outcome.falls_through {
            return outcome;
        }
        let stmt_outcome = stmt_outcome(stmt);
        outcome = Outcome {
            falls_through: stmt_outcome.falls_through,
            returns: outcome.returns || stmt_outcome.returns,
            breaks: outcome.breaks || stmt_outcome.breaks,
            continues: outcome.continues || stmt_outcome.continues,
        };
    }
    outcome
}

fn stmt_outcome(stmt: &Stmt<'_>) -> Outcome {
    match &stmt.kind {
        StmtKind::Placeholder => Outcome::COVERED,
        StmtKind::Return(_) => Outcome::RETURNS,
        StmtKind::Break => Outcome::BREAKS,
        StmtKind::Continue => Outcome::CONTINUES,
        StmtKind::Expr(expr) => call_outcome(expr).unwrap_or(Outcome::FALLTHROUGH),
        StmtKind::Revert(_) => Outcome::COVERED,
        StmtKind::Block(block)
        | StmtKind::UncheckedBlock(block)
        | StmtKind::AssemblyBlock(block) => block_outcome(*block),
        StmtKind::If(_, then_stmt, else_stmt) => {
            let then_outcome = stmt_outcome(then_stmt);
            let else_outcome = else_stmt.map_or(Outcome::FALLTHROUGH, stmt_outcome);
            then_outcome.merge(else_outcome)
        }
        StmtKind::Loop(block, source) => {
            // `for`/`while`/`do-while` are all desugared to a `Loop` whose body holds the condition
            // as a synthetic `else break`. The loop can be left (and thus fall through to the
            // following statement) via a `break`, including that synthetic condition break; a loop
            // without any `break` (e.g. `for (;;)`) never falls through. For `do-while` the
            // condition sits *after* the body, so a `continue` in the body also reaches it and can
            // exit the loop. `break`/`continue` are otherwise consumed by the loop; only `return`
            // keeps escaping toward the modifier's end.
            let body = block_outcome(*block);
            let falls_through =
                body.breaks || (matches!(source, LoopSource::DoWhile) && body.continues);
            Outcome { falls_through, returns: body.returns, ..Outcome::COVERED }
        }
        StmtKind::Try(try_stmt) => {
            // Every execution enters exactly one clause (the `returns` clause on success or a
            // matching `catch`), or the call reverts uncaught. There is no implicit fall-through
            // path that skips all clauses, so start from `COVERED`.
            let mut outcome = Outcome::COVERED;
            for clause in try_stmt.clauses {
                outcome = outcome.merge(block_outcome(clause.block));
            }
            outcome
        }
        StmtKind::Switch(switch) => {
            // A Yul `switch` value that matches no `case` falls through unless a `default` clause
            // is present (stored last, with no constant).
            let has_default = switch.cases.last().is_some_and(|case| case.constant.is_none());
            let mut outcome = if has_default { Outcome::COVERED } else { Outcome::FALLTHROUGH };
            for case in switch.cases {
                outcome = outcome.merge(block_outcome(case.body));
            }
            outcome
        }
        StmtKind::DeclSingle(_)
        | StmtKind::DeclMulti(_, _)
        | StmtKind::Emit(_)
        | StmtKind::Err(_) => Outcome::FALLTHROUGH,
    }
}

/// Classifies a statement-level call expression that terminates the current path before reaching
/// `_`, if any. Covers both the Solidity `revert`/`revert(...)` builtins and the Yul halting
/// builtins reachable when recursing into an `assembly { .. }` block.
///
/// - Failing halts (`revert`, Yul `revert`/`invalid`) leave every path either reverting or reaching
///   `_`, so they are [`Outcome::COVERED`] (not flagged).
/// - Successful halts (Yul `return`/`stop`, `selfdestruct`) let the surrounding call finish
///   *without* running the modified function body, which is exactly what this lint flags, so they
///   behave like a `return` ([`Outcome::RETURNS`]).
fn call_outcome(expr: &Expr<'_>) -> Option<Outcome> {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return None };
    let ExprKind::Ident(resolutions) = &callee.peel_parens().kind else { return None };
    resolutions.iter().find_map(|res| match res {
        Res::Builtin(
            Builtin::Revert | Builtin::RevertMsg | Builtin::YulRevert | Builtin::YulInvalid,
        ) => Some(Outcome::COVERED),
        Res::Builtin(Builtin::Require | Builtin::Assert)
            if args.exprs().next().is_some_and(is_literal_false) =>
        {
            Some(Outcome::COVERED)
        }
        Res::Builtin(
            Builtin::YulReturn
            | Builtin::YulStop
            | Builtin::YulSelfdestruct
            | Builtin::Selfdestruct,
        ) => Some(Outcome::RETURNS),
        _ => None,
    })
}

fn is_literal_false(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Lit(lit) if matches!(lit.kind, ast::LitKind::Bool(false))
    )
}
