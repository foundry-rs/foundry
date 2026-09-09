use super::WriteAfterWrite;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::loop_update},
};
use solar::{
    interface::Span,
    sema::{
        Gcx, Hir,
        hir::{
            BinOpKind, Block, CallArgs, CallOptions, Expr, ExprKind, Function, Res, Stmt, StmtKind,
            VariableId,
        },
    },
};
use std::collections::HashMap;

declare_forge_lint!(
    WRITE_AFTER_WRITE,
    Severity::Gas,
    "write-after-write",
    "redundant storage write; value overwritten before being read"
);

impl<'gcx> LateLintPass<'gcx> for WriteAfterWrite {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        if let Some(body) = func.body {
            Analyzer { ctx, hir: &gcx.hir, pending: HashMap::new() }.check_block(body);
        }
    }
}

/// Tracks state variable writes that no later read has observed yet; a second write to such a
/// variable makes the pending one redundant.
struct Analyzer<'a, 'gcx> {
    ctx: &'a LintContext<'a, 'a>,
    hir: &'gcx Hir<'gcx>,
    pending: HashMap<VariableId, Span>,
}

impl Analyzer<'_, '_> {
    /// Returns whether control flow continues past the block.
    fn check_block(&mut self, block: Block<'_>) -> bool {
        block.stmts.iter().all(|stmt| self.check_stmt(stmt))
    }

