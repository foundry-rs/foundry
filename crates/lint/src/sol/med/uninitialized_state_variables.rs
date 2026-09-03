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
            ItemId, LoopSource, Res, Stmt, StmtKind, TypeKind, VariableId, Visit,
        },
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

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
        _gcx: solar::sema::Gcx<'hir>,
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
        // Bail out conservatively if any function body contains inline assembly,
        // because we cannot soundly track reads or writes through it.
        let bases = contract.linearized_bases;

        for &cid in bases {
            for func_id in hir.contract(cid).all_functions() {
                let function = hir.function(func_id);
                // Local variable IDs cannot cross function boundaries.
                let mut aliases = HashMap::new();

                for modifier in function.modifiers {
                    for expr in modifier.args.exprs() {
                        if collect_expr_writes_checked(
                            hir,
                            expr,
                            &candidate_set,
                            &mut written,
                            bases,
                            &mut aliases,
                        )
                        .is_err()
                        {
                            return;
                        }
                    }
                }

                if let Some(body) = function.body
                    && collect_block_writes_checked(
                        hir,
                        body,
                        &candidate_set,
                        &mut written,
                        bases,
                        &mut aliases,
                    )
                    .is_err()
                {
                    return;
                }
            }

            for base_modifier in hir.contract(cid).bases_args {
                for expr in base_modifier.args.exprs() {
                    let mut aliases = HashMap::new();
                    if collect_expr_writes_checked(
                        hir,
                        expr,
                        &candidate_set,
                        &mut written,
                        bases,
                        &mut aliases,
                    )
                    .is_err()
                    {
                        return;
                    }
                }
            }

            // Walk state-vars initializer expressions for side-effect writes to other state vars
            for var_id in hir.contract(cid).variables() {
                if let Some(init) = hir.variable(var_id).initializer {
                    let mut aliases = HashMap::new();
                    if collect_expr_writes_checked(
                        hir,
                        init,
                        &candidate_set,
                        &mut written,
                        bases,
                        &mut aliases,
                    )
                    .is_err()
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

type Aliases = HashMap<VariableId, HashSet<VariableId>>;

#[derive(Default)]
struct Flow {
    falls_through: bool,
    breaks: Option<Aliases>,
    continues: Option<Aliases>,
}

impl Flow {
    const fn fallthrough() -> Self {
        Self { falls_through: true, breaks: None, continues: None }
    }
}

fn collect_block_writes_checked<'hir>(
    hir: &'hir Hir<'hir>,
    block: Block<'hir>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
    bases: &'hir [ContractId],
    aliases: &mut Aliases,
) -> Result<Flow, ()> {
    collect_stmts_writes_checked(hir, block.stmts, candidates, writes, bases, aliases)
}

fn collect_stmts_writes_checked<'hir>(
    hir: &'hir Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
    bases: &'hir [ContractId],
    aliases: &mut Aliases,
) -> Result<Flow, ()> {
    let mut flow = Flow::fallthrough();
    for stmt in stmts {
        if !flow.falls_through {
            break;
        }
        let stmt_flow = collect_stmt_writes_checked(hir, stmt, candidates, writes, bases, aliases)?;
        merge_optional_aliases(&mut flow.breaks, stmt_flow.breaks);
        merge_optional_aliases(&mut flow.continues, stmt_flow.continues);
        flow.falls_through = stmt_flow.falls_through;
    }
    Ok(flow)
}

fn collect_stmt_writes_checked<'hir>(
    hir: &'hir Hir<'hir>,
    stmt: &'hir Stmt<'hir>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
    bases: &'hir [ContractId],
    aliases: &mut Aliases,
) -> Result<Flow, ()> {
    match &stmt.kind {
        // Assembly can write storage directly; bail conservatively.
        StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => Err(()),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            collect_block_writes_checked(hir, *block, candidates, writes, bases, aliases)
        }
        StmtKind::Loop(block, source) => {
            let entry = aliases.clone();
            let mut head = entry.clone();
            // Re-run the loop until every reachable alias target has propagated through its
            // backedges. Alias sets only grow at the loop head, so this always converges.
            loop {
                let mut iteration_aliases = head.clone();
                let flow = collect_loop_iteration(
                    hir,
                    *block,
                    *source,
                    candidates,
                    writes,
                    bases,
                    &mut iteration_aliases,
                )?;

                let mut next_head = entry.clone();
                if flow.falls_through {
                    merge_aliases(&mut next_head, iteration_aliases);
                }
                if let Some(continued) = flow.continues {
                    merge_aliases(&mut next_head, continued);
                }

                if next_head == head {
                    if let Some(exits) = flow.breaks {
                        *aliases = exits;
                        return Ok(Flow::fallthrough());
                    }
                    return Ok(Flow::default());
                }
                head = next_head;
            }
        }
        StmtKind::If(condition, then_stmt, else_stmt) => {
            collect_expr_writes_checked(hir, condition, candidates, writes, bases, aliases)?;
            let mut then_aliases = aliases.clone();
            let then_flow = collect_stmt_writes_checked(
                hir,
                then_stmt,
                candidates,
                writes,
                bases,
                &mut then_aliases,
            )?;
            let mut else_aliases = aliases.clone();
            let else_flow = if let Some(else_stmt) = else_stmt {
                collect_stmt_writes_checked(
                    hir,
                    else_stmt,
                    candidates,
                    writes,
                    bases,
                    &mut else_aliases,
                )?
            } else {
                Flow::fallthrough()
            };

            let mut fallthrough = None;
            if then_flow.falls_through {
                merge_optional_aliases(&mut fallthrough, Some(then_aliases));
            }
            if else_flow.falls_through {
                merge_optional_aliases(&mut fallthrough, Some(else_aliases));
            }
            let falls_through = fallthrough.is_some();
            if let Some(joined) = fallthrough {
                *aliases = joined;
            }
            Ok(Flow {
                falls_through,
                breaks: merge_optional(then_flow.breaks, else_flow.breaks),
                continues: merge_optional(then_flow.continues, else_flow.continues),
            })
        }
        StmtKind::Try(stmt_try) => {
            collect_expr_writes_checked(hir, &stmt_try.expr, candidates, writes, bases, aliases)?;
            let before = aliases.clone();
            let mut fallthrough = None;
            let mut breaks = None;
            let mut continues = None;
            for clause in stmt_try.clauses {
                let mut clause_aliases = before.clone();
                let flow = collect_block_writes_checked(
                    hir,
                    clause.block,
                    candidates,
                    writes,
                    bases,
                    &mut clause_aliases,
                )?;
                if flow.falls_through {
                    merge_optional_aliases(&mut fallthrough, Some(clause_aliases));
                }
                merge_optional_aliases(&mut breaks, flow.breaks);
                merge_optional_aliases(&mut continues, flow.continues);
            }
            let falls_through = fallthrough.is_some();
            if let Some(joined) = fallthrough {
                *aliases = joined;
            }
            Ok(Flow { falls_through, breaks, continues })
        }
        StmtKind::DeclSingle(var_id) => {
            if let Some(initializer) = hir.variable(*var_id).initializer {
                collect_expr_writes_checked(hir, initializer, candidates, writes, bases, aliases)?;

                if hir.variable(*var_id).data_location == Some(DataLocation::Storage)
                    && let Some(target) = resolve_alias_targets(initializer, candidates, aliases)
                {
                    aliases.insert(*var_id, target);
                }
            }
            Ok(Flow::fallthrough())
        }
        StmtKind::DeclMulti(_, expr) | StmtKind::Emit(expr) | StmtKind::Expr(expr) => {
            collect_expr_writes_checked(hir, expr, candidates, writes, bases, aliases)?;
            Ok(Flow::fallthrough())
        }
        StmtKind::Revert(expr) | StmtKind::Return(Some(expr)) => {
            collect_expr_writes_checked(hir, expr, candidates, writes, bases, aliases)?;
            Ok(Flow::default())
        }
        StmtKind::Return(None) => Ok(Flow::default()),
        StmtKind::Break => Ok(Flow { breaks: Some(aliases.clone()), ..Flow::default() }),
        StmtKind::Continue => Ok(Flow { continues: Some(aliases.clone()), ..Flow::default() }),
        StmtKind::Placeholder => Ok(Flow::fallthrough()),
    }
}

