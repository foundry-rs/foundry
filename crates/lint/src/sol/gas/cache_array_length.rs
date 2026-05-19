use super::CacheArrayLength;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::{Symbol, kw, sym},
    sema::hir::{
        self, BinOpKind, ElementaryType, ExprKind, ItemId, LoopSource, Res, StateMutability,
        StmtKind, TypeKind, UnOpKind, VariableId,
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
        hir: &'hir hir::Hir<'hir>,
        stmt: &'hir hir::Stmt<'hir>,
    ) {
        let StmtKind::Loop(block, LoopSource::For) = &stmt.kind else { return };
        let Some((condition, body)) = for_loop_parts(*block) else { return };

        let mut reads = Vec::new();
        collect_condition_length_reads(hir, condition, &mut reads);
        if reads.is_empty() {
            return;
        }

        let mut facts = LoopFacts::default();
        collect_stmt_facts(hir, body, &mut facts);
        if facts.should_skip() {
            return;
        }

        for read in reads {
            if expr_is_loop_invariant(hir, read.base, &facts.written_vars) {
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
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    reads: &mut Vec<LengthRead<'hir>>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Binary(lhs, op, rhs) if is_comparison(op.kind) => {
            collect_length_reads(hir, lhs, reads);
            collect_length_reads(hir, rhs, reads);
        }
        ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
            collect_condition_length_reads(hir, lhs, reads);
            collect_condition_length_reads(hir, rhs, reads);
        }
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            collect_condition_length_reads(hir, inner, reads);
        }
        _ => {}
    }
}

fn collect_length_reads<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    reads: &mut Vec<LengthRead<'hir>>,
) {
    let expr = expr.peel_parens();
    if let ExprKind::Member(base, member) = &expr.kind
        && member.name == sym::length
        && is_array_like(hir, base)
    {
        reads.push(LengthRead { expr, base });
        return;
    }

    match &expr.kind {
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_length_reads(hir, expr, reads);
            }
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            collect_length_reads(hir, lhs, reads);
            collect_length_reads(hir, rhs, reads);
        }
        ExprKind::Call(callee, args, named_args) => {
            collect_length_reads(hir, callee, reads);
            for arg in args.exprs() {
                collect_length_reads(hir, arg, reads);
            }
            if let Some(named_args) = named_args {
                for arg in *named_args {
                    collect_length_reads(hir, &arg.value, reads);
                }
            }
        }
        ExprKind::Delete(inner) | ExprKind::Payable(inner) | ExprKind::Unary(_, inner) => {
            collect_length_reads(hir, inner, reads);
        }
        ExprKind::Index(base, index) => {
            collect_length_reads(hir, base, reads);
            if let Some(index) = index {
                collect_length_reads(hir, index, reads);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_length_reads(hir, base, reads);
            if let Some(start) = start {
                collect_length_reads(hir, start, reads);
            }
            if let Some(end) = end {
                collect_length_reads(hir, end, reads);
            }
        }
        ExprKind::Member(base, _) => collect_length_reads(hir, base, reads),
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            collect_length_reads(hir, condition, reads);
            collect_length_reads(hir, then_expr, reads);
            collect_length_reads(hir, else_expr, reads);
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_length_reads(hir, expr, reads);
            }
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => {}
    }
}

fn collect_stmt_facts<'hir>(
    hir: &'hir hir::Hir<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
    facts: &mut LoopFacts,
) {
    match &stmt.kind {
        StmtKind::DeclSingle(var_id) => {
            if let Some(expr) = hir.variable(*var_id).initializer {
                collect_expr_facts(hir, expr, facts);
            }
        }
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr)
        | StmtKind::Expr(expr) => collect_expr_facts(hir, expr, facts),
        StmtKind::Return(expr) => {
            if let Some(expr) = expr {
                collect_expr_facts(hir, expr, facts);
            }
        }
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            for stmt in block.stmts {
                collect_stmt_facts(hir, stmt, facts);
            }
        }
        StmtKind::If(condition, then_stmt, else_stmt) => {
            collect_expr_facts(hir, condition, facts);
            collect_stmt_facts(hir, then_stmt, facts);
            if let Some(else_stmt) = else_stmt {
                collect_stmt_facts(hir, else_stmt, facts);
            }
        }
        StmtKind::Try(stmt_try) => {
            collect_expr_facts(hir, &stmt_try.expr, facts);
            for clause in stmt_try.clauses {
                for stmt in clause.block.stmts {
                    collect_stmt_facts(hir, stmt, facts);
                }
            }
        }
        StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Err(_) => {}
    }
}

