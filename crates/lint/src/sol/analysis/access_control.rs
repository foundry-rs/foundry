//! Access-control guard detection: whether a function dominates its body with a check comparing
//! the caller against contract state, and which state that check depends on.

use super::{
    branch_always_exits, function_ids, is_require_or_assert, is_sender_member, lhs_local_var,
    loop_stmts, stmt_expr, underlying_var, visit_stmts,
};
use solar::sema::hir::{
    self, BinOpKind, Expr, ExprKind, FunctionId, Stmt, StmtKind, UnOpKind, VariableId,
};
use std::{collections::HashSet, iter, ops::ControlFlow};

/// True when the function or one of its modifiers contains a dominating access check.
pub fn is_protected<'gcx>(hir: &'gcx hir::Hir<'gcx>, func_id: FunctionId) -> bool {
    modifiers_and_self(hir, func_id).any(|id| has_access_guard(hir, id, &mut HashSet::new()))
}

/// The modifiers of `func_id` that resolve to functions, followed by `func_id` itself.
pub fn modifiers_and_self<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    func_id: FunctionId,
) -> impl Iterator<Item = FunctionId> + 'gcx {
    hir.function(func_id)
        .modifiers
        .iter()
        .filter_map(|modifier| modifier.id.as_function())
        .chain(iter::once(func_id))
}

/// Whether `func_id` checks the caller before its `_` placeholder (anywhere for functions): a
/// guarding `if`, a `require`/`assert` on an access check, or a call into a function that does.
/// Bodyless declarations (interface functions, virtual modifiers) fall back to a name heuristic.
pub fn has_access_guard<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    if !seen.insert(func_id) {
        return false;
    }
    let func = hir.function(func_id);
    match func.body {
        Some(body) => for_each_guard(hir, body, seen, &mut |_| ControlFlow::Break(())).is_break(),
        None => looks_like_access_control(func),
    }
}

/// State variables the access checks of `func_id` and its modifiers (up to `_`) depend on.
pub fn guard_vars<'gcx>(hir: &'gcx hir::Hir<'gcx>, func_id: FunctionId) -> HashSet<VariableId> {
    let mut out = HashSet::new();
    for id in modifiers_and_self(hir, func_id) {
        let Some(body) = hir.function(id).body else { continue };
        let mut seen = HashSet::from([id]);
        let _ = for_each_guard(hir, body, &mut HashSet::from([id]), &mut |guard| {
            match guard {
                Guard::Check(cond) => expr_state_vars(hir, cond, &mut seen, &mut out),
                Guard::Call(callee_id) => function_state_vars(hir, callee_id, &mut seen, &mut out),
            }
            ControlFlow::Continue(())
        });
    }
    out
}

/// A function whose name marks it as an access check (`auth`, `onlyOwner`, `_checkRole`, ...)
/// and that returns nothing, so calling it for its effect is meaningful.
pub fn looks_like_access_control(func: &hir::Function<'_>) -> bool {
    let Some(name) = func.name else { return false };
    if !func.returns.is_empty() {
        return false;
    }
    let lower = name.as_str().to_ascii_lowercase();
    matches!(lower.as_str(), "auth" | "requiresauth" | "restricted")
        || ["only", "check", "_check"].iter().any(|prefix| {
            ["admin", "guardian", "manager", "owner", "role"]
                .iter()
                .any(|role| lower.starts_with(&format!("{prefix}{role}")))
        })
}

/// `Some(true)` when `expr` holding means the caller is authorized, `Some(false)` when it means
/// the caller is *not* authorized, `None` when `expr` is not an access check. An access check
/// reads `msg.sender`/`tx.origin` (directly, through `aliases` or through a helper) and state
/// (directly or through a helper).
pub fn access_check_polarity<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    expr: &Expr<'_>,
    aliases: &HashSet<VariableId>,
) -> Option<bool> {
    let is_check = |sender: &Expr<'_>, authority: &Expr<'_>| {
        expr_reads_sender(hir, sender, &mut HashSet::new(), aliases)
            && expr_reads_state(hir, authority)
    };
    match &expr.peel_parens().kind {
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            access_check_polarity(hir, inner, aliases).map(|polarity| !polarity)
        }
        ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
            // `a && b` is authorized as soon as one side is; `a || b` is unauthorized as soon as
            // one side is. The opposite polarity needs both sides.
            let dominant = op.kind == BinOpKind::And;
            let lhs = access_check_polarity(hir, lhs, aliases);
            let rhs = access_check_polarity(hir, rhs, aliases);
            if lhs == Some(dominant) || rhs == Some(dominant) {
                Some(dominant)
            } else if lhs == Some(!dominant) && rhs == Some(!dominant) {
                Some(!dominant)
            } else {
                None
            }
        }
        ExprKind::Binary(lhs, op, rhs)
            if matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne)
                && (is_check(lhs, rhs) || is_check(rhs, lhs)) =>
        {
            Some(op.kind == BinOpKind::Eq)
        }
        _ => is_check(expr, expr).then_some(true),
    }
}

/// Locals initialized or assigned from a value that reads `msg.sender`.
pub fn sender_aliases<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    stmts: impl IntoIterator<Item = &'gcx Stmt<'gcx>>,
) -> HashSet<VariableId> {
    let mut aliases = HashSet::new();
    let _ = visit_stmts(hir, stmts, |stmt| {
        let (var_id, value) = match stmt.kind {
            StmtKind::DeclSingle(var_id) => (Some(var_id), hir.variable(var_id).initializer),
            StmtKind::Expr(expr) => match &expr.peel_parens().kind {
                ExprKind::Assign(lhs, _, rhs) => (lhs_local_var(hir, lhs), Some(*rhs)),
                _ => (None, None),
            },
            _ => (None, None),
        };
        if let Some(var_id) = var_id
            && let Some(value) = value
            && expr_reads_sender(hir, value, &mut HashSet::new(), &aliases)
        {
            aliases.insert(var_id);
        }
        ControlFlow::Continue(())
    });
    aliases
}