fn collect_loop_iteration<'hir>(
    hir: &'hir Hir<'hir>,
    block: Block<'hir>,
    source: LoopSource,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
    bases: &'hir [ContractId],
    aliases: &mut Aliases,
) -> Result<Flow, ()> {
    if source == LoopSource::DoWhile
        && let Some((condition, body)) = block.stmts.split_last()
    {
        let mut body_flow =
            collect_stmts_writes_checked(hir, body, candidates, writes, bases, aliases)?;
        let mut condition_aliases = None;
        if body_flow.falls_through {
            merge_optional_aliases(&mut condition_aliases, Some(aliases.clone()));
        }
        merge_optional_aliases(&mut condition_aliases, body_flow.continues.take());

        if let Some(mut condition_aliases) = condition_aliases {
            let condition_flow = collect_stmt_writes_checked(
                hir,
                condition,
                candidates,
                writes,
                bases,
                &mut condition_aliases,
            )?;
            if condition_flow.falls_through {
                *aliases = condition_aliases;
            }
            merge_optional_aliases(&mut body_flow.breaks, condition_flow.breaks);
            return Ok(Flow {
                falls_through: condition_flow.falls_through,
                breaks: body_flow.breaks,
                continues: condition_flow.continues,
            });
        }
        return Ok(Flow { breaks: body_flow.breaks, ..Flow::default() });
    }

    let mut flow = collect_block_writes_checked(hir, block, candidates, writes, bases, aliases)?;
    if source == LoopSource::ForWithUpdate
        && let Some(next) = for_loop_next_expr(block)
        && let Some(continued) = &mut flow.continues
    {
        collect_expr_writes_checked(hir, next, candidates, writes, bases, continued)?;
    }
    Ok(flow)
}

