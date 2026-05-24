use super::UnprotectedInitializer;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, FunctionKind, StateMutability, Visibility},
    interface::{Symbol, kw, sym},
    sema::{
        builtins::Builtin,
        hir::{self, ContractId, ExprKind, FunctionId, ItemId, Res, StmtKind, VariableId},
    },
};
use std::collections::HashSet;

declare_forge_lint!(
    UNPROTECTED_INITIALIZER,
    Severity::High,
    "unprotected-initializer",
    "upgradeable initializer is not protected against direct implementation calls"
);

impl<'hir> LateLintPass<'hir> for UnprotectedInitializer {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        contract_id: ContractId,
    ) {
        let contract = hir.contract(contract_id);
        if !matches!(contract.kind, ContractKind::Contract) || contract.linearization_failed() {
            return;
        }

        let upgradeable = contract
            .linearized_bases
            .iter()
            .any(|&base_id| hir.contract(base_id).name.as_str() == "Initializable");
        if !upgradeable
            && !contract.functions().any(|fid| has_initializer_modifier(hir, hir.function(fid)))
        {
            return;
        }

        if initializers_disabled_in_constructor(hir, contract) {
            return;
        }

        if !has_destructive_entrypoint(hir, contract) {
            return;
        }

        for fid in contract.functions() {
            let func = hir.function(fid);
            if !is_public_initializer(hir, func) || has_modifier_named(hir, func, "onlyProxy") {
                continue;
            }

            let Some(body) = func.body else { continue };
            let mut analyzer =
                StateWriteAnalyzer { hir, bases: contract.linearized_bases, stack: Vec::new() };
            if analyzer.block_writes_state(body) {
                ctx.emit(&UNPROTECTED_INITIALIZER, func.name.map_or(func.span, |name| name.span));
            }
        }
    }
}

fn is_public_initializer(hir: &hir::Hir<'_>, func: &hir::Function<'_>) -> bool {
    func.kind.is_function()
        && matches!(func.visibility, Visibility::Public | Visibility::External)
        && !matches!(func.state_mutability, StateMutability::Pure | StateMutability::View)
        && has_initializer_modifier(hir, func)
}

fn initializers_disabled_in_constructor(hir: &hir::Hir<'_>, contract: &hir::Contract<'_>) -> bool {
    contract.linearized_bases.iter().filter_map(|&cid| hir.contract(cid).ctor).any(|ctor_id| {
        let ctor = hir.function(ctor_id);
        function_calls_named(hir, ctor, contract.linearized_bases, "_disableInitializers")
    })
}

fn has_destructive_entrypoint(hir: &hir::Hir<'_>, contract: &hir::Contract<'_>) -> bool {
    effective_runtime_dispatch_surface(hir, contract.linearized_bases).into_iter().any(|fid| {
        let func = hir.function(fid);
        if has_modifier_named(hir, func, "onlyProxy") {
            return false;
        }

        let Some(body) = func.body else { return false };
        let mut finder =
            DestructiveSinkFinder { hir, bases: contract.linearized_bases, stack: vec![fid] };
        finder.block_has_destructive_sink(body)
    })
}

fn effective_runtime_dispatch_surface(hir: &hir::Hir<'_>, bases: &[ContractId]) -> Vec<FunctionId> {
    let mut seen_functions: HashSet<(Symbol, String)> = HashSet::new();
    let mut seen_fallback = false;
    let mut seen_receive = false;
    let mut entries = Vec::new();

    for &cid in bases {
        for fid in hir.contract(cid).all_functions() {
            let func = hir.function(fid);
            match func.kind {
                FunctionKind::Function => {
                    if !matches!(func.visibility, Visibility::Public | Visibility::External) {
                        continue;
                    }
                    let Some(name) = func.name else { continue };
                    if seen_functions.insert((name.name, parameter_signature(hir, func.parameters)))
                    {
                        entries.push(fid);
                    }
                }
                FunctionKind::Fallback => {
                    if !seen_fallback {
                        seen_fallback = true;
                        entries.push(fid);
                    }
                }
                FunctionKind::Receive => {
                    if !seen_receive {
                        seen_receive = true;
                        entries.push(fid);
                    }
                }
                FunctionKind::Constructor | FunctionKind::Modifier => {}
            }
        }
    }

    entries
}

