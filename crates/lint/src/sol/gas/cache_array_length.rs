use super::CacheArrayLength;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::ElementaryType,
    interface::{kw, sym},
    sema::{
        Gcx,
        hir::{
            self, BinOpKind, ExprKind, ItemId, LoopSource, Res, StateMutability, StmtKind,
            VariableId,
        },
        ty::TyKind,
    },
};

declare_forge_lint!(
    CACHE_ARRAY_LENGTH,
    Severity::Gas,
    "cache-array-length",
    "array length read in loop condition should be cached outside the loop"
);

#[derive(Clone, Copy)]
struct LengthRead<'hir> {
    expr: &'hir hir::Expr<'hir>,
    base: &'hir hir::Expr<'hir>,
}

#[derive(Default)]
struct LoopFacts {
    written_vars: Vec<VariableId>,
    mutates_array_length: bool,
    has_state_mutating_call: bool,
}

impl LoopFacts {
    const fn should_skip(&self) -> bool {
        self.mutates_array_length || self.has_state_mutating_call
    }

    fn push_written_var(&mut self, var_id: VariableId) {
        if !self.written_vars.contains(&var_id) {
            self.written_vars.push(var_id);
        }
    }
}

impl<'hir> LateLintPass<'hir> for CacheArrayLength {
    fn check_stmt(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        stmt: &'hir hir::Stmt<'hir>,
    ) {
        let StmtKind::Loop(block, LoopSource::For) = &stmt.kind else { return };
        let Some((condition, body)) = for_loop_parts(*block) else { return };

        let mut reads = Vec::new();
        collect_condition_length_reads(gcx, condition, &mut reads);
        if reads.is_empty() {
            return;
        }

        let mut facts = LoopFacts::default();
        collect_stmt_facts(gcx, hir, body, &mut facts);
        if facts.should_skip() {
            return;
        }

        for read in reads {
            if expr_is_loop_invariant(gcx, hir, read.base, &facts.written_vars) {
                ctx.emit(&CACHE_ARRAY_LENGTH, read.expr.span);
            }
        }
    }
}

fn for_loop_parts<'hir>(
    block: hir::Block<'hir>,
) -> Option<(&'hir hir::Expr<'hir>, &'hir hir::Stmt<'hir>)> {
    let first = block.stmts.first()?;
    match &first.kind {
        StmtKind::If(condition, _, Some(else_stmt)) => {
            matches!(&else_stmt.kind, StmtKind::Break).then_some((*condition, first))
        }
        _ => None,
    }
}

fn collect_condition_length_reads<'hir>(
    gcx: Gcx<'hir>,
    expr: &'hir hir::Expr<'hir>,
    reads: &mut Vec<LengthRead<'hir>>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Binary(lhs, op, rhs) if is_comparison(op.kind) => {
            if matches!(lhs.peel_parens().kind, ExprKind::Ident(_)) {
                collect_state_array_length_read(gcx, rhs, reads);
            }
            if matches!(rhs.peel_parens().kind, ExprKind::Ident(_)) {
                collect_state_array_length_read(gcx, lhs, reads);
            }
        }
        _ => {}
    }
}

fn collect_state_array_length_read<'hir>(
    gcx: Gcx<'hir>,
    expr: &'hir hir::Expr<'hir>,
    reads: &mut Vec<LengthRead<'hir>>,
) {
    let expr = expr.peel_parens();
    if let ExprKind::Member(base, member) = &expr.kind
        && member.name == sym::length
        && is_state_array(gcx, base)
    {
        reads.push(LengthRead { expr, base });
    }
}

fn collect_stmt_facts<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
    facts: &mut LoopFacts,
) {
    match &stmt.kind {
        StmtKind::DeclSingle(var_id) => {
            if let Some(expr) = hir.variable(*var_id).initializer {
                collect_expr_facts(gcx, hir, expr, facts);
            }
        }
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr)
        | StmtKind::Expr(expr) => collect_expr_facts(gcx, hir, expr, facts),
        StmtKind::Return(expr) => {
            if let Some(expr) = expr {
                collect_expr_facts(gcx, hir, expr, facts);
            }
        }
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            for stmt in block.stmts {
                collect_stmt_facts(gcx, hir, stmt, facts);
            }
        }
        StmtKind::If(condition, then_stmt, else_stmt) => {
            collect_expr_facts(gcx, hir, condition, facts);
            collect_stmt_facts(gcx, hir, then_stmt, facts);
            if let Some(else_stmt) = else_stmt {
                collect_stmt_facts(gcx, hir, else_stmt, facts);
            }
        }
        StmtKind::Try(stmt_try) => {
            collect_expr_facts(gcx, hir, &stmt_try.expr, facts);
            for clause in stmt_try.clauses {
                for stmt in clause.block.stmts {
                    collect_stmt_facts(gcx, hir, stmt, facts);
                }
            }
        }
        StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::AssemblyBlock(_)
        | StmtKind::Switch(_)
        | StmtKind::Err(_) => {}
    }
}

