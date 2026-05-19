use super::UninitializedStateVariables;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::ContractKind,
    interface::data_structures::Never,
    sema::{
        Hir,
        hir::{
            Block, ContractId, DataLocation, Expr, ExprKind, ItemId, Res, Stmt, StmtKind, TypeKind,
            VariableId, Visit,
        },
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    UNINITIALIZED_STATE_VARIABLES,
    Severity::Med,
    "uninitialized-state",
    "state variable is read but never written"
);

impl<'hir> LateLintPass<'hir> for UninitializedStateVariables {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir Hir<'hir>,
        contract_id: ContractId,
    ) {
        let contract = hir.contract(contract_id);

        if matches!(contract.kind, ContractKind::Interface | ContractKind::AbstractContract) {
            return;
        }

        // Only analyse the most-derived contract in each hierarchy so each
        // state variable is reported exactly once and write/read information
        // from every level of the inheritance chain is visible.
        if hir.contracts().any(|c| c.linearized_bases.iter().skip(1).any(|&id| id == contract_id)) {
            return;
        }

        // If C3 linearization failed the linearized_bases list is incomplete;
        // skip rather than produce unsound results.
        if contract.linearization_failed() {
            return;
        }

        // Collect non-constant, non-immutable state variables from the whole
        // inheritance chain (linearized_bases[0] is the contract itself).
        let state_vars: Vec<VariableId> = contract
            .linearized_bases
            .iter()
            .flat_map(|&cid| hir.contract(cid).variables())
            .filter(|&var_id| {
                let var = hir.variable(var_id);
                !var.is_constant()
                    && !var.is_immutable()
                    && !matches!(var.ty.kind, TypeKind::Mapping(_))
            })
            .collect();

        if state_vars.is_empty() {
            return;
        }

        let candidate_set: HashSet<VariableId> = state_vars.iter().copied().collect();

        let mut written: HashSet<VariableId> = HashSet::new();

        for &var_id in &state_vars {
            if hir.variable(var_id).initializer.is_some() {
                written.insert(var_id);
            }
        }

        // Walk every function in the inheritance chain.
        // Bail out conservatively if any function body contains inline assembly
        // (lowered to StmtKind::Err by Solar), because we cannot soundly track
        // reads or writes through it.
        for &cid in contract.linearized_bases {
            for func_id in hir.contract(cid).all_functions() {
                let function = hir.function(func_id);

                for modifier in function.modifiers {
                    for expr in modifier.args.exprs() {
                        if collect_expr_writes_checked(hir, expr, &candidate_set, &mut written)
                            .is_err()
                        {
                            return;
                        }
                    }
                }

                if let Some(body) = function.body
                    && collect_block_writes_checked(hir, body, &candidate_set, &mut written)
                        .is_err()
                {
                    return;
                }
            }

            for base_modifier in hir.contract(cid).bases_args {
                for expr in base_modifier.args.exprs() {
                    if collect_expr_writes_checked(hir, expr, &candidate_set, &mut written).is_err()
                    {
                        return;
                    }
                }
            }
        }

        let mut reader = ReadVarCollector { hir, read: HashSet::new() };
        for &cid in contract.linearized_bases {
            for func_id in hir.contract(cid).all_functions() {
                let _ = reader.visit_nested_function(func_id);
            }
            for var_id in hir.contract(cid).variables() {
                let _ = reader.visit_nested_var(var_id);
            }
            // Walk inheritance-specifier constructor args on the read side too
            // (e.g. `contract B is A(owner)` reads `owner`).
            for base_modifier in hir.contract(cid).bases_args {
                let _ = reader.visit_modifier(base_modifier);
            }
        }

        // Flag variables that are read but never written.
        for var_id in state_vars {
            if reader.read.contains(&var_id) && !written.contains(&var_id) {
                let var = hir.variable(var_id);
                ctx.emit(&UNINITIALIZED_STATE_VARIABLES, var.span);
            }
        }
    }
}

fn collect_block_writes_checked<'hir>(
    hir: &'hir Hir<'hir>,
    block: Block<'hir>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
) -> Result<(), ()> {
    for stmt in block.stmts {
        collect_stmt_writes_checked(hir, stmt, candidates, writes)?;
    }
    Ok(())
}