fn parameter_signature(hir: &hir::Hir<'_>, params: &[VariableId]) -> String {
    params
        .iter()
        .map(|&param| format!("{:?}", hir.variable(param).ty.kind))
        .collect::<Vec<_>>()
        .join(",")
}

struct DestructiveSinkFinder<'hir> {
    hir: &'hir hir::Hir<'hir>,
    bases: &'hir [ContractId],
    stack: Vec<FunctionId>,
}

impl<'hir> DestructiveSinkFinder<'hir> {
    fn block_has_destructive_sink(&mut self, block: hir::Block<'hir>) -> bool {
        block.stmts.iter().any(|stmt| self.stmt_has_destructive_sink(stmt))
    }

    fn stmt_has_destructive_sink(&mut self, stmt: &'hir hir::Stmt<'hir>) -> bool {
        match &stmt.kind {
            StmtKind::DeclSingle(var_id) => self
                .hir
                .variable(*var_id)
                .initializer
                .is_some_and(|init| self.expr_has_destructive_sink(init)),
            StmtKind::DeclMulti(_, expr)
            | StmtKind::Emit(expr)
            | StmtKind::Revert(expr)
            | StmtKind::Return(Some(expr))
            | StmtKind::Expr(expr) => self.expr_has_destructive_sink(expr),
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                self.block_has_destructive_sink(*block)
            }
            StmtKind::If(condition, then_stmt, else_stmt) => {
                self.expr_has_destructive_sink(condition)
                    || self.stmt_has_destructive_sink(then_stmt)
                    || else_stmt.is_some_and(|stmt| self.stmt_has_destructive_sink(stmt))
            }
            StmtKind::Try(stmt_try) => {
                self.expr_has_destructive_sink(&stmt_try.expr)
                    || stmt_try
                        .clauses
                        .iter()
                        .any(|clause| self.block_has_destructive_sink(clause.block))
            }
            StmtKind::Return(None)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Placeholder
            | StmtKind::Err(_) => false,
        }
    }

    fn expr_has_destructive_sink(&mut self, expr: &'hir hir::Expr<'hir>) -> bool {
        match &expr.kind {
            ExprKind::Call(callee, args, opts) => {
                if is_destructive_call(callee) {
                    return true;
                }

                if self.expr_has_destructive_sink(callee)
                    || opts.is_some_and(|opts| {
                        opts.iter().any(|opt| self.expr_has_destructive_sink(&opt.value))
                    })
                    || args.exprs().any(|arg| self.expr_has_destructive_sink(arg))
                {
                    return true;
                }

                resolved_internal_function_ids(self.hir, callee, self.bases)
                    .into_iter()
                    .any(|func_id| self.function_has_destructive_sink(func_id))
            }
            ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
                self.expr_has_destructive_sink(lhs) || self.expr_has_destructive_sink(rhs)
            }
            ExprKind::Unary(_, inner) | ExprKind::Delete(inner) | ExprKind::Payable(inner) => {
                self.expr_has_destructive_sink(inner)
            }
            ExprKind::Index(base, index) => {
                self.expr_has_destructive_sink(base)
                    || index.is_some_and(|index| self.expr_has_destructive_sink(index))
            }
            ExprKind::Slice(base, start, end) => {
                self.expr_has_destructive_sink(base)
                    || start.is_some_and(|start| self.expr_has_destructive_sink(start))
                    || end.is_some_and(|end| self.expr_has_destructive_sink(end))
            }
            ExprKind::Member(base, _) => self.expr_has_destructive_sink(base),
            ExprKind::Ternary(condition, if_true, if_false) => {
                self.expr_has_destructive_sink(condition)
                    || self.expr_has_destructive_sink(if_true)
                    || self.expr_has_destructive_sink(if_false)
            }
            ExprKind::Array(exprs) => exprs.iter().any(|expr| self.expr_has_destructive_sink(expr)),
            ExprKind::Tuple(exprs) => {
                exprs.iter().flatten().any(|expr| self.expr_has_destructive_sink(expr))
            }
            ExprKind::Lit(_)
            | ExprKind::Ident(_)
            | ExprKind::New(_)
            | ExprKind::TypeCall(_)
            | ExprKind::Type(_)
            | ExprKind::Err(_) => false,
        }
    }

    fn function_has_destructive_sink(&mut self, func_id: FunctionId) -> bool {
        if self.stack.contains(&func_id) {
            return false;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return false };
        self.stack.push(func_id);
        let found = self.block_has_destructive_sink(body);
        self.stack.pop();
        found
    }
}