fn for_loop_next_expr<'hir>(block: Block<'hir>) -> Option<&'hir Expr<'hir>> {
    let [stmt] = block.stmts else { return None };
    let inner = match &stmt.kind {
        StmtKind::If(_, then_stmt, _) => {
            let StmtKind::Block(inner) = &then_stmt.kind else { return None };
            inner
        }
        StmtKind::Block(inner) => inner,
        _ => return None,
    };
    if inner.span != block.span {
        return None;
    }
    let [_, next] = inner.stmts else { return None };
    let StmtKind::Expr(next) = &next.kind else { return None };
    Some(*next)
}

fn collect_expr_writes_checked<'hir>(
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
    bases: &'hir [ContractId],
    aliases: &mut Aliases,
) -> Result<(), ()> {
    match &expr.kind {
        ExprKind::Assign(lhs, _, rhs) => {
            // Reassigning a bare storage pointer repoints it rather than writing its target.
            let is_bare_alias_repoint = matches!(
                &lhs.peel_parens().kind,
                ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..])
                    if !candidates.contains(id) && aliases.contains_key(id)
            );
            if !is_bare_alias_repoint {
                collect_lvalue_writes(lhs, candidates, writes, Some(aliases));
            }
            collect_expr_writes_checked(hir, lhs, candidates, writes, bases, aliases)?;
            collect_expr_writes_checked(hir, rhs, candidates, writes, bases, aliases)?;

            // Replace the alias target, dropping stale targets when the RHS is unresolved.
            if let ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) = &lhs.peel_parens().kind
                && !candidates.contains(id)
                && hir.variable(*id).data_location == Some(DataLocation::Storage)
            {
                match resolve_alias_targets(rhs, candidates, aliases) {
                    Some(targets) => {
                        aliases.insert(*id, targets);
                    }
                    None => {
                        aliases.remove(id);
                    }
                }
            }
        }
        ExprKind::Delete(inner) => {
            collect_lvalue_writes(inner, candidates, writes, Some(aliases));
            collect_expr_writes_checked(hir, inner, candidates, writes, bases, aliases)?;
        }
        ExprKind::Unary(op, inner) => {
            if op.kind.has_side_effects() {
                collect_lvalue_writes(inner, candidates, writes, Some(aliases));
            }
            collect_expr_writes_checked(hir, inner, candidates, writes, bases, aliases)?;
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_expr_writes_checked(hir, expr, candidates, writes, bases, aliases)?;
            }
        }
        ExprKind::Binary(lhs, _, rhs) => {
            collect_expr_writes_checked(hir, lhs, candidates, writes, bases, aliases)?;
            collect_expr_writes_checked(hir, rhs, candidates, writes, bases, aliases)?;
        }
        ExprKind::Call(callee, args, named_args) => {
            if let ExprKind::Member(base, _) = &callee.kind {
                // Covers push/pop and library dispatch (`using Lib for T` with `T storage self`);
                // can't resolve callee without Gcx. Treat the receiver as a write target to avoid
                // false positives. Do not extend this heuristic through aliases: the call may be
                // read-only.
                collect_lvalue_writes(base, candidates, writes, None);
            }

            // Direct calls to internal functions that take a `storage` parameter
            // mutate the corresponding argument in place; treat it as a write.
            //
            // Handles bare identifier callees (`_set(slot, v)`) and qualified member
            // callees (`BaseSetter._set(slot, v)`, `super._set(slot, v)`). This conservative
            // heuristic remains limited to direct state arguments because storage parameters may
            // be read-only.
            let funcs = collect_callee_funcs(hir, callee, bases);
            if !funcs.is_empty() {
                mark_storage_args(&funcs, hir, args, candidates, writes);
            }

            collect_expr_writes_checked(hir, callee, candidates, writes, bases, aliases)?;
            for expr in args.exprs() {
                collect_expr_writes_checked(hir, expr, candidates, writes, bases, aliases)?;
            }
            if let Some(named_args) = named_args {
                for arg in named_args.args {
                    collect_expr_writes_checked(
                        hir, &arg.value, candidates, writes, bases, aliases,
                    )?;
                }
            }
        }
        ExprKind::Index(base, index) => {
            collect_expr_writes_checked(hir, base, candidates, writes, bases, aliases)?;
            if let Some(index) = index {
                collect_expr_writes_checked(hir, index, candidates, writes, bases, aliases)?;
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_expr_writes_checked(hir, base, candidates, writes, bases, aliases)?;
            if let Some(start) = start {
                collect_expr_writes_checked(hir, start, candidates, writes, bases, aliases)?;
            }
            if let Some(end) = end {
                collect_expr_writes_checked(hir, end, candidates, writes, bases, aliases)?;
            }
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) => {
            collect_expr_writes_checked(hir, base, candidates, writes, bases, aliases)?;
        }
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            collect_expr_writes_checked(hir, condition, candidates, writes, bases, aliases)?;
            let mut then_aliases = aliases.clone();
            collect_expr_writes_checked(
                hir,
                then_expr,
                candidates,
                writes,
                bases,
                &mut then_aliases,
            )?;
            let mut else_aliases = aliases.clone();
            collect_expr_writes_checked(
                hir,
                else_expr,
                candidates,
                writes,
                bases,
                &mut else_aliases,
            )?;
            merge_aliases(&mut then_aliases, else_aliases);
            *aliases = then_aliases;
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_expr_writes_checked(hir, expr, candidates, writes, bases, aliases)?;
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
                collect_lvalue_writes(arg_expr, candidates, writes, None);
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
                collect_lvalue_writes(&named_arg.value, candidates, writes, None);
            }
        }
    }
}

