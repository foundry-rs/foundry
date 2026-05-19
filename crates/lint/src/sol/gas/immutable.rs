use super::CouldBeImmutable;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{self, UnOpKind},
    interface::{kw, sym},
    sema::hir::{self, ExprKind, Res, StmtKind, TypeKind},
};
use std::collections::HashSet;

declare_forge_lint!(
    COULD_BE_IMMUTABLE,
    Severity::Gas,
    "could-be-immutable",
    "state variable could be declared immutable"
);

impl<'hir> LateLintPass<'hir> for CouldBeImmutable {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        contract_id: hir::ContractId,
    ) {
        let contract = hir.contract(contract_id);
        if contract.kind == ast::ContractKind::Interface {
            return;
        }
        if !is_most_derived_contract(hir, contract_id) {
            return;
        }

        let candidates: Vec<_> = contract
            .linearized_bases
            .iter()
            .flat_map(|&contract_id| hir.contract(contract_id).variables())
            .filter(|&id| is_immutable_candidate_type(hir.variable(id)))
            .collect();

        if candidates.is_empty() {
            return;
        }
        let candidate_set: HashSet<_> = candidates.iter().copied().collect();

        if contract_contains_unlowered_stmt(hir, contract) {
            return;
        }

        let mut constructor_writes = HashSet::new();
        let mut runtime_writes = HashSet::new();

        for &var_id in &candidates {
            let var = hir.variable(var_id);
            if var.initializer.is_some_and(|expr| !is_compile_time_constant(hir, expr)) {
                constructor_writes.insert(var_id);
            }
        }

        for &contract_id in contract.linearized_bases {
            for function_id in hir.contract(contract_id).all_functions() {
                let function = hir.function(function_id);
                if function.is_constructor() {
                    collect_modifier_writes(
                        hir,
                        function,
                        &candidate_set,
                        &mut constructor_writes,
                        &mut runtime_writes,
                        &mut HashSet::new(),
                    );

                    if let Some(body) = function.body {
                        collect_state_writes(hir, body, &candidate_set, &mut constructor_writes);
                    }
                } else {
                    // Immutable variables can only be assigned inline or directly in constructor
                    // bodies, so writes hidden behind internal helpers are not valid candidates.
                    let mut modifier_argument_writes = HashSet::new();
                    collect_modifier_writes(
                        hir,
                        function,
                        &candidate_set,
                        &mut modifier_argument_writes,
                        &mut runtime_writes,
                        &mut HashSet::new(),
                    );
                    runtime_writes.extend(modifier_argument_writes);

                    if let Some(body) = function.body {
                        collect_state_writes(hir, body, &candidate_set, &mut runtime_writes);
                    }
                }
            }
        }

        for &var_id in &candidates {
            if constructor_writes.contains(&var_id) && !runtime_writes.contains(&var_id) {
                let var = hir.variable(var_id);
                ctx.emit(&COULD_BE_IMMUTABLE, var.name.map_or(var.span, |name| name.span));
            }
        }
    }
}

fn is_most_derived_contract(hir: &hir::Hir<'_>, contract_id: hir::ContractId) -> bool {
    !hir.contracts()
        .any(|contract| contract.linearized_bases.iter().skip(1).any(|&id| id == contract_id))
}

fn collect_modifier_writes<'hir>(
    hir: &'hir hir::Hir<'hir>,
    function: &'hir hir::Function<'hir>,
    candidates: &HashSet<hir::VariableId>,
    argument_writes: &mut HashSet<hir::VariableId>,
    body_writes: &mut HashSet<hir::VariableId>,
    visited_modifiers: &mut HashSet<hir::FunctionId>,
) {
    for modifier in function.modifiers {
        for expr in modifier.args.exprs() {
            collect_expr_writes(expr, candidates, argument_writes);
        }

        let Some(modifier_id) = modifier.id.as_function() else { continue };
        if !visited_modifiers.insert(modifier_id) {
            continue;
        }

        let modifier = hir.function(modifier_id);
        let mut nested_argument_writes = HashSet::new();
        collect_modifier_writes(
            hir,
            modifier,
            candidates,
            &mut nested_argument_writes,
            body_writes,
            visited_modifiers,
        );
        body_writes.extend(nested_argument_writes);
        if let Some(body) = modifier.body {
            collect_state_writes(hir, body, candidates, body_writes);
        }
    }
}