fn is_destructive_call(callee: &hir::Expr<'_>) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Member(_, member) => matches!(member.name, kw::Delegatecall | kw::Callcode),
        ExprKind::Ident(resolutions) => {
            resolutions.iter().any(|res| matches!(res, Res::Builtin(Builtin::Selfdestruct)))
        }
        _ => false,
    }
}

fn has_initializer_modifier(hir: &hir::Hir<'_>, func: &hir::Function<'_>) -> bool {
    has_modifier_named(hir, func, "initializer") || has_modifier_named(hir, func, "reinitializer")
}

fn has_modifier_named(hir: &hir::Hir<'_>, func: &hir::Function<'_>, name: &str) -> bool {
    func.modifiers.iter().any(|modifier| modifier_name_is(hir, modifier, name))
}

fn modifier_name_is(hir: &hir::Hir<'_>, modifier: &hir::Modifier<'_>, name: &str) -> bool {
    match modifier.id {
        ItemId::Function(fid) => hir.function(fid).name.is_some_and(|ident| ident.as_str() == name),
        ItemId::Contract(cid) => hir.contract(cid).name.as_str() == name,
        _ => false,
    }
}

fn function_calls_named(
    hir: &hir::Hir<'_>,
    func: &hir::Function<'_>,
    bases: &[ContractId],
    name: &str,
) -> bool {
    let Some(body) = func.body else { return false };
    let mut finder = CallNameFinder { hir, name, bases, stack: vec![] };
    finder.block_calls_named(body)
}

struct CallNameFinder<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    name: &'a str,
    bases: &'hir [ContractId],
    stack: Vec<FunctionId>,
}

impl<'hir> CallNameFinder<'_, 'hir> {
    fn block_calls_named(&mut self, block: hir::Block<'hir>) -> bool {
        block.stmts.iter().any(|stmt| self.stmt_calls_named(stmt))
    }

    fn stmt_calls_named(&mut self, stmt: &'hir hir::Stmt<'hir>) -> bool {
        match &stmt.kind {
            StmtKind::DeclSingle(var_id) => self
                .hir
                .variable(*var_id)
                .initializer
                .is_some_and(|init| self.expr_calls_named(init)),
            StmtKind::DeclMulti(_, expr)
            | StmtKind::Emit(expr)
            | StmtKind::Revert(expr)
            | StmtKind::Return(Some(expr))
            | StmtKind::Expr(expr) => self.expr_calls_named(expr),
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                self.block_calls_named(*block)
            }
            StmtKind::If(condition, then_stmt, else_stmt) => {
                self.expr_calls_named(condition)
                    || self.stmt_calls_named(then_stmt)
                    || else_stmt.is_some_and(|stmt| self.stmt_calls_named(stmt))
            }
            StmtKind::Try(stmt_try) => {
                self.expr_calls_named(&stmt_try.expr)
                    || stmt_try.clauses.iter().any(|clause| self.block_calls_named(clause.block))
            }
            StmtKind::Return(None)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Placeholder
            | StmtKind::Err(_) => false,
        }
    }

    fn expr_calls_named(&mut self, expr: &'hir hir::Expr<'hir>) -> bool {
        match &expr.kind {
            ExprKind::Call(callee, args, opts) => {
                if callee_name_is(self.hir, callee, self.name) {
                    return true;
                }

                if let Some(opts) = opts
                    && opts.iter().any(|opt| self.expr_calls_named(&opt.value))
                {
                    return true;
                }

                if args.exprs().any(|arg| self.expr_calls_named(arg)) {
                    return true;
                }

                resolved_internal_function_ids(self.hir, callee, self.bases)
                    .into_iter()
                    .any(|func_id| self.function_calls_named(func_id))
            }
            ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
                self.expr_calls_named(lhs) || self.expr_calls_named(rhs)
            }
            ExprKind::Unary(_, inner) | ExprKind::Delete(inner) | ExprKind::Payable(inner) => {
                self.expr_calls_named(inner)
            }
            ExprKind::Index(base, index) => {
                self.expr_calls_named(base)
                    || index.is_some_and(|index| self.expr_calls_named(index))
            }
            ExprKind::Slice(base, start, end) => {
                self.expr_calls_named(base)
                    || start.is_some_and(|start| self.expr_calls_named(start))
                    || end.is_some_and(|end| self.expr_calls_named(end))
            }
            ExprKind::Member(base, _) => self.expr_calls_named(base),
            ExprKind::Ternary(condition, if_true, if_false) => {
                self.expr_calls_named(condition)
                    || self.expr_calls_named(if_true)
                    || self.expr_calls_named(if_false)
            }
            ExprKind::Array(exprs) => exprs.iter().any(|expr| self.expr_calls_named(expr)),
            ExprKind::Tuple(exprs) => {
                exprs.iter().flatten().any(|expr| self.expr_calls_named(expr))
            }
            ExprKind::Lit(_)
            | ExprKind::Ident(_)
            | ExprKind::New(_)
            | ExprKind::TypeCall(_)
            | ExprKind::Type(_)
            | ExprKind::Err(_) => false,
        }
    }

    fn function_calls_named(&mut self, func_id: FunctionId) -> bool {
        if self.stack.contains(&func_id) {
            return false;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return false };
        self.stack.push(func_id);
        let found = self.block_calls_named(body);
        self.stack.pop();
        found
    }
}