    /// Returns whether control flow continues past the statement.
    fn check_stmt(&mut self, stmt: &Stmt<'_>) -> bool {
        match &stmt.kind {
            StmtKind::Expr(expr) => self.process_expr(expr),
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.hir.variable(*var_id).initializer {
                    self.reads(init);
                }
            }
            StmtKind::DeclMulti(_, expr) => self.reads(expr),
            // `emit` only logs, so unlike a call it cannot observe pending writes.
            StmtKind::Emit(expr) => match &expr.peel_parens().kind {
                ExprKind::Call(callee, args, opts) => self.read_call_parts(callee, args, *opts),
                _ => self.reads(expr),
            },
            // Terminal statements: the code after them is unreachable and can never overwrite
            // the pending writes.
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.reads(expr);
                }
                self.pending.clear();
                return false;
            }
            StmtKind::Revert(expr) => {
                self.reads(expr);
                self.pending.clear();
                return false;
            }
            StmtKind::Break | StmtKind::Continue => {
                self.pending.clear();
                return false;
            }
            // Branches are analyzed in isolation so intra-branch pairs are still caught, while
            // outer pending writes are dropped since any branch may observe or skip them.
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.reads(cond);
                let then_continues = self.isolated(|this| this.check_stmt(then_stmt));
                if let Some(else_stmt) = else_stmt {
                    let else_continues = self.isolated(|this| this.check_stmt(else_stmt));
                    return then_continues || else_continues;
                }
            }
            // A loop may run zero times, so it never stops the outer flow.
            StmtKind::Loop(block, source) => {
                self.isolated(|this| {
                    this.check_block(*block)
                        && loop_update(*source).is_none_or(|update| this.check_stmt(update))
                });
            }
            StmtKind::Try(try_stmt) => {
                self.reads(&try_stmt.expr);
                for clause in try_stmt.clauses {
                    self.isolated(|this| this.check_block(clause.block));
                }
            }
            // Nested blocks are sequential: they share pending writes and propagate terminal flow.
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                return self.check_block(*block);
            }
            // The placeholder runs the modified function; inline assembly and errors are opaque.
            StmtKind::Placeholder
            | StmtKind::AssemblyBlock(_)
            | StmtKind::Switch(_)
            | StmtKind::Err(_) => self.pending.clear(),
        }
        true
    }

    /// Runs `f` on a fresh pending set and discards its result set afterwards.
    fn isolated<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.pending.clear();
        let result = f(self);
        self.pending.clear();
        result
    }

    /// Tracks the writes and reads of an expression evaluated for its side effects.
    fn process_expr(&mut self, expr: &Expr<'_>) {
        let expr = expr.peel_parens();
        match &expr.kind {
            ExprKind::Assign(lhs, op, rhs) => {
                // The RHS is evaluated before the assignment takes effect; a compound assignment
                // also reads the current LHS value.
                self.reads(rhs);
                if op.is_none() {
                    self.write_lhs(lhs, expr.span);
                } else {
                    self.reads(lhs);
                }
            }
            // Pre/post increment and decrement read the variable, then write it.
            ExprKind::Unary(op, inner) if op.kind.has_side_effects() => {
                self.reads(inner);
                if let Some(var) = self.state_var(inner) {
                    self.pending.insert(var, expr.span);
                }
            }
            // `delete x` is a pure write.
            ExprKind::Delete(inner) => match self.state_var(inner) {
                Some(var) => self.write(var, expr.span),
                None => self.reads(inner),
            },
            // Any call may observe storage through re-entrancy or view semantics.
            ExprKind::Call(callee, args, opts) => {
                self.read_call_parts(callee, args, *opts);
                self.pending.clear();
            }
            _ => self.reads(expr),
        }
    }

    /// Records a plain `=` write; tuple destructuring records each component with its own span.
    fn write_lhs(&mut self, lhs: &Expr<'_>, span: Span) {
        match &lhs.peel_parens().kind {
            ExprKind::Tuple(exprs) => {
                exprs.iter().flatten().for_each(|e| self.write_lhs(e, e.span));
            }
            _ => match self.state_var(lhs) {
                Some(var) => self.write(var, span),
                // Index/member access: computing the slot reads the base.
                None => self.reads(lhs),
            },
        }
    }

    fn write(&mut self, var: VariableId, span: Span) {
        if let Some(prev_span) = self.pending.insert(var, span) {
            self.ctx.emit(&WRITE_AFTER_WRITE, prev_span);
        }
    }

    /// Removes every state variable read by `expr` from the pending writes. Nested writes are
    /// tracked through [`Self::process_expr`].
    fn reads(&mut self, expr: &Expr<'_>) {
        let expr = expr.peel_parens();
        match &expr.kind {
            ExprKind::Ident(reses) => {
                for var in reses.iter().filter_map(Res::as_variable) {
                    self.pending.remove(&var);
                }
            }
            ExprKind::Assign(..) | ExprKind::Delete(_) => self.process_expr(expr),
            // Short-circuit operands and ternary arms may not execute, so they are isolated to
            // avoid false positives on the conditional path.
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                self.reads(lhs);
                self.isolated(|this| this.reads(rhs));
            }
            ExprKind::Ternary(cond, then_expr, else_expr) => {
                self.reads(cond);
                self.isolated(|this| this.reads(then_expr));
                self.isolated(|this| this.reads(else_expr));
            }
            ExprKind::Call(callee, args, opts) => {
                self.read_call_parts(callee, args, *opts);
                self.pending.clear();
            }
            ExprKind::Binary(lhs, _, rhs) => {
                self.reads(lhs);
                self.reads(rhs);
            }
            ExprKind::Unary(_, inner) | ExprKind::Payable(inner) | ExprKind::Member(inner, _) => {
                self.reads(inner);
            }
            ExprKind::Index(base, index) => {
                self.reads(base);
                if let Some(index) = index {
                    self.reads(index);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.reads(base);
                for expr in [*start, *end].into_iter().flatten() {
                    self.reads(expr);
                }
            }
            ExprKind::Tuple(exprs) => exprs.iter().flatten().for_each(|e| self.reads(e)),
            ExprKind::Array(exprs) => exprs.iter().for_each(|e| self.reads(e)),
            ExprKind::Lit(_) | ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
            ExprKind::YulMember(..) | ExprKind::Err(_) => self.pending.clear(),
        }
    }

    /// Callee, arguments and call options are all evaluated before the call itself.
    fn read_call_parts(
        &mut self,
        callee: &Expr<'_>,
        args: &CallArgs<'_>,
        opts: Option<&CallOptions<'_>>,
    ) {
        self.reads(callee);
        for arg in args.exprs() {
            self.reads(arg);
        }
        for opt in opts.into_iter().flat_map(|opts| opts.args) {
            self.reads(&opt.value);
        }
    }

    /// The state variable a bare identifier refers to.
    fn state_var(&self, expr: &Expr<'_>) -> Option<VariableId> {
        let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return None };
        reses
            .iter()
            .filter_map(Res::as_variable)
            .find(|&var| self.hir.variable(var).is_state_variable())
    }
}