/// Whether `expr` reads `msg.sender`/`tx.origin`, one of `aliases`, or calls a user function that
/// reads the sender.
pub fn expr_reads_sender<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    expr: &Expr<'_>,
    seen: &mut HashSet<FunctionId>,
    aliases: &HashSet<VariableId>,
) -> bool {
    expr.visit(&mut |e| {
        let reads = is_sender_member(e)
            || underlying_var(e).is_some_and(|v| aliases.contains(&v))
            || matches!(&e.kind, ExprKind::Call(callee, ..)
                if function_ids(callee).any(|id| function_reads_sender(hir, id, seen)));
        if reads { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
    })
    .is_break()
}

/// Whether the body of `func_id` reads `msg.sender`/`tx.origin`, following calls.
pub fn function_reads_sender<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    seen.insert(func_id)
        && hir.function(func_id).body.is_some_and(|body| {
            visit_stmts(hir, body.stmts, |stmt| {
                let reads = stmt_expr(hir, stmt)
                    .is_some_and(|expr| expr_reads_sender(hir, expr, seen, &HashSet::new()));
                if reads { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
            })
            .is_break()
        })
}

/// State variables read by `expr`, following calls into user functions.
pub fn expr_state_vars<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    expr: &Expr<'_>,
    seen: &mut HashSet<FunctionId>,
    out: &mut HashSet<VariableId>,
) {
    let _ = expr.visit(&mut |e| {
        if let Some(var_id) = underlying_var(e)
            && hir.variable(var_id).kind.is_state()
        {
            out.insert(var_id);
        }
        if let ExprKind::Call(callee, ..) = &e.kind {
            for callee_id in function_ids(callee) {
                function_state_vars(hir, callee_id, seen, out);
            }
        }
        ControlFlow::<()>::Continue(())
    });
}

/// State variables read by the body of `func_id`, following calls into user functions.
pub fn function_state_vars<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
    out: &mut HashSet<VariableId>,
) {
    if seen.insert(func_id)
        && let Some(body) = hir.function(func_id).body
    {
        let _ = visit_stmts(hir, body.stmts, |stmt| {
            if let Some(expr) = stmt_expr(hir, stmt) {
                expr_state_vars(hir, expr, seen, out);
            }
            ControlFlow::Continue(())
        });
    }
}

fn expr_reads_state<'gcx>(hir: &'gcx hir::Hir<'gcx>, expr: &Expr<'_>) -> bool {
    let mut vars = HashSet::new();
    expr_state_vars(hir, expr, &mut HashSet::new(), &mut vars);
    !vars.is_empty()
}

/// An access check among the dominating statements of a function body.
enum Guard<'a> {
    /// The condition of a guarding `if` or of a `require`/`assert`.
    Check(&'a Expr<'a>),
    /// A call into a function that itself checks the caller.
    Call(FunctionId),
}

/// Calls `f` for every access check that dominates `body` (runs unconditionally before the `_`
/// placeholder) until it breaks. `seen` guards the recursion into called functions.
fn for_each_guard<'gcx>(
    hir: &'gcx hir::Hir<'gcx>,
    body: hir::Block<'gcx>,
    seen: &mut HashSet<FunctionId>,
    f: &mut impl FnMut(Guard<'_>) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let mut stmts = Vec::new();
    let _ = dominating_stmts(body.stmts, &mut stmts);
    let aliases = sender_aliases(hir, stmts.iter().copied());
    for stmt in stmts {
        if let StmtKind::If(cond, then_stmt, else_stmt) = stmt.kind {
            let exits = match access_check_polarity(hir, cond, &aliases) {
                Some(false) => branch_always_exits(then_stmt),
                Some(true) => else_stmt.is_some_and(branch_always_exits),
                None => false,
            };
            if exits {
                f(Guard::Check(cond))?;
            }
            continue;
        }
        let Some(expr) = stmt_expr(hir, stmt) else { continue };
        expr.visit(&mut |e| {
            match &e.kind {
                ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                    if let Some(cond) = args.exprs().next()
                        && access_check_polarity(hir, cond, &aliases) == Some(true)
                    {
                        f(Guard::Check(cond))?;
                    }
                }
                ExprKind::Call(callee, ..) => {
                    for callee_id in function_ids(callee) {
                        if looks_like_access_control(hir.function(callee_id))
                            || has_access_guard(hir, callee_id, seen)
                        {
                            f(Guard::Call(callee_id))?;
                        }
                    }
                }
                _ => {}
            }
            ControlFlow::Continue(())
        })?;
    }
    ControlFlow::Continue(())
}

/// Collects into `out` the statements that run unconditionally before the `_` placeholder (all of
/// them for functions), descending into blocks and loops. Breaks when the placeholder is reached.
fn dominating_stmts<'gcx>(
    stmts: impl IntoIterator<Item = &'gcx Stmt<'gcx>>,
    out: &mut Vec<&'gcx Stmt<'gcx>>,
) -> ControlFlow<()> {
    for stmt in stmts {
        match stmt.kind {
            StmtKind::Placeholder => return ControlFlow::Break(()),
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                dominating_stmts(block.stmts, out)?;
            }
            StmtKind::Loop(block, source) => dominating_stmts(loop_stmts(block, source), out)?,
            _ => out.push(stmt),
        }
    }
    ControlFlow::Continue(())
}