fn is_immutable_candidate_type(var: &hir::Variable<'_>) -> bool {
    var.is_state_variable()
        && var.mutability.is_none()
        && match var.ty.kind {
            TypeKind::Elementary(ty) => ty.is_value_type(),
            TypeKind::Custom(hir::ItemId::Contract(_)) => true,
            _ => false,
        }
}

fn contract_contains_unlowered_stmt<'hir>(
    hir: &'hir hir::Hir<'hir>,
    contract: &'hir hir::Contract<'hir>,
) -> bool {
    contract.linearized_bases.iter().any(|&contract_id| {
        hir.contract(contract_id).all_functions().any(|function_id| {
            hir.function(function_id).body.is_some_and(|body| block_contains_unlowered_stmt(body))
        })
    })
}

fn block_contains_unlowered_stmt(block: hir::Block<'_>) -> bool {
    block.stmts.iter().any(stmt_contains_unlowered_stmt)
}

fn stmt_contains_unlowered_stmt(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Err(_) => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            block_contains_unlowered_stmt(*block)
        }
        StmtKind::If(_, then_stmt, else_stmt) => {
            stmt_contains_unlowered_stmt(then_stmt)
                || else_stmt.is_some_and(stmt_contains_unlowered_stmt)
        }
        StmtKind::Try(stmt_try) => {
            stmt_try.clauses.iter().any(|clause| block_contains_unlowered_stmt(clause.block))
        }
        StmtKind::DeclSingle(_)
        | StmtKind::DeclMulti(_, _)
        | StmtKind::Emit(_)
        | StmtKind::Revert(_)
        | StmtKind::Return(_)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Expr(_)
        | StmtKind::Placeholder => false,
    }
}

fn collect_state_writes<'hir>(
    hir: &'hir hir::Hir<'hir>,
    block: hir::Block<'hir>,
    candidates: &HashSet<hir::VariableId>,
    writes: &mut HashSet<hir::VariableId>,
) {
    for stmt in block.stmts {
        collect_stmt_writes(hir, stmt, candidates, writes);
    }
}

fn collect_stmt_writes<'hir>(
    hir: &'hir hir::Hir<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
    candidates: &HashSet<hir::VariableId>,
    writes: &mut HashSet<hir::VariableId>,
) {
    match &stmt.kind {
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            collect_state_writes(hir, *block, candidates, writes);
        }
        StmtKind::If(condition, then_stmt, else_stmt) => {
            collect_expr_writes(condition, candidates, writes);
            collect_stmt_writes(hir, then_stmt, candidates, writes);
            if let Some(else_stmt) = else_stmt {
                collect_stmt_writes(hir, else_stmt, candidates, writes);
            }
        }
        StmtKind::Try(stmt_try) => {
            collect_expr_writes(&stmt_try.expr, candidates, writes);
            for clause in stmt_try.clauses {
                collect_state_writes(hir, clause.block, candidates, writes);
            }
        }
        StmtKind::DeclSingle(var_id) => {
            if let Some(initializer) = hir.variable(*var_id).initializer {
                collect_expr_writes(initializer, candidates, writes);
            }
        }
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr)
        | StmtKind::Return(Some(expr))
        | StmtKind::Expr(expr) => collect_expr_writes(expr, candidates, writes),
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::Err(_) => {}
    }
}

