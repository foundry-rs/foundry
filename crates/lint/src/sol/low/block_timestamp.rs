use super::BlockTimestamp;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            any_subexpr, branch_always_exits, builtins, function_ids, is_builtin, loop_stmts,
            tuple_elems,
        },
    },
};
use solar::{
    ast::Visibility,
    interface::{kw, sym},
    sema::{
        Gcx, Hir,
        builtins::Builtin,
        hir::{
            BinOpKind, Expr, ExprKind, Function, FunctionId, Res, Stmt, StmtKind, VariableId, Visit,
        },
    },
};
use std::{collections::HashSet, convert::Infallible, ops::ControlFlow};

declare_forge_lint!(
    BLOCK_TIMESTAMP,
    Severity::Low,
    "block-timestamp",
    "usage of `block.timestamp` in a comparison may be manipulated by validators"
);

impl<'gcx> LateLintPass<'gcx> for BlockTimestamp {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        let Some(body) = func.body else { return };
        // The contract's own internal helpers that return `block.timestamp` directly.
        let helpers = func
            .contract
            .map(|c| gcx.hir.contract(c).functions())
            .into_iter()
            .flatten()
            .filter(|&id| {
                let helper = gcx.hir.function(id);
                matches!(helper.visibility, Visibility::Internal | Visibility::Private)
                    && helper.body.is_some_and(|body| returns_timestamp(body.stmts))
            })
            .collect();
        Checker { ctx, gcx, helpers, aliases: HashSet::new() }.block(body.stmts);
    }
}

/// Flow-sensitive walk reporting comparisons involving `block.timestamp`, a helper returning it,
/// or a local holding a value derived from either.
struct Checker<'a, 's, 'c, 'gcx> {
    ctx: &'a LintContext<'s, 'c>,
    gcx: Gcx<'gcx>,
    helpers: Vec<FunctionId>,
    /// Locals currently holding a timestamp-derived value.
    aliases: HashSet<VariableId>,
}

impl<'gcx> Checker<'_, '_, '_, 'gcx> {
    /// Walks statements in order, stopping at the first one control cannot continue past.
    fn block(&mut self, stmts: impl IntoIterator<Item = &'gcx Stmt<'gcx>>) {
        for stmt in stmts {
            let _ = self.visit_stmt(stmt);
            if branch_always_exits(stmt) {
                break;
            }
        }
    }

    /// Walks an alternative arm on a copy of the current aliases; when control can continue past
    /// `stmts`, the aliases the arm leaves are added to `merged`.
    fn arm(
        &mut self,
        merged: &mut HashSet<VariableId>,
        stmts: impl IntoIterator<Item = &'gcx Stmt<'gcx>>,
        walk: impl FnOnce(&mut Self),
    ) {
        let saved = self.aliases.clone();
        walk(self);
        let aliases = std::mem::replace(&mut self.aliases, saved);
        if !stmts.into_iter().any(branch_always_exits) {
            merged.extend(aliases);
        }
    }

    /// Binds the variables of an lvalue (tuple targets included) to whether they now hold a
    /// timestamp-derived value.
    fn bind(&mut self, lhs: &Expr<'_>, is_source: bool) {
        match &lhs.peel_parens().kind {
            ExprKind::Tuple(elems) => elems.iter().flatten().for_each(|e| self.bind(e, is_source)),
            ExprKind::Ident(reses) => {
                for var in reses.iter().filter_map(Res::as_variable) {
                    self.set_alias(var, is_source);
                }
            }
            _ => {}
        }
    }

    fn set_alias(&mut self, var: VariableId, is_source: bool) {
        if self.gcx.hir.variable(var).is_local_or_return() {
            if is_source {
                self.aliases.insert(var);
            } else {
                self.aliases.remove(&var);
            }
        }
    }

    /// Whether each of `n` targets receives a timestamp-derived value from `rhs`: element-wise
    /// for a matching tuple, otherwise `rhs` as a whole for every target.
    fn source_values(&self, rhs: &Expr<'_>, n: usize) -> Vec<bool> {
        match tuple_elems(rhs) {
            Some(elems) if elems.len() == n => {
                elems.iter().map(|e| e.is_some_and(|e| self.is_source_value(e))).collect()
            }
            _ => vec![self.is_source_value(rhs); n],
        }
    }

    /// True if the value of `expr` derives from a timestamp source: the source itself, or one
    /// flowing through arithmetic, unary operators, a ternary arm or a parenthesized tuple.
    fn is_source_value(&self, expr: &Expr<'_>) -> bool {
        self.is_source(expr)
            || match &expr.peel_parens().kind {
                ExprKind::Binary(lhs, op, rhs) if !is_cmp(op.kind) => {
                    self.is_source_value(lhs) || self.is_source_value(rhs)
                }
                ExprKind::Unary(_, inner)
                | ExprKind::Payable(inner)
                | ExprKind::YulMember(inner, _) => self.is_source_value(inner),
                ExprKind::Ternary(_, then_expr, else_expr) => {
                    self.is_source_value(then_expr) || self.is_source_value(else_expr)
                }
                ExprKind::Tuple([Some(inner)]) => self.is_source_value(inner),
                _ => false,
            }
    }

    /// `block.timestamp`, a call to a helper returning it, or an alias of either.
    fn is_source(&self, expr: &Expr<'_>) -> bool {
        is_block_timestamp(expr)
            || expr.as_variable().is_some_and(|var| self.aliases.contains(&var))
            || matches!(&expr.peel_parens().kind, ExprKind::Call(callee, ..)
                if function_ids(callee).any(|id| self.helpers.contains(&id)))
    }
}

