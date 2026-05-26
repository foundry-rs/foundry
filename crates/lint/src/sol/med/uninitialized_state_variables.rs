use super::UninitializedStateVariables;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::ContractKind,
    interface::{data_structures::Never, sym},
    sema::{
        Hir,
        hir::{
            Block, CallArgs, CallArgsKind, ContractId, DataLocation, Expr, ExprKind, Function,
            ItemId, Res, Stmt, StmtKind, TypeKind, VariableId, Visit,
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
        let bases = contract.linearized_bases;

        for &cid in bases {
            for func_id in hir.contract(cid).all_functions() {
                let function = hir.function(func_id);

                for modifier in function.modifiers {
                    for expr in modifier.args.exprs() {
                        if collect_expr_writes_checked(
                            hir,
                            expr,
                            &candidate_set,
                            &mut written,
                            bases,
                        )
                        .is_err()
                        {
                            return;
                        }
                    }
                }

                if let Some(body) = function.body
                    && collect_block_writes_checked(hir, body, &candidate_set, &mut written, bases)
                        .is_err()
                {
                    return;
                }
            }

            for base_modifier in hir.contract(cid).bases_args {
                for expr in base_modifier.args.exprs() {
                    if collect_expr_writes_checked(hir, expr, &candidate_set, &mut written, bases)
                        .is_err()
                    {
                        return;
                    }
                }
            }

            // Walk state-vars initializer expressions for side-effect writes to other state vars
            for var_id in hir.contract(cid).variables() {
                if let Some(init) = hir.variable(var_id).initializer
                    && collect_expr_writes_checked(hir, init, &candidate_set, &mut written, bases)
                        .is_err()
                {
                    return;
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
    bases: &'hir [ContractId],
) -> Result<(), ()> {
    for stmt in block.stmts {
        collect_stmt_writes_checked(hir, stmt, candidates, writes, bases)?;
    }
    Ok(())
}

fn collect_stmt_writes_checked<'hir>(
    hir: &'hir Hir<'hir>,
    stmt: &'hir Stmt<'hir>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
    bases: &'hir [ContractId],
) -> Result<(), ()> {
    match &stmt.kind {
        // Assembly is lowered to StmtKind::Err; bail conservatively.
        StmtKind::Err(_) => return Err(()),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            collect_block_writes_checked(hir, *block, candidates, writes, bases)?;
        }
        StmtKind::If(condition, then_stmt, else_stmt) => {
            collect_expr_writes_checked(hir, condition, candidates, writes, bases)?;
            collect_stmt_writes_checked(hir, then_stmt, candidates, writes, bases)?;
            if let Some(else_stmt) = else_stmt {
                collect_stmt_writes_checked(hir, else_stmt, candidates, writes, bases)?;
            }
        }
        StmtKind::Try(stmt_try) => {
            collect_expr_writes_checked(hir, &stmt_try.expr, candidates, writes, bases)?;
            for clause in stmt_try.clauses {
                collect_block_writes_checked(hir, clause.block, candidates, writes, bases)?;
            }
        }
        StmtKind::DeclSingle(var_id) => {
            if let Some(initializer) = hir.variable(*var_id).initializer {
                collect_expr_writes_checked(hir, initializer, candidates, writes, bases)?;
            }
        }
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr)
        | StmtKind::Return(Some(expr))
        | StmtKind::Expr(expr) => {
            collect_expr_writes_checked(hir, expr, candidates, writes, bases)?
        }
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder => {}
    }
    Ok(())
}

fn collect_expr_writes_checked<'hir>(
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
    bases: &'hir [ContractId],
) -> Result<(), ()> {
    match &expr.kind {
        ExprKind::Assign(lhs, _, rhs) => {
            collect_lvalue_writes(lhs, candidates, writes);
            collect_expr_writes_checked(hir, lhs, candidates, writes, bases)?;
            collect_expr_writes_checked(hir, rhs, candidates, writes, bases)?;
        }
        ExprKind::Delete(inner) => {
            collect_lvalue_writes(inner, candidates, writes);
            collect_expr_writes_checked(hir, inner, candidates, writes, bases)?;
        }
        ExprKind::Unary(op, inner) => {
            if op.kind.has_side_effects() {
                collect_lvalue_writes(inner, candidates, writes);
            }
            collect_expr_writes_checked(hir, inner, candidates, writes, bases)?;
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_expr_writes_checked(hir, expr, candidates, writes, bases)?;
            }
        }
        ExprKind::Binary(lhs, _, rhs) => {
            collect_expr_writes_checked(hir, lhs, candidates, writes, bases)?;
            collect_expr_writes_checked(hir, rhs, candidates, writes, bases)?;
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
            //
            // Handles bare identifier callees (`_set(slot, v)`) and qualified member
            // callees (`BaseSetter._set(slot, v)`, `super._set(slot, v)`).
            let funcs = collect_callee_funcs(hir, callee, bases);
            if !funcs.is_empty() {
                mark_storage_args(&funcs, hir, args, candidates, writes);
            }

            collect_expr_writes_checked(hir, callee, candidates, writes, bases)?;
            for expr in args.exprs() {
                collect_expr_writes_checked(hir, expr, candidates, writes, bases)?;
            }
            if let Some(named_args) = named_args {
                for arg in *named_args {
                    collect_expr_writes_checked(hir, &arg.value, candidates, writes, bases)?;
                }
            }
        }
        ExprKind::Index(base, index) => {
            collect_expr_writes_checked(hir, base, candidates, writes, bases)?;
            if let Some(index) = index {
                collect_expr_writes_checked(hir, index, candidates, writes, bases)?;
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_expr_writes_checked(hir, base, candidates, writes, bases)?;
            if let Some(start) = start {
                collect_expr_writes_checked(hir, start, candidates, writes, bases)?;
            }
            if let Some(end) = end {
                collect_expr_writes_checked(hir, end, candidates, writes, bases)?;
            }
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) => {
            collect_expr_writes_checked(hir, base, candidates, writes, bases)?;
        }
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            collect_expr_writes_checked(hir, condition, candidates, writes, bases)?;
            collect_expr_writes_checked(hir, then_expr, candidates, writes, bases)?;
            collect_expr_writes_checked(hir, else_expr, candidates, writes, bases)?;
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_expr_writes_checked(hir, expr, candidates, writes, bases)?;
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

/// Collect the set of internal function candidates that a call expression may invoke.
///
/// Handles three callee shapes:
/// - `f(...)` bare `Ident` with function resolutions
/// - `Contract.f(...)` `Member` whose base resolves to a `ContractId`
/// - `super.f(...)` `Member` whose base is the `super` builtin; searches all linearized bases
///   except the current contract (`bases[0]`), matching Solidity's MRO dispatch semantics
fn collect_callee_funcs<'hir>(
    hir: &'hir Hir<'hir>,
    callee: &'hir Expr<'hir>,
    bases: &[ContractId],
) -> Vec<&'hir Function<'hir>> {
    match &callee.kind {
        ExprKind::Ident(resolutions) => resolutions
            .iter()
            .filter_map(|res| {
                if let Res::Item(ItemId::Function(func_id)) = res {
                    Some(hir.function(*func_id))
                } else {
                    None
                }
            })
            .collect(),
        ExprKind::Member(base, method) => {
            if let ExprKind::Ident(resolutions) = &base.peel_parens().kind {
                let is_super = resolutions
                    .iter()
                    .any(|r| matches!(r, Res::Builtin(b) if b.name() == sym::super_));

                let contract_ids: Vec<ContractId> = if is_super {
                    // `super.f(...)` dispatches to the *parent* MRO entries, never to
                    // the current contract (bases[0]).  Including bases[0] would let a
                    // child-only storage overload of `f` suppress a warning even when
                    // `super.f` actually resolves to a non-storage parent overload.
                    bases.get(1..).unwrap_or_default().to_vec()
                } else {
                    resolutions
                        .iter()
                        .filter_map(|res| {
                            if let Res::Item(ItemId::Contract(cid)) = res {
                                Some(*cid)
                            } else {
                                None
                            }
                        })
                        .collect()
                };

                contract_ids
                    .into_iter()
                    .flat_map(|cid| hir.contract(cid).all_functions())
                    .filter_map(|fid| {
                        let f = hir.function(fid);
                        f.name.is_some_and(|n| n == *method).then_some(f)
                    })
                    .collect()
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

/// For each call argument, if ANY resolved overload has a `storage` parameter at that
/// position, treat the argument as a write target.
fn mark_storage_args<'hir>(
    funcs: &[&Function<'hir>],
    hir: &'hir Hir<'hir>,
    args: &CallArgs<'hir>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
) {
    if let CallArgsKind::Unnamed(_) = args.kind {
        for (i, arg_expr) in args.exprs().enumerate() {
            let any_storage = funcs.iter().any(|func| {
                func.parameters.get(i).is_some_and(|&pid| {
                    matches!(hir.variable(pid).data_location, Some(DataLocation::Storage))
                })
            });
            if any_storage {
                collect_lvalue_writes(arg_expr, candidates, writes);
            }
        }
    }

    if let CallArgsKind::Named(named) = args.kind {
        for named_arg in named {
            let any_storage = funcs.iter().any(|func| {
                let param = func
                    .parameters
                    .iter()
                    .find(|&&pid| hir.variable(pid).name.is_some_and(|n| n == named_arg.name));
                param.is_some_and(|&pid| {
                    matches!(hir.variable(pid).data_location, Some(DataLocation::Storage))
                })
            });
            if any_storage {
                collect_lvalue_writes(&named_arg.value, candidates, writes);
            }
        }
    }
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
        ExprKind::Index(base, _) | ExprKind::Slice(base, _, _) | ExprKind::Member(base, _) => {
            collect_lvalue_writes(base, candidates, writes)
        }
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