fn collect_expr_writes<'hir>(
    expr: &'hir hir::Expr<'hir>,
    candidates: &HashSet<hir::VariableId>,
    writes: &mut HashSet<hir::VariableId>,
) {
    match &expr.kind {
        ExprKind::Assign(lhs, _, rhs) => {
            collect_lvalue_writes(lhs, candidates, writes);
            collect_expr_writes(lhs, candidates, writes);
            collect_expr_writes(rhs, candidates, writes);
        }
        ExprKind::Delete(inner) => {
            collect_lvalue_writes(inner, candidates, writes);
            collect_expr_writes(inner, candidates, writes);
        }
        ExprKind::Unary(op, inner) => {
            if op.kind.has_side_effects() {
                collect_lvalue_writes(inner, candidates, writes);
            }
            collect_expr_writes(inner, candidates, writes);
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_expr_writes(expr, candidates, writes);
            }
        }
        ExprKind::Binary(lhs, _, rhs) => {
            collect_expr_writes(lhs, candidates, writes);
            collect_expr_writes(rhs, candidates, writes);
        }
        ExprKind::Call(callee, args, named_args) => {
            collect_expr_writes(callee, candidates, writes);
            for expr in args.exprs() {
                collect_expr_writes(expr, candidates, writes);
            }
            if let Some(named_args) = named_args {
                for arg in *named_args {
                    collect_expr_writes(&arg.value, candidates, writes);
                }
            }
        }
        ExprKind::Index(base, index) => {
            collect_expr_writes(base, candidates, writes);
            if let Some(index) = index {
                collect_expr_writes(index, candidates, writes);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_expr_writes(base, candidates, writes);
            if let Some(start) = start {
                collect_expr_writes(start, candidates, writes);
            }
            if let Some(end) = end {
                collect_expr_writes(end, candidates, writes);
            }
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) => {
            collect_expr_writes(base, candidates, writes);
        }
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            collect_expr_writes(condition, candidates, writes);
            collect_expr_writes(then_expr, candidates, writes);
            collect_expr_writes(else_expr, candidates, writes);
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_expr_writes(expr, candidates, writes);
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

fn collect_lvalue_writes(
    expr: &hir::Expr<'_>,
    candidates: &HashSet<hir::VariableId>,
    writes: &mut HashSet<hir::VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(hir::ItemId::Variable(id)), ..]) if candidates.contains(id) => {
            writes.insert(*id);
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_lvalue_writes(expr, candidates, writes);
            }
        }
        ExprKind::Index(base, _)
        | ExprKind::Slice(base, _, _)
        | ExprKind::Member(base, _)
        | ExprKind::Payable(base) => collect_lvalue_writes(base, candidates, writes),
        _ => {}
    }
}

fn is_compile_time_constant(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    match &expr.kind {
        ExprKind::Lit(_) | ExprKind::Type(_) | ExprKind::TypeCall(_) => true,
        ExprKind::Ident(resolutions) => resolutions.iter().all(|res| match res {
            Res::Item(hir::ItemId::Variable(var_id)) => hir.variable(*var_id).is_constant(),
            _ => false,
        }),
        ExprKind::Unary(op, inner) => {
            !matches!(
                op.kind,
                UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
            ) && is_compile_time_constant(hir, inner)
        }
        ExprKind::Binary(lhs, _, rhs) => {
            is_compile_time_constant(hir, lhs) && is_compile_time_constant(hir, rhs)
        }
        ExprKind::Call(callee, args, named_args) => {
            is_allowed_constant_call(callee)
                && args.exprs().all(|expr| is_compile_time_constant(hir, expr))
                && named_args.is_none_or(|args| {
                    args.iter().all(|arg| is_compile_time_constant(hir, &arg.value))
                })
        }
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            is_compile_time_constant(hir, condition)
                && is_compile_time_constant(hir, then_expr)
                && is_compile_time_constant(hir, else_expr)
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().all(|expr| is_compile_time_constant(hir, expr))
        }
        ExprKind::Array(_)
        | ExprKind::Assign(_, _, _)
        | ExprKind::Delete(_)
        | ExprKind::Index(_, _)
        | ExprKind::Slice(_, _, _)
        | ExprKind::Member(_, _)
        | ExprKind::New(_)
        | ExprKind::Payable(_)
        | ExprKind::Err(_) => false,
    }
}

fn is_allowed_constant_call(callee: &hir::Expr<'_>) -> bool {
    match &callee.kind {
        ExprKind::Type(_) => true,
        ExprKind::Ident([Res::Builtin(builtin), ..]) => {
            let name = builtin.name();
            name == kw::Keccak256
                || name == kw::Addmod
                || name == kw::Mulmod
                || name == sym::sha256
                || name == sym::ripemd160
                || name == sym::ecrecover
        }
        _ => false,
    }
}