/// Resolves a storage pointer to its possible state-variable targets.
fn resolve_alias_targets(
    expr: &Expr<'_>,
    candidates: &HashSet<VariableId>,
    aliases: &HashMap<VariableId, HashSet<VariableId>>,
) -> Option<HashSet<VariableId>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) => {
            if candidates.contains(id) {
                Some(HashSet::from([*id]))
            } else {
                aliases.get(id).cloned()
            }
        }
        ExprKind::Index(base, _) | ExprKind::Slice(base, _, _) | ExprKind::Member(base, _) => {
            resolve_alias_targets(base, candidates, aliases)
        }
        _ => None,
    }
}

fn merge_aliases(aliases: &mut Aliases, other: Aliases) {
    for (alias, targets) in other {
        aliases.entry(alias).or_default().extend(targets);
    }
}

fn merge_optional_aliases(aliases: &mut Option<Aliases>, other: Option<Aliases>) {
    if let Some(other) = other {
        if let Some(aliases) = aliases {
            merge_aliases(aliases, other);
        } else {
            *aliases = Some(other);
        }
    }
}

fn merge_optional(mut aliases: Option<Aliases>, other: Option<Aliases>) -> Option<Aliases> {
    merge_optional_aliases(&mut aliases, other);
    aliases
}

fn collect_lvalue_writes(
    expr: &Expr<'_>,
    candidates: &HashSet<VariableId>,
    writes: &mut HashSet<VariableId>,
    aliases: Option<&Aliases>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) => {
            if candidates.contains(id) {
                writes.insert(*id);
            } else if let Some(targets) = aliases.and_then(|aliases| aliases.get(id)) {
                writes.extend(targets);
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_lvalue_writes(expr, candidates, writes, aliases);
            }
        }
        ExprKind::Index(base, _) | ExprKind::Slice(base, _, _) | ExprKind::Member(base, _) => {
            collect_lvalue_writes(base, candidates, writes, aliases)
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