fn collect_expr_facts<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    facts: &mut LoopFacts,
) {
    let expr = expr.peel_parens();
    if array_length_mutated(hir, expr) {
        facts.mutates_array_length = true;
    }

    match &expr.kind {
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_expr_facts(hir, expr, facts);
            }
        }
        ExprKind::Assign(lhs, _, rhs) => {
            collect_written_vars(lhs, facts);
            collect_expr_facts(hir, lhs, facts);
            collect_expr_facts(hir, rhs, facts);
        }
        ExprKind::Binary(lhs, _, rhs) => {
            collect_expr_facts(hir, lhs, facts);
            collect_expr_facts(hir, rhs, facts);
        }
        ExprKind::Call(callee, args, named_args) => {
            if call_may_mutate_state(hir, callee) {
                facts.has_state_mutating_call = true;
            }
            collect_expr_facts(hir, callee, facts);
            for arg in args.exprs() {
                collect_expr_facts(hir, arg, facts);
            }
            if let Some(named_args) = named_args {
                for arg in *named_args {
                    collect_expr_facts(hir, &arg.value, facts);
                }
            }
        }
        ExprKind::Delete(inner) => {
            collect_written_vars(inner, facts);
            collect_expr_facts(hir, inner, facts);
        }
        ExprKind::Payable(inner) => collect_expr_facts(hir, inner, facts),
        ExprKind::Unary(op, inner) => {
            if op.kind.has_side_effects() {
                collect_written_vars(inner, facts);
            }
            collect_expr_facts(hir, inner, facts);
        }
        ExprKind::Index(base, index) => {
            collect_expr_facts(hir, base, facts);
            if let Some(index) = index {
                collect_expr_facts(hir, index, facts);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_expr_facts(hir, base, facts);
            if let Some(start) = start {
                collect_expr_facts(hir, start, facts);
            }
            if let Some(end) = end {
                collect_expr_facts(hir, end, facts);
            }
        }
        ExprKind::Member(base, _) => collect_expr_facts(hir, base, facts),
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            collect_expr_facts(hir, condition, facts);
            collect_expr_facts(hir, then_expr, facts);
            collect_expr_facts(hir, else_expr, facts);
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_expr_facts(hir, expr, facts);
            }
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
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

fn array_length_mutated<'hir>(hir: &'hir hir::Hir<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    match &expr.kind {
        ExprKind::Assign(lhs, _, _) | ExprKind::Delete(lhs) => is_array_like(hir, lhs),
        ExprKind::Call(callee, _, _) => {
            let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return false };
            matches!(member.name, sym::push | kw::Pop) && is_array_like(hir, base)
        }
        _ => false,
    }
}

fn call_may_mutate_state(hir: &hir::Hir<'_>, callee: &hir::Expr<'_>) -> bool {
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
            if matches!(member.name, sym::push | kw::Pop) && is_array_like(hir, base) =>
        {
            false
        }
        _ => match &expr_type(hir, callee).map(|ty| &ty.kind) {
            Some(TypeKind::Function(function)) => {
                function.state_mutability >= StateMutability::Payable
            }
            _ => true,
        },
    }
}