fn callee_name_is(hir: &hir::Hir<'_>, callee: &hir::Expr<'_>, name: &str) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Ident(resolutions) => resolutions.iter().any(|res| {
            matches!(res, Res::Item(ItemId::Function(fid)) if hir.function(*fid).name.is_some_and(|ident| ident.as_str() == name))
        }),
        ExprKind::Member(_, member) => member.as_str() == name,
        _ => false,
    }
}

struct StateWriteAnalyzer<'hir> {
    hir: &'hir hir::Hir<'hir>,
    bases: &'hir [ContractId],
    stack: Vec<FunctionId>,
}

impl<'hir> StateWriteAnalyzer<'hir> {
    fn block_writes_state(&mut self, block: hir::Block<'hir>) -> bool {
        block.stmts.iter().any(|stmt| self.stmt_writes_state(stmt))
    }

    fn stmt_writes_state(&mut self, stmt: &'hir hir::Stmt<'hir>) -> bool {
        match &stmt.kind {
            StmtKind::DeclSingle(var_id) => self
                .hir
                .variable(*var_id)
                .initializer
                .is_some_and(|init| self.expr_writes_state(init)),
            StmtKind::DeclMulti(_, expr)
            | StmtKind::Emit(expr)
            | StmtKind::Revert(expr)
            | StmtKind::Return(Some(expr))
            | StmtKind::Expr(expr) => self.expr_writes_state(expr),
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                self.block_writes_state(*block)
            }
            StmtKind::If(condition, then_stmt, else_stmt) => {
                self.expr_writes_state(condition)
                    || self.stmt_writes_state(then_stmt)
                    || else_stmt.is_some_and(|stmt| self.stmt_writes_state(stmt))
            }
            StmtKind::Try(stmt_try) => {
                self.expr_writes_state(&stmt_try.expr)
                    || stmt_try.clauses.iter().any(|clause| self.block_writes_state(clause.block))
            }
            StmtKind::Return(None)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Placeholder
            | StmtKind::Err(_) => false,
        }
    }

    fn expr_writes_state(&mut self, expr: &'hir hir::Expr<'hir>) -> bool {
        match &expr.kind {
            ExprKind::Assign(lhs, _, rhs) => {
                state_write_lhs_vars(self.hir, lhs).next().is_some()
                    || self.expr_writes_state(lhs)
                    || self.expr_writes_state(rhs)
            }
            ExprKind::Delete(inner) => {
                state_write_lhs_vars(self.hir, inner).next().is_some()
                    || self.expr_writes_state(inner)
            }
            ExprKind::Unary(op, inner) => {
                (op.kind.has_side_effects()
                    && state_write_lhs_vars(self.hir, inner).next().is_some())
                    || self.expr_writes_state(inner)
            }
            ExprKind::Call(callee, args, opts) => {
                if member_call_writes_state(self.hir, callee) {
                    return true;
                }

                if self.expr_writes_state(callee)
                    || opts.is_some_and(|opts| {
                        opts.iter().any(|opt| self.expr_writes_state(&opt.value))
                    })
                    || args.exprs().any(|arg| self.expr_writes_state(arg))
                {
                    return true;
                }

                resolved_internal_function_ids(self.hir, callee, self.bases)
                    .into_iter()
                    .any(|func_id| self.function_writes_state(func_id))
            }
            ExprKind::Binary(lhs, _, rhs) => {
                self.expr_writes_state(lhs) || self.expr_writes_state(rhs)
            }
            ExprKind::Index(base, index) => {
                self.expr_writes_state(base)
                    || index.is_some_and(|index| self.expr_writes_state(index))
            }
            ExprKind::Slice(base, start, end) => {
                self.expr_writes_state(base)
                    || start.is_some_and(|start| self.expr_writes_state(start))
                    || end.is_some_and(|end| self.expr_writes_state(end))
            }
            ExprKind::Member(base, _) | ExprKind::Payable(base) => self.expr_writes_state(base),
            ExprKind::Ternary(condition, if_true, if_false) => {
                self.expr_writes_state(condition)
                    || self.expr_writes_state(if_true)
                    || self.expr_writes_state(if_false)
            }
            ExprKind::Array(exprs) => exprs.iter().any(|expr| self.expr_writes_state(expr)),
            ExprKind::Tuple(exprs) => {
                exprs.iter().flatten().any(|expr| self.expr_writes_state(expr))
            }
            ExprKind::Lit(_)
            | ExprKind::Ident(_)
            | ExprKind::New(_)
            | ExprKind::TypeCall(_)
            | ExprKind::Type(_)
            | ExprKind::Err(_) => false,
        }
    }

    fn function_writes_state(&mut self, func_id: FunctionId) -> bool {
        if self.stack.contains(&func_id) {
            return false;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return false };
        self.stack.push(func_id);
        let writes = self.block_writes_state(body);
        self.stack.pop();
        writes
    }
}