fn collect_expr_facts<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    facts: &mut LoopFacts,
) {
    let expr = expr.peel_parens();
    if array_length_mutated(gcx, expr) {
        facts.mutates_array_length = true;
    }

    match &expr.kind {
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_expr_facts(gcx, hir, expr, facts);
            }
        }
        ExprKind::Assign(lhs, _, rhs) => {
            collect_written_vars(lhs, facts);
            collect_expr_facts(gcx, hir, lhs, facts);
            collect_expr_facts(gcx, hir, rhs, facts);
        }
        ExprKind::Binary(lhs, _, rhs) => {
            collect_expr_facts(gcx, hir, lhs, facts);
            collect_expr_facts(gcx, hir, rhs, facts);
        }
        ExprKind::Call(callee, args, named_args) => {
            if call_may_mutate_state(gcx, hir, callee) {
                facts.has_state_mutating_call = true;
            }
            collect_expr_facts(gcx, hir, callee, facts);
            for arg in args.exprs() {
                collect_expr_facts(gcx, hir, arg, facts);
            }
            if let Some(named_args) = named_args {
                for arg in named_args.args {
                    collect_expr_facts(gcx, hir, &arg.value, facts);
                }
            }
        }
        ExprKind::Delete(inner) => {
            collect_written_vars(inner, facts);
            collect_expr_facts(gcx, hir, inner, facts);
        }
        ExprKind::Payable(inner) => collect_expr_facts(gcx, hir, inner, facts),
        ExprKind::Unary(op, inner) => {
            if op.kind.has_side_effects() {
                collect_written_vars(inner, facts);
            }
            collect_expr_facts(gcx, hir, inner, facts);
        }
        ExprKind::Index(base, index) => {
            collect_expr_facts(gcx, hir, base, facts);
            if let Some(index) = index {
                collect_expr_facts(gcx, hir, index, facts);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_expr_facts(gcx, hir, base, facts);
            if let Some(start) = start {
                collect_expr_facts(gcx, hir, start, facts);
            }
            if let Some(end) = end {
                collect_expr_facts(gcx, hir, end, facts);
            }
        }
        ExprKind::Member(base, _) => collect_expr_facts(gcx, hir, base, facts),
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            collect_expr_facts(gcx, hir, condition, facts);
            collect_expr_facts(gcx, hir, then_expr, facts);
            collect_expr_facts(gcx, hir, else_expr, facts);
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_expr_facts(gcx, hir, expr, facts);
            }
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::YulMember(..)
        | ExprKind::Err(_) => {}
    }
}

fn collect_written_vars(expr: &hir::Expr<'_>, facts: &mut LoopFacts) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            if let Some(var_id) = variable_resolution(resolutions) {
                facts.push_written_var(var_id);
            }
        }
        ExprKind::Index(base, _) => {
            collect_written_vars(base, facts);
        }
        ExprKind::Slice(base, _, _) => {
            collect_written_vars(base, facts);
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) => collect_written_vars(base, facts),
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_written_vars(expr, facts);
            }
        }
        _ => {}
    }
}

fn array_length_mutated<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    match &expr.kind {
        ExprKind::Assign(lhs, _, _) | ExprKind::Delete(lhs) => is_array_like(gcx, lhs),
        ExprKind::Call(callee, _, _) => {
            let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return false };
            matches!(member.name, sym::push | kw::Pop) && is_array_like(gcx, base)
        }
        _ => false,
    }
}

fn call_may_mutate_state<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Type(_) => false,
        ExprKind::Ident(resolutions) => resolutions
            .iter()
            .find_map(|res| {
                if let Res::Item(ItemId::Function(function_id)) = res {
                    Some(hir.function(*function_id).mutates_state())
                } else {
                    None
                }
            })
            .unwrap_or(true),
        ExprKind::Member(base, member)
            if matches!(member.name, sym::push | kw::Pop) && is_array_like(gcx, base) =>
        {
            false
        }
        _ => match gcx.type_of_expr(callee.peel_parens().id).map(|ty| ty.peel_refs().kind) {
            Some(TyKind::Fn(function)) => function.state_mutability >= StateMutability::Payable,
            _ => true,
        },
    }
}