fn collect_stmt_writes_checked<'hir>(
    hir: &'hir Hir<'hir>,
    stmt: &'hir Stmt<'hir>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
) -> Result<(), ()> {
    match &stmt.kind {
        // Assembly is lowered to StmtKind::Err; bail conservatively.
        StmtKind::Err(_) => return Err(()),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            collect_block_writes_checked(hir, *block, candidates, writes)?;
        }
        StmtKind::If(condition, then_stmt, else_stmt) => {
            collect_expr_writes_checked(hir, condition, candidates, writes)?;
            collect_stmt_writes_checked(hir, then_stmt, candidates, writes)?;
            if let Some(else_stmt) = else_stmt {
                collect_stmt_writes_checked(hir, else_stmt, candidates, writes)?;
            }
        }
        StmtKind::Try(stmt_try) => {
            collect_expr_writes_checked(hir, &stmt_try.expr, candidates, writes)?;
            for clause in stmt_try.clauses {
                collect_block_writes_checked(hir, clause.block, candidates, writes)?;
            }
        }
        StmtKind::DeclSingle(var_id) => {
            if let Some(initializer) = hir.variable(*var_id).initializer {
                collect_expr_writes_checked(hir, initializer, candidates, writes)?;
            }
        }
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr)
        | StmtKind::Return(Some(expr))
        | StmtKind::Expr(expr) => collect_expr_writes_checked(hir, expr, candidates, writes)?,
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder => {}
    }
    Ok(())
}

fn collect_expr_writes_checked<'hir>(
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
) -> Result<(), ()> {
    match &expr.kind {
        ExprKind::Assign(lhs, _, rhs) => {
            collect_lvalue_writes(lhs, candidates, writes);
            collect_expr_writes_checked(hir, lhs, candidates, writes)?;
            collect_expr_writes_checked(hir, rhs, candidates, writes)?;
        }
        ExprKind::Delete(inner) => {
            collect_lvalue_writes(inner, candidates, writes);
            collect_expr_writes_checked(hir, inner, candidates, writes)?;
        }
        ExprKind::Unary(op, inner) => {
            if op.kind.has_side_effects() {
                collect_lvalue_writes(inner, candidates, writes);
            }
            collect_expr_writes_checked(hir, inner, candidates, writes)?;
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_expr_writes_checked(hir, expr, candidates, writes)?;
            }
        }
        ExprKind::Binary(lhs, _, rhs) => {
            collect_expr_writes_checked(hir, lhs, candidates, writes)?;
            collect_expr_writes_checked(hir, rhs, candidates, writes)?;
        }
        ExprKind::Call(callee, args, named_args) => {
            if let ExprKind::Member(base, _) = &callee.kind {
                // Covers push/pop and library dispatch (`using Lib for T` with `T storage self`);
                // can't resolve callee without Gcx. Treat the receiver as a write target to avoid
                // false positives.
                collect_lvalue_writes(base, candidates, writes);
            }

            // Direct calls to internal functions that take a `storage` parameter
            // mutate the corresponding argument in place; treat it as a write.
            if let ExprKind::Ident(resolutions) = &callee.kind {
                for res in *resolutions {
                    if let Res::Item(ItemId::Function(func_id)) = res {
                        let func = hir.function(*func_id);
                        for (&param_id, arg_expr) in func.parameters.iter().zip(args.exprs()) {
                            if matches!(
                                hir.variable(param_id).data_location,
                                Some(DataLocation::Storage)
                            ) {
                                collect_lvalue_writes(arg_expr, candidates, writes);
                            }
                        }
                    }
                }
            }
            collect_expr_writes_checked(hir, callee, candidates, writes)?;
            for expr in args.exprs() {
                collect_expr_writes_checked(hir, expr, candidates, writes)?;
            }
            if let Some(named_args) = named_args {
                for arg in *named_args {
                    collect_expr_writes_checked(hir, &arg.value, candidates, writes)?;
                }
            }
        }
        ExprKind::Index(base, index) => {
            collect_expr_writes_checked(hir, base, candidates, writes)?;
            if let Some(index) = index {
                collect_expr_writes_checked(hir, index, candidates, writes)?;
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_expr_writes_checked(hir, base, candidates, writes)?;
            if let Some(start) = start {
                collect_expr_writes_checked(hir, start, candidates, writes)?;
            }
            if let Some(end) = end {
                collect_expr_writes_checked(hir, end, candidates, writes)?;
            }
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) => {
            collect_expr_writes_checked(hir, base, candidates, writes)?;
        }
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            collect_expr_writes_checked(hir, condition, candidates, writes)?;
            collect_expr_writes_checked(hir, then_expr, candidates, writes)?;
            collect_expr_writes_checked(hir, else_expr, candidates, writes)?;
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_expr_writes_checked(hir, expr, candidates, writes)?;
            }
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => {}
    }
    Ok(())
}

fn collect_lvalue_writes(
    expr: &Expr<'_>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) if candidates.contains(id) => {
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

struct ReadVarCollector<'hir> {
    hir: &'hir Hir<'hir>,
    read: HashSet<VariableId>,
}

impl<'hir> Visit<'hir> for ReadVarCollector<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Ident(resolutions) = &expr.kind {
            for res in *resolutions {
                if let Res::Item(ItemId::Variable(var_id)) = res {
                    self.read.insert(*var_id);
                }
            }
        }
        self.walk_expr(expr)
    }
}