fn expr_is_loop_invariant(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    written_vars: &[VariableId],
) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            variable_resolution(resolutions).is_none_or(|var_id| !written_vars.contains(&var_id))
        }
        ExprKind::Lit(_) | ExprKind::Type(_) | ExprKind::TypeCall(_) => true,
        ExprKind::Array(exprs) => {
            exprs.iter().all(|expr| expr_is_loop_invariant(hir, expr, written_vars))
        }
        ExprKind::Binary(lhs, _, rhs) => {
            expr_is_loop_invariant(hir, lhs, written_vars)
                && expr_is_loop_invariant(hir, rhs, written_vars)
        }
        ExprKind::Call(callee, args, named_args) => {
            call_is_safe_to_cache(hir, callee)
                && expr_is_loop_invariant(hir, callee, written_vars)
                && args.exprs().all(|arg| expr_is_loop_invariant(hir, arg, written_vars))
                && named_args.is_none_or(|named_args| {
                    named_args
                        .iter()
                        .all(|arg| expr_is_loop_invariant(hir, &arg.value, written_vars))
                })
        }
        ExprKind::Index(base, index) => {
            expr_is_loop_invariant(hir, base, written_vars)
                && index.is_none_or(|index| expr_is_loop_invariant(hir, index, written_vars))
        }
        ExprKind::Slice(base, start, end) => {
            expr_is_loop_invariant(hir, base, written_vars)
                && start.is_none_or(|start| expr_is_loop_invariant(hir, start, written_vars))
                && end.is_none_or(|end| expr_is_loop_invariant(hir, end, written_vars))
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) => {
            expr_is_loop_invariant(hir, base, written_vars)
        }
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            expr_is_loop_invariant(hir, condition, written_vars)
                && expr_is_loop_invariant(hir, then_expr, written_vars)
                && expr_is_loop_invariant(hir, else_expr, written_vars)
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().all(|expr| expr_is_loop_invariant(hir, expr, written_vars))
        }
        ExprKind::Unary(op, inner) => {
            !op.kind.has_side_effects() && expr_is_loop_invariant(hir, inner, written_vars)
        }
        ExprKind::Assign(_, _, _) | ExprKind::Delete(_) | ExprKind::New(_) | ExprKind::Err(_) => {
            false
        }
    }
}

fn call_is_safe_to_cache(hir: &hir::Hir<'_>, callee: &hir::Expr<'_>) -> bool {
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
        _ => match &expr_type(hir, callee).map(|ty| &ty.kind) {
            Some(TypeKind::Function(function)) => {
                function.state_mutability <= StateMutability::View
            }
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

fn is_array_like(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let Some(ty) = expr_type(hir, expr) else { return false };
    match &ty.kind {
        TypeKind::Array(array) => array.size.is_none(),
        TypeKind::Elementary(ElementaryType::Bytes) => true,
        _ => false,
    }
}

fn expr_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            let var_id = variable_resolution(resolutions)?;
            Some(&hir.variable(var_id).ty)
        }
        ExprKind::Index(base, _) => match &expr_type(hir, base)?.kind {
            TypeKind::Array(array) => Some(&array.element),
            TypeKind::Mapping(mapping) => Some(&mapping.value),
            _ => None,
        },
        ExprKind::Member(base, member) => {
            struct_field_type(hir, expr_type(hir, base)?, member.name)
        }
        ExprKind::Call(callee, _, _) => call_return_type(hir, callee),
        _ => None,
    }
}

fn call_return_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    match &callee.peel_parens().kind {
        ExprKind::Type(ty) => Some(ty),
        ExprKind::Ident(resolutions) => {
            let function_id = resolutions.iter().find_map(|res| {
                if let Res::Item(ItemId::Function(function_id)) = res {
                    Some(*function_id)
                } else {
                    None
                }
            })?;
            function_return_type(hir, function_id)
        }
        _ => match &expr_type(hir, callee)?.kind {
            TypeKind::Function(function) => {
                let [return_id] = function.returns else { return None };
                Some(&hir.variable(*return_id).ty)
            }
            _ => None,
        },
    }
}

fn function_return_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    function_id: hir::FunctionId,
) -> Option<&'hir hir::Type<'hir>> {
    let [return_id] = hir.function(function_id).returns else { return None };
    Some(&hir.variable(*return_id).ty)
}

fn struct_field_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    ty: &'hir hir::Type<'hir>,
    member: Symbol,
) -> Option<&'hir hir::Type<'hir>> {
    let TypeKind::Custom(ItemId::Struct(struct_id)) = &ty.kind else { return None };
    hir.strukt(*struct_id)
        .fields
        .iter()
        .map(|&field_id| hir.variable(field_id))
        .find(|field| field.name.is_some_and(|name| name.name == member))
        .map(|field| &field.ty)
}

fn variable_resolution(resolutions: &[Res]) -> Option<VariableId> {
    resolutions.iter().find_map(|res| {
        if let Res::Item(ItemId::Variable(var_id)) = res { Some(*var_id) } else { None }
    })
}