fn expr_is_loop_invariant<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    written_vars: &[VariableId],
) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            variable_resolution(resolutions).is_none_or(|var_id| !written_vars.contains(&var_id))
        }
        ExprKind::Lit(_) | ExprKind::Type(_) | ExprKind::TypeCall(_) => true,
        ExprKind::Array(exprs) => {
            exprs.iter().all(|expr| expr_is_loop_invariant(gcx, hir, expr, written_vars))
        }
        ExprKind::Binary(lhs, _, rhs) => {
            expr_is_loop_invariant(gcx, hir, lhs, written_vars)
                && expr_is_loop_invariant(gcx, hir, rhs, written_vars)
        }
        ExprKind::Call(callee, args, named_args) => {
            call_is_safe_to_cache(gcx, hir, callee)
                && expr_is_loop_invariant(gcx, hir, callee, written_vars)
                && args.exprs().all(|arg| expr_is_loop_invariant(gcx, hir, arg, written_vars))
                && named_args.is_none_or(|named_args| {
                    named_args
                        .args
                        .iter()
                        .all(|arg| expr_is_loop_invariant(gcx, hir, &arg.value, written_vars))
                })
        }
        ExprKind::Index(base, index) => {
            expr_is_loop_invariant(gcx, hir, base, written_vars)
                && index.is_none_or(|index| expr_is_loop_invariant(gcx, hir, index, written_vars))
        }
        ExprKind::Slice(base, start, end) => {
            expr_is_loop_invariant(gcx, hir, base, written_vars)
                && start.is_none_or(|start| expr_is_loop_invariant(gcx, hir, start, written_vars))
                && end.is_none_or(|end| expr_is_loop_invariant(gcx, hir, end, written_vars))
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) => {
            expr_is_loop_invariant(gcx, hir, base, written_vars)
        }
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            expr_is_loop_invariant(gcx, hir, condition, written_vars)
                && expr_is_loop_invariant(gcx, hir, then_expr, written_vars)
                && expr_is_loop_invariant(gcx, hir, else_expr, written_vars)
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().all(|expr| expr_is_loop_invariant(gcx, hir, expr, written_vars))
        }
        ExprKind::Unary(op, inner) => {
            !op.kind.has_side_effects() && expr_is_loop_invariant(gcx, hir, inner, written_vars)
        }
        ExprKind::Assign(_, _, _)
        | ExprKind::Delete(_)
        | ExprKind::New(_)
        | ExprKind::YulMember(..)
        | ExprKind::Err(_) => false,
    }
}

fn call_is_safe_to_cache<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Type(_) => true,
        ExprKind::Ident(resolutions) => resolutions
            .iter()
            .find_map(|res| {
                if let Res::Item(ItemId::Function(function_id)) = res {
                    Some(hir.function(*function_id).state_mutability <= StateMutability::View)
                } else {
                    None
                }
            })
            .unwrap_or(false),
        _ => match gcx.type_of_expr(callee.peel_parens().id).map(|ty| ty.peel_refs().kind) {
            Some(TyKind::Fn(function)) => function.state_mutability <= StateMutability::View,
            _ => false,
        },
    }
}

const fn is_comparison(op: BinOpKind) -> bool {
    matches!(
        op,
        BinOpKind::Lt
            | BinOpKind::Le
            | BinOpKind::Gt
            | BinOpKind::Ge
            | BinOpKind::Eq
            | BinOpKind::Ne
    )
}

fn is_array_like<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    let Some(ty) = gcx.type_of_expr(expr.peel_parens().id) else { return false };
    matches!(ty.peel_refs().kind, TyKind::DynArray(_) | TyKind::Elementary(ElementaryType::Bytes))
}

fn is_state_array<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    let ExprKind::Ident(resolutions) = &expr.peel_parens().kind else { return false };
    let Some(var_id) = variable_resolution(resolutions) else { return false };
    gcx.hir.variable(var_id).is_state_variable()
        && matches!(
            gcx.type_of_expr(expr.peel_parens().id).map(|ty| ty.peel_refs().kind),
            Some(TyKind::DynArray(_))
        )
}

fn variable_resolution(resolutions: &[Res]) -> Option<VariableId> {
    resolutions.iter().find_map(|res| {
        if let Res::Item(ItemId::Variable(var_id)) = res { Some(*var_id) } else { None }
    })
}