fn member_call_writes_state(hir: &hir::Hir<'_>, callee: &hir::Expr<'_>) -> bool {
    let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return false };
    matches!(member.as_str(), "push" | "pop") && state_write_lhs_vars(hir, base).next().is_some()
}

fn state_write_lhs_vars<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> impl Iterator<Item = VariableId> + 'hir {
    let mut vars = HashSet::new();
    collect_state_write_lhs_vars(hir, expr, &mut vars);
    vars.into_iter()
}

fn collect_state_write_lhs_vars(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    vars: &mut HashSet<VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            for res in *resolutions {
                if let Res::Item(ItemId::Variable(var_id)) = res
                    && hir.variable(*var_id).kind.is_state()
                {
                    vars.insert(*var_id);
                }
            }
        }
        ExprKind::Index(base, _) | ExprKind::Slice(base, _, _) | ExprKind::Member(base, _) => {
            collect_state_write_lhs_vars(hir, base, vars);
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_state_write_lhs_vars(hir, expr, vars);
            }
        }
        _ => {}
    }
}

fn resolved_internal_function_ids(
    hir: &hir::Hir<'_>,
    callee: &hir::Expr<'_>,
    bases: &[ContractId],
) -> Vec<FunctionId> {
    match &callee.peel_parens().kind {
        ExprKind::Ident(resolutions) => resolutions
            .iter()
            .filter_map(|res| match res {
                Res::Item(ItemId::Function(func_id)) => Some(*func_id),
                _ => None,
            })
            .collect(),
        ExprKind::Member(base, method) => {
            let ExprKind::Ident(resolutions) = &base.peel_parens().kind else { return vec![] };
            let is_super = resolutions
                .iter()
                .any(|res| matches!(res, Res::Builtin(builtin) if builtin.name() == sym::super_));

            let contracts: Vec<_> = if is_super {
                bases.get(1..).unwrap_or_default().to_vec()
            } else {
                resolutions
                    .iter()
                    .filter_map(|res| match res {
                        Res::Item(ItemId::Contract(cid)) => Some(*cid),
                        _ => None,
                    })
                    .collect()
            };

            contracts
                .into_iter()
                .flat_map(|cid| hir.contract(cid).all_functions())
                .filter(|&fid| {
                    hir.function(fid).name.is_some_and(|name| name.as_str() == method.as_str())
                })
                .collect()
        }
        _ => vec![],
    }
}