impl<'gcx> Visit<'gcx> for Checker<'_, '_, '_, 'gcx> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Infallible> {
        match &stmt.kind {
            StmtKind::DeclSingle(var) => {
                if let Some(init) = self.gcx.hir.variable(*var).initializer {
                    self.visit_expr(init)?;
                    let is_source = self.is_source_value(init);
                    self.set_alias(*var, is_source);
                }
            }
            StmtKind::DeclMulti(vars, expr) => {
                self.visit_expr(expr)?;
                for (var, is_source) in vars.iter().zip(self.source_values(expr, vars.len())) {
                    if let Some(var) = var {
                        self.set_alias(*var, is_source);
                    }
                }
            }
            StmtKind::Block(block)
            | StmtKind::UncheckedBlock(block)
            | StmtKind::AssemblyBlock(block) => {
                self.block(block.stmts);
            }
            // Only the arms control can continue past contribute their aliases.
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.visit_expr(cond)?;
                let mut merged = HashSet::new();
                self.arm(&mut merged, std::slice::from_ref(*then_stmt), |s| {
                    let _ = s.visit_stmt(then_stmt);
                });
                match else_stmt {
                    Some(else_stmt) => {
                        self.arm(&mut merged, std::slice::from_ref(*else_stmt), |s| {
                            let _ = s.visit_stmt(else_stmt);
                        })
                    }
                    None => merged.extend(self.aliases.iter().copied()),
                }
                self.aliases = merged;
            }
            StmtKind::Loop(block, source) => {
                let mut merged = self.aliases.clone();
                let stmts = loop_stmts(*block, *source);
                self.arm(&mut merged, stmts.clone(), |s| s.block(stmts));
                self.aliases = merged;
            }
            StmtKind::Try(try_stmt) => {
                self.visit_expr(&try_stmt.expr)?;
                let mut merged = self.aliases.clone();
                for clause in try_stmt.clauses {
                    self.arm(&mut merged, clause.block.stmts, |s| s.block(clause.block.stmts));
                }
                self.aliases = merged;
            }
            StmtKind::Switch(switch) => {
                self.visit_expr(switch.selector)?;
                let mut merged = self.aliases.clone();
                for case in switch.cases {
                    self.arm(&mut merged, case.body.stmts, |s| s.block(case.body.stmts));
                }
                self.aliases = merged;
            }
            _ => self.walk_stmt(stmt)?,
        }
        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Infallible> {
        match &expr.peel_parens().kind {
            // The right-hand side is evaluated first, against the aliases before the write.
            ExprKind::Assign(lhs, op, rhs) => {
                self.visit_expr(rhs)?;
                if op.is_some() {
                    self.visit_expr(lhs)?;
                    let is_source = self.is_source_value(rhs) || self.is_source_value(lhs);
                    self.bind(lhs, is_source);
                } else if let Some(elems) = tuple_elems(lhs) {
                    for (elem, is_source) in elems.iter().zip(self.source_values(rhs, elems.len()))
                    {
                        if let Some(elem) = elem {
                            self.bind(elem, is_source);
                        }
                    }
                } else {
                    let is_source = self.is_source_value(rhs);
                    self.bind(lhs, is_source);
                }
            }
            ExprKind::Binary(lhs, op, rhs) => {
                if is_cmp(op.kind) && (self.contains_source(lhs) || self.contains_source(rhs)) {
                    self.ctx.emit(&BLOCK_TIMESTAMP, expr.span);
                }
                self.visit_expr(lhs)?;
                self.visit_expr(rhs)?;
            }
            ExprKind::Ternary(cond, then_expr, else_expr) => {
                self.visit_expr(cond)?;
                let mut merged = HashSet::new();
                self.arm(&mut merged, &[], |s| {
                    let _ = s.visit_expr(then_expr);
                });
                self.visit_expr(else_expr)?;
                self.aliases.extend(merged);
            }
            _ => self.walk_expr(expr)?,
        }
        ControlFlow::Continue(())
    }
}

impl<'gcx> Checker<'_, '_, '_, 'gcx> {
    /// True if `expr` or any subexpression is a timestamp source.
    fn contains_source(&self, expr: &'gcx Expr<'gcx>) -> bool {
        any_subexpr(expr, |e| self.is_source(e))
    }
}

const fn is_cmp(kind: BinOpKind) -> bool {
    matches!(
        kind,
        BinOpKind::Lt
            | BinOpKind::Le
            | BinOpKind::Gt
            | BinOpKind::Ge
            | BinOpKind::Eq
            | BinOpKind::Ne
    )
}

/// `block.timestamp`, or the Yul `timestamp()` builtin.
fn is_block_timestamp(expr: &Expr<'_>) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Member(base, member)
        if member.name == kw::Timestamp && is_builtin(base, sym::block))
        || builtins(expr).any(|b| b == Builtin::BlockTimestamp)
}

/// True if a `return` reachable through plain blocks and `if` arms mentions `block.timestamp`.
fn returns_timestamp(stmts: &[Stmt<'_>]) -> bool {
    stmts.iter().any(|stmt| match &stmt.kind {
        StmtKind::Return(Some(expr)) => any_subexpr(expr, is_block_timestamp),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => returns_timestamp(block.stmts),
        StmtKind::If(_, then_stmt, else_stmt) => {
            returns_timestamp(std::slice::from_ref(*then_stmt))
                || else_stmt.is_some_and(|e| returns_timestamp(std::slice::from_ref(e)))
        }
        _ => false,
    })
}
