use super::MissingEventsAccessControl;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::primitives::{branch_always_exits, is_require_or_assert},
    },
};
use solar::{
    ast::{ContractKind, DataLocation, LitKind, StateMutability, Visibility},
    interface::{Span, kw, sym},
    sema::hir::{self, EventId, ExprKind, FunctionId, ItemId, Res, StmtKind, VariableId},
};
use std::collections::{HashMap, HashSet};

declare_forge_lint!(
    MISSING_EVENTS_ACCESS_CONTROL,
    Severity::Low,
    "missing-events-access-control",
    "access control changes should emit events"
);

impl<'hir> LateLintPass<'hir> for MissingEventsAccessControl {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        _gcx: solar::sema::Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        contract: &'hir hir::Contract<'hir>,
    ) {
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract) {
            return;
        }

        let access_control_vars = access_control_state_vars(hir, contract);
        if access_control_vars.is_empty() {
            return;
        }

        for func_id in contract.all_functions() {
            let func = hir.function(func_id);
            if !is_protected_entry_point(hir, func_id, func) {
                continue;
            }

            let guard_vars = entry_point_access_guard_vars(hir, func_id, func);
            let mut analyzer = WriteAnalyzer::new(hir, &access_control_vars, &guard_vars);
            let writes = analyzer.analyze_entry_point(func_id);
            let mut emitted = HashSet::new();

            for write in writes {
                if write.evented {
                    continue;
                }

                if !emitted.insert(write.var_id) {
                    continue;
                }

                let name = hir
                    .variable(write.var_id)
                    .name
                    .map(|name| name.as_str().to_string())
                    .unwrap_or_else(|| "state variable".to_string());
                ctx.emit_with_msg(
                    &MISSING_EVENTS_ACCESS_CONTROL,
                    write.span,
                    format!("`{name}` is changed without an event but is used for access control"),
                );
            }
        }
    }
}

fn is_external_function(func: &hir::Function<'_>) -> bool {
    func.kind.is_function()
        && matches!(func.visibility, Visibility::Public | Visibility::External)
        && !func.is_constructor()
        && !func.is_special()
}

fn is_state_mutating_entry_point(func: &hir::Function<'_>) -> bool {
    is_external_function(func)
        && !matches!(func.state_mutability, StateMutability::Pure | StateMutability::View)
}

fn is_protected_entry_point(
    hir: &hir::Hir<'_>,
    func_id: FunctionId,
    func: &hir::Function<'_>,
) -> bool {
    is_state_mutating_entry_point(func) && is_protected(hir, func_id, func)
}

fn access_control_state_vars(
    hir: &hir::Hir<'_>,
    contract: &hir::Contract<'_>,
) -> HashSet<VariableId> {
    let mut out = HashSet::new();

    for func_id in contract.all_functions() {
        let func = hir.function(func_id);
        for modifier in func.modifiers {
            if let Some(modifier_id) = modifier.id.as_function() {
                collect_access_control_state_vars_in_function(
                    hir,
                    modifier_id,
                    &mut HashSet::new(),
                    &mut HashSet::new(),
                    &mut out,
                    true,
                );
            }
        }

        collect_access_control_state_vars_in_function(
            hir,
            func_id,
            &mut HashSet::new(),
            &mut HashSet::new(),
            &mut out,
            false,
        );
    }

    out.retain(|var_id| {
        let var = hir.variable(*var_id);
        var.kind.is_state() && !var.is_constant() && !var.is_immutable()
    });
    out
}

fn entry_point_access_guard_vars(
    hir: &hir::Hir<'_>,
    func_id: FunctionId,
    func: &hir::Function<'_>,
) -> HashSet<VariableId> {
    let mut out = HashSet::new();

    for modifier in func.modifiers {
        if let Some(modifier_id) = modifier.id.as_function() {
            collect_access_control_state_vars_in_function(
                hir,
                modifier_id,
                &mut HashSet::new(),
                &mut HashSet::new(),
                &mut out,
                true,
            );
        }
    }

    collect_access_control_state_vars_in_function(
        hir,
        func_id,
        &mut HashSet::new(),
        &mut HashSet::new(),
        &mut out,
        false,
    );
    out
}

fn collect_access_control_state_vars_in_function(
    hir: &hir::Hir<'_>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
    sender_aliases: &mut HashSet<VariableId>,
    out: &mut HashSet<VariableId>,
    stop_at_placeholder: bool,
) {
    if !seen.insert(func_id) {
        return;
    }

    let func = hir.function(func_id);
    let Some(body) = func.body else { return };

    for stmt in body.stmts {
        if stop_at_placeholder && matches!(stmt.kind, StmtKind::Placeholder) {
            break;
        }
        collect_access_control_state_vars_in_stmt(hir, stmt, seen, sender_aliases, out);
    }
}

fn collect_access_control_state_vars_in_stmt(
    hir: &hir::Hir<'_>,
    stmt: &hir::Stmt<'_>,
    seen: &mut HashSet<FunctionId>,
    sender_aliases: &mut HashSet<VariableId>,
    out: &mut HashSet<VariableId>,
) {
    match stmt.kind {
        StmtKind::If(cond, then_stmt, else_stmt) => {
            if expr_looks_like_access_check(hir, cond, sender_aliases)
                && (stmt_exits_or_reverts(then_stmt)
                    || else_stmt.is_some_and(stmt_exits_or_reverts))
            {
                collect_access_check_state_vars(hir, cond, seen, out);
            }
            let mut then_aliases = sender_aliases.clone();
            collect_access_control_state_vars_in_stmt(hir, then_stmt, seen, &mut then_aliases, out);
            if let Some(else_stmt) = else_stmt {
                let mut else_aliases = sender_aliases.clone();
                collect_access_control_state_vars_in_stmt(
                    hir,
                    else_stmt,
                    seen,
                    &mut else_aliases,
                    out,
                );
            }
        }
        StmtKind::Expr(expr) => {
            collect_access_control_state_vars_in_expr(hir, expr, seen, sender_aliases, out);
            update_sender_aliases_from_assignment(hir, expr, seen, sender_aliases);
        }
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            for stmt in block.stmts {
                collect_access_control_state_vars_in_stmt(hir, stmt, seen, sender_aliases, out);
            }
        }
        StmtKind::Try(try_stmt) => {
            collect_access_control_state_vars_in_expr(
                hir,
                &try_stmt.expr,
                seen,
                sender_aliases,
                out,
            );
            for clause in try_stmt.clauses {
                for stmt in clause.block.stmts {
                    collect_access_control_state_vars_in_stmt(hir, stmt, seen, sender_aliases, out);
                }
            }
        }
        StmtKind::Return(Some(expr)) | StmtKind::Emit(expr) | StmtKind::Revert(expr) => {
            collect_access_control_state_vars_in_expr(hir, expr, seen, sender_aliases, out);
        }
        StmtKind::DeclSingle(var_id) => {
            if let Some(init) = hir.variable(var_id).initializer {
                collect_access_control_state_vars_in_expr(hir, init, seen, sender_aliases, out);
            }
            update_sender_alias_from_decl(hir, var_id, seen, sender_aliases);
        }
        StmtKind::DeclMulti(_, expr) => {
            collect_access_control_state_vars_in_expr(hir, expr, seen, sender_aliases, out);
        }
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::AssemblyBlock(_)
        | StmtKind::Switch(_)
        | StmtKind::Err(_) => {}
    }
}

fn collect_access_control_state_vars_in_expr(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    seen: &mut HashSet<FunctionId>,
    sender_aliases: &HashSet<VariableId>,
    out: &mut HashSet<VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Call(callee, args, opts) if is_require_or_assert(callee) => {
            if let Some(cond) = args.exprs().next()
                && expr_looks_like_access_check(hir, cond, sender_aliases)
            {
                collect_access_check_state_vars(hir, cond, seen, out);
            }
            for arg in args.exprs() {
                collect_access_control_state_vars_in_expr(hir, arg, seen, sender_aliases, out);
            }
            if let Some(opts) = opts {
                for opt in opts.args {
                    collect_access_control_state_vars_in_expr(
                        hir,
                        &opt.value,
                        seen,
                        sender_aliases,
                        out,
                    );
                }
            }
        }
        ExprKind::Call(callee, args, opts) => {
            for callee_id in resolved_function_ids(callee) {
                let callee_func = hir.function(callee_id);
                if callee_func
                    .name
                    .is_some_and(|name| name_looks_like_access_control(name.as_str()))
                    || function_has_access_guard(hir, callee_id, &mut HashSet::new())
                {
                    collect_state_vars_read_in_function(hir, callee_id, seen, out);
                }
            }

            collect_access_control_state_vars_in_expr(hir, callee, seen, sender_aliases, out);
            if let Some(opts) = opts {
                for opt in opts.args {
                    collect_access_control_state_vars_in_expr(
                        hir,
                        &opt.value,
                        seen,
                        sender_aliases,
                        out,
                    );
                }
            }
            for arg in args.exprs() {
                collect_access_control_state_vars_in_expr(hir, arg, seen, sender_aliases, out);
            }
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            collect_access_control_state_vars_in_expr(hir, lhs, seen, sender_aliases, out);
            collect_access_control_state_vars_in_expr(hir, rhs, seen, sender_aliases, out);
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => {
            collect_access_control_state_vars_in_expr(hir, inner, seen, sender_aliases, out);
        }
        ExprKind::Index(base, index) => {
            collect_access_control_state_vars_in_expr(hir, base, seen, sender_aliases, out);
            if let Some(index) = index {
                collect_access_control_state_vars_in_expr(hir, index, seen, sender_aliases, out);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_access_control_state_vars_in_expr(hir, base, seen, sender_aliases, out);
            if let Some(start) = start {
                collect_access_control_state_vars_in_expr(hir, start, seen, sender_aliases, out);
            }
            if let Some(end) = end {
                collect_access_control_state_vars_in_expr(hir, end, seen, sender_aliases, out);
            }
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            collect_access_control_state_vars_in_expr(hir, cond, seen, sender_aliases, out);
            collect_access_control_state_vars_in_expr(hir, true_expr, seen, sender_aliases, out);
            collect_access_control_state_vars_in_expr(hir, false_expr, seen, sender_aliases, out);
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_access_control_state_vars_in_expr(hir, expr, seen, sender_aliases, out);
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_access_control_state_vars_in_expr(hir, expr, seen, sender_aliases, out);
            }
        }
        ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
        ExprKind::Ident(_) | ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => {}
    }
}

fn collect_access_check_state_vars(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    seen: &mut HashSet<FunctionId>,
    out: &mut HashSet<VariableId>,
) {
    collect_state_vars_read_in_expr(hir, expr, seen, out);

    for callee_id in called_function_ids(expr) {
        collect_state_vars_read_in_function(hir, callee_id, seen, out);
    }
}

fn collect_state_vars_read_in_function(
    hir: &hir::Hir<'_>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
    out: &mut HashSet<VariableId>,
) {
    if !seen.insert(func_id) {
        return;
    }

    let func = hir.function(func_id);
    let Some(body) = func.body else { return };
    for stmt in body.stmts {
        collect_state_vars_read_in_stmt(hir, stmt, seen, out);
    }
}

fn collect_state_vars_read_in_stmt(
    hir: &hir::Hir<'_>,
    stmt: &hir::Stmt<'_>,
    seen: &mut HashSet<FunctionId>,
    out: &mut HashSet<VariableId>,
) {
    match stmt.kind {
        StmtKind::DeclSingle(var_id) => {
            if let Some(init) = hir.variable(var_id).initializer {
                collect_state_vars_read_in_expr(hir, init, seen, out);
            }
        }
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Expr(expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr)
        | StmtKind::Return(Some(expr)) => collect_state_vars_read_in_expr(hir, expr, seen, out),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            for stmt in block.stmts {
                collect_state_vars_read_in_stmt(hir, stmt, seen, out);
            }
        }
        StmtKind::If(cond, then_stmt, else_stmt) => {
            collect_state_vars_read_in_expr(hir, cond, seen, out);
            collect_state_vars_read_in_stmt(hir, then_stmt, seen, out);
            if let Some(else_stmt) = else_stmt {
                collect_state_vars_read_in_stmt(hir, else_stmt, seen, out);
            }
        }
        StmtKind::Try(try_stmt) => {
            collect_state_vars_read_in_expr(hir, &try_stmt.expr, seen, out);
            for clause in try_stmt.clauses {
                for stmt in clause.block.stmts {
                    collect_state_vars_read_in_stmt(hir, stmt, seen, out);
                }
            }
        }
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::AssemblyBlock(_)
        | StmtKind::Switch(_)
        | StmtKind::Err(_) => {}
    }
}

fn collect_state_vars_read_in_expr(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    seen: &mut HashSet<FunctionId>,
    out: &mut HashSet<VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            for res in *reses {
                if let Res::Item(ItemId::Variable(var_id)) = res
                    && hir.variable(*var_id).kind.is_state()
                {
                    out.insert(*var_id);
                }
            }
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            collect_state_vars_read_in_expr(hir, lhs, seen, out);
            collect_state_vars_read_in_expr(hir, rhs, seen, out);
        }
        ExprKind::Call(callee, args, opts) => {
            for callee_id in resolved_function_ids(callee) {
                collect_state_vars_read_in_function(hir, callee_id, seen, out);
            }

            collect_state_vars_read_in_expr(hir, callee, seen, out);
            if let Some(opts) = opts {
                for opt in opts.args {
                    collect_state_vars_read_in_expr(hir, &opt.value, seen, out);
                }
            }
            for arg in args.exprs() {
                collect_state_vars_read_in_expr(hir, arg, seen, out);
            }
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => collect_state_vars_read_in_expr(hir, inner, seen, out),
        ExprKind::Index(base, index) => {
            collect_state_vars_read_in_expr(hir, base, seen, out);
            if let Some(index) = index {
                collect_state_vars_read_in_expr(hir, index, seen, out);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_state_vars_read_in_expr(hir, base, seen, out);
            if let Some(start) = start {
                collect_state_vars_read_in_expr(hir, start, seen, out);
            }
            if let Some(end) = end {
                collect_state_vars_read_in_expr(hir, end, seen, out);
            }
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            collect_state_vars_read_in_expr(hir, cond, seen, out);
            collect_state_vars_read_in_expr(hir, true_expr, seen, out);
            collect_state_vars_read_in_expr(hir, false_expr, seen, out);
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_state_vars_read_in_expr(hir, expr, seen, out);
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_state_vars_read_in_expr(hir, expr, seen, out);
            }
        }
        ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
        ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => {}
    }
}

#[derive(Clone, Debug, Default)]
struct WriteSources {
    inputs: HashSet<VariableId>,
    states: HashSet<VariableId>,
    reads_sender: bool,
}

impl WriteSources {
    fn input(var_id: VariableId) -> Self {
        Self { inputs: HashSet::from([var_id]), states: HashSet::new(), reads_sender: false }
    }

    fn state(var_id: VariableId) -> Self {
        Self { inputs: HashSet::new(), states: HashSet::from([var_id]), reads_sender: false }
    }

    fn sender() -> Self {
        Self { inputs: HashSet::new(), states: HashSet::new(), reads_sender: true }
    }

    fn is_empty(&self) -> bool {
        self.inputs.is_empty() && self.states.is_empty() && !self.reads_sender
    }

    fn extend(&mut self, other: Self) {
        self.inputs.extend(other.inputs);
        self.states.extend(other.states);
        self.reads_sender |= other.reads_sender;
    }

    fn intersects(&self, other: &Self) -> bool {
        self.reads_sender && other.reads_sender
            || self.inputs.iter().any(|var_id| other.inputs.contains(var_id))
            || self.states.iter().any(|var_id| other.states.contains(var_id))
    }
}

#[derive(Clone, Debug)]
struct StateWrite {
    var_id: VariableId,
    span: Span,
    sources: WriteSources,
    fixed_clear: bool,
    evented: bool,
}

#[derive(Clone)]
struct AnalyzerState {
    taint: HashMap<VariableId, WriteSources>,
    storage_aliases: HashMap<VariableId, VariableId>,
    writes: Vec<StateWrite>,
}

struct WriteAnalyzer<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    targets: &'a HashSet<VariableId>,
    guard_targets: &'a HashSet<VariableId>,
    taint: HashMap<VariableId, WriteSources>,
    storage_aliases: HashMap<VariableId, VariableId>,
    writes: Vec<StateWrite>,
    call_stack: Vec<FunctionId>,
}

impl<'a, 'hir> WriteAnalyzer<'a, 'hir> {
    fn new(
        hir: &'hir hir::Hir<'hir>,
        targets: &'a HashSet<VariableId>,
        guard_targets: &'a HashSet<VariableId>,
    ) -> Self {
        Self {
            hir,
            targets,
            guard_targets,
            taint: HashMap::new(),
            storage_aliases: HashMap::new(),
            writes: Vec::new(),
            call_stack: Vec::new(),
        }
    }

    fn analyze_entry_point(&mut self, func_id: FunctionId) -> Vec<StateWrite> {
        let func = self.hir.function(func_id);
        self.taint.clear();
        self.storage_aliases.clear();
        for &param in func.parameters {
            self.taint.insert(param, WriteSources::input(param));
        }

        self.analyze_function(func_id);
        std::mem::take(&mut self.writes)
    }

    fn analyze_function(&mut self, func_id: FunctionId) {
        if self.call_stack.contains(&func_id) {
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        self.call_stack.push(func_id);
        self.analyze_modifiers(func);
        for stmt in body.stmts {
            self.analyze_stmt(stmt);
        }
        self.call_stack.pop();
    }

    fn analyze_modifiers(&mut self, func: &'hir hir::Function<'hir>) {
        for modifier in func.modifiers {
            let Some(modifier_id) = modifier.id.as_function() else { continue };
            if self.call_stack.contains(&modifier_id) {
                continue;
            }

            for arg in modifier.args.exprs() {
                self.analyze_expr(arg);
            }

            let modifier_func = self.hir.function(modifier_id);
            let Some(body) = modifier_func.body else { continue };
            let saved_taint = self.taint.clone();
            let saved_storage_aliases = self.storage_aliases.clone();

            for (param, arg) in modifier_func.parameters.iter().copied().zip(modifier.args.exprs())
            {
                self.set_local_taint(param, self.value_sources(arg));
            }

            self.call_stack.push(modifier_id);
            for stmt in body.stmts {
                self.analyze_stmt(stmt);
            }
            self.call_stack.pop();

            self.taint = saved_taint;
            self.storage_aliases = saved_storage_aliases;
        }
    }

    fn analyze_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                let var = self.hir.variable(var_id);
                if let Some(init) = var.initializer
                    && !var.kind.is_state()
                {
                    self.analyze_expr(init);
                    self.set_local_taint(var_id, self.value_sources(init));
                    self.set_storage_alias_from_initializer(var_id, init);
                }
            }
            StmtKind::DeclMulti(vars, expr) => {
                self.analyze_expr(expr);
                let sources = self.value_sources(expr);
                for var_id in vars.iter().flatten().copied() {
                    if !self.hir.variable(var_id).kind.is_state() {
                        self.set_local_taint(var_id, sources.clone());
                    }
                }
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                for stmt in block.stmts {
                    self.analyze_stmt(stmt);
                }
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.analyze_expr(cond);
                let base = self.capture_state();

                self.analyze_stmt(then_stmt);
                let then_state = self.capture_state();

                self.restore_state(base.clone());
                if let Some(else_stmt) = else_stmt {
                    self.analyze_stmt(else_stmt);
                }
                let else_state = self.capture_state();

                let then_exits = stmt_exits_or_reverts(then_stmt);
                let else_exits = else_stmt.is_some_and(stmt_exits_or_reverts);
                self.restore_state(self.merge_branch_states(
                    base,
                    then_state,
                    else_state,
                    then_exits,
                    else_exits,
                    else_stmt.is_some(),
                ));
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr);
                for clause in try_stmt.clauses {
                    for stmt in clause.block.stmts {
                        self.analyze_stmt(stmt);
                    }
                }
            }
            StmtKind::Expr(expr) | StmtKind::Revert(expr) => {
                self.analyze_expr(expr);
            }
            StmtKind::Emit(expr) => {
                self.analyze_expr(expr);
                self.mark_event(expr);
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.analyze_expr(expr);
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

    fn analyze_expr(&mut self, expr: &'hir hir::Expr<'hir>) {
        match &expr.peel_parens().kind {
            ExprKind::Assign(lhs, op, rhs) => {
                self.analyze_expr(rhs);

                let rhs_sources = self.value_sources(rhs);
                let mut write_sources = rhs_sources.clone();
                write_sources.extend(self.lhs_index_sources(lhs));
                if op.is_some() {
                    write_sources.extend(self.value_sources(lhs));
                }

                let fixed_clear = expr_is_zero_value(rhs);
                for var_id in state_lhs_vars(self.hir, lhs, &self.storage_aliases) {
                    if self.targets.contains(&var_id)
                        && self.write_is_reportable(var_id, &write_sources, fixed_clear)
                    {
                        self.writes.push(StateWrite {
                            var_id,
                            span: lhs.span,
                            sources: write_sources.clone(),
                            fixed_clear,
                            evented: false,
                        });
                    }
                }

                if let Some(local) = lhs_local_var(self.hir, lhs) {
                    let mut local_sources = rhs_sources;
                    if op.is_some() {
                        local_sources.extend(self.value_sources(lhs));
                    }
                    self.set_local_taint(local, local_sources);
                    self.set_storage_alias_from_initializer(local, rhs);
                } else {
                    self.analyze_lhs_indices(lhs);
                }
            }
            ExprKind::Delete(inner) => {
                let sources = self.lhs_index_sources(inner);
                for var_id in state_lhs_vars(self.hir, inner, &self.storage_aliases) {
                    if self.targets.contains(&var_id)
                        && self.write_is_reportable(var_id, &sources, true)
                    {
                        self.writes.push(StateWrite {
                            var_id,
                            span: inner.span,
                            sources: sources.clone(),
                            fixed_clear: true,
                            evented: false,
                        });
                    }
                }
                self.analyze_lhs_indices(inner);
            }
            ExprKind::Call(callee, args, opts) => {
                self.analyze_expr(callee);
                if let Some(opts) = opts {
                    for opt in opts.args {
                        self.analyze_expr(&opt.value);
                    }
                }
                for arg in args.exprs() {
                    self.analyze_expr(arg);
                }

                for callee_id in resolved_function_ids(callee) {
                    self.analyze_internal_call(callee_id, args);
                }
            }
            ExprKind::Binary(lhs, _, rhs) => {
                self.analyze_expr(lhs);
                self.analyze_expr(rhs);
            }
            ExprKind::Unary(_, inner) | ExprKind::Member(inner, _) | ExprKind::Payable(inner) => {
                self.analyze_expr(inner);
            }
            ExprKind::Index(base, index) => {
                self.analyze_expr(base);
                if let Some(index) = index {
                    self.analyze_expr(index);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_expr(base);
                if let Some(start) = start {
                    self.analyze_expr(start);
                }
                if let Some(end) = end {
                    self.analyze_expr(end);
                }
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                self.analyze_expr(cond);
                self.analyze_expr(true_expr);
                self.analyze_expr(false_expr);
            }
            ExprKind::Array(exprs) => {
                for expr in *exprs {
                    self.analyze_expr(expr);
                }
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().copied().flatten() {
                    self.analyze_expr(expr);
                }
            }
            ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
            ExprKind::Ident(_) | ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => {}
        }
    }

    fn analyze_internal_call(&mut self, callee_id: FunctionId, args: &hir::CallArgs<'hir>) {
        if self.call_stack.contains(&callee_id) {
            return;
        }

        let callee = self.hir.function(callee_id);
        let Some(_) = callee.body else { return };

        let saved_taint = std::mem::take(&mut self.taint);
        let saved_storage_aliases = std::mem::take(&mut self.storage_aliases);
        for (param, arg) in callee.parameters.iter().copied().zip(args.exprs()) {
            let sources = collect_value_sources(self.hir, &saved_taint, arg);
            if !sources.is_empty() {
                self.taint.insert(param, sources);
            }
        }

        self.analyze_function(callee_id);
        self.taint = saved_taint;
        self.storage_aliases = saved_storage_aliases;
    }

    fn analyze_lhs_indices(&mut self, expr: &'hir hir::Expr<'hir>) {
        match &expr.peel_parens().kind {
            ExprKind::Index(base, index) => {
                self.analyze_lhs_indices(base);
                if let Some(index) = index {
                    self.analyze_expr(index);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_lhs_indices(base);
                if let Some(start) = start {
                    self.analyze_expr(start);
                }
                if let Some(end) = end {
                    self.analyze_expr(end);
                }
            }
            ExprKind::Member(base, _) | ExprKind::Payable(base) => self.analyze_lhs_indices(base),
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().copied().flatten() {
                    self.analyze_lhs_indices(expr);
                }
            }
            _ => {}
        }
    }

    fn value_sources(&self, expr: &hir::Expr<'_>) -> WriteSources {
        collect_value_sources(self.hir, &self.taint, expr)
    }

    fn lhs_index_sources(&self, expr: &hir::Expr<'_>) -> WriteSources {
        collect_lhs_index_sources(self.hir, &self.taint, expr)
    }

    fn set_local_taint(&mut self, var_id: VariableId, sources: WriteSources) {
        if sources.is_empty() {
            self.taint.remove(&var_id);
        } else {
            self.taint.insert(var_id, sources);
        }
    }

    fn set_storage_alias_from_initializer(
        &mut self,
        var_id: VariableId,
        init: &'hir hir::Expr<'hir>,
    ) {
        let var = self.hir.variable(var_id);
        if var.kind.is_state() || var.data_location != Some(DataLocation::Storage) {
            self.storage_aliases.remove(&var_id);
            return;
        }

        if let Some(root) = state_lhs_vars(self.hir, init, &self.storage_aliases).into_iter().next()
        {
            self.storage_aliases.insert(var_id, root);
        } else {
            self.storage_aliases.remove(&var_id);
        }
    }

    fn mark_event(&mut self, expr: &hir::Expr<'_>) {
        let Some(event_id) = emitted_event_id(expr) else { return };
        let event_sources = self.value_sources(expr);

        for write in &mut self.writes {
            if !write.evented
                && (write.fixed_clear || write.sources.intersects(&event_sources))
                && event_mentions_state_var(self.hir, event_id, write.var_id)
            {
                write.evented = true;
            }
        }
    }

    fn capture_state(&self) -> AnalyzerState {
        AnalyzerState {
            taint: self.taint.clone(),
            storage_aliases: self.storage_aliases.clone(),
            writes: self.writes.clone(),
        }
    }

    fn restore_state(&mut self, state: AnalyzerState) {
        self.taint = state.taint;
        self.storage_aliases = state.storage_aliases;
        self.writes = state.writes;
    }

    fn merge_branch_states(
        &self,
        base: AnalyzerState,
        then_state: AnalyzerState,
        else_state: AnalyzerState,
        then_exits: bool,
        else_exits: bool,
        has_else: bool,
    ) -> AnalyzerState {
        let mut writes = base.writes.clone();
        let base_len = writes.len();
        for (idx, write) in writes.iter_mut().enumerate().take(base_len) {
            write.evented = then_state.writes[idx].evented && else_state.writes[idx].evented;
        }
        writes.extend_from_slice(&then_state.writes[base_len..]);
        writes.extend_from_slice(&else_state.writes[base_len..]);

        let (taint, storage_aliases) = match (then_exits, else_exits, has_else) {
            (true, true, _) => (base.taint, base.storage_aliases),
            (true, _, _) => (else_state.taint, else_state.storage_aliases),
            (_, true, true) => (then_state.taint, then_state.storage_aliases),
            _ => (
                merge_source_maps(&then_state.taint, &else_state.taint),
                merge_alias_maps(&then_state.storage_aliases, &else_state.storage_aliases),
            ),
        };

        AnalyzerState { taint, storage_aliases, writes }
    }

    fn write_is_reportable(
        &self,
        var_id: VariableId,
        sources: &WriteSources,
        fixed_clear: bool,
    ) -> bool {
        self.targets.contains(&var_id)
            && (!sources.is_empty() || (fixed_clear && self.guard_targets.contains(&var_id)))
    }
}

fn merge_source_maps(
    lhs: &HashMap<VariableId, WriteSources>,
    rhs: &HashMap<VariableId, WriteSources>,
) -> HashMap<VariableId, WriteSources> {
    let mut out = lhs.clone();
    for (var_id, sources) in rhs {
        out.entry(*var_id).or_default().extend(sources.clone());
    }
    out.retain(|_, sources| !sources.is_empty());
    out
}

fn merge_alias_maps(
    lhs: &HashMap<VariableId, VariableId>,
    rhs: &HashMap<VariableId, VariableId>,
) -> HashMap<VariableId, VariableId> {
    lhs.iter()
        .filter_map(|(alias, lhs_root)| {
            rhs.get(alias)
                .is_some_and(|rhs_root| rhs_root == lhs_root)
                .then_some((*alias, *lhs_root))
        })
        .collect()
}

fn collect_value_sources(
    hir: &hir::Hir<'_>,
    taint: &HashMap<VariableId, WriteSources>,
    expr: &hir::Expr<'_>,
) -> WriteSources {
    let mut out = WriteSources::default();
    collect_value_sources_into(hir, taint, expr, &mut out);
    out
}

fn collect_value_sources_into(
    hir: &hir::Hir<'_>,
    taint: &HashMap<VariableId, WriteSources>,
    expr: &hir::Expr<'_>,
    out: &mut WriteSources,
) {
    if is_sender_member(expr) {
        out.extend(WriteSources::sender());
        return;
    }

    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            for res in *reses {
                if let Res::Item(ItemId::Variable(var_id)) = res {
                    if hir.variable(*var_id).kind.is_state() {
                        out.extend(WriteSources::state(*var_id));
                    }
                    if let Some(sources) = taint.get(var_id) {
                        out.extend(sources.clone());
                    }
                }
            }
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            collect_value_sources_into(hir, taint, lhs, out);
            collect_value_sources_into(hir, taint, rhs, out);
        }
        ExprKind::Call(callee, args, opts) => {
            collect_value_sources_into(hir, taint, callee, out);
            if let Some(opts) = opts {
                for opt in opts.args {
                    collect_value_sources_into(hir, taint, &opt.value, out);
                }
            }
            for arg in args.exprs() {
                collect_value_sources_into(hir, taint, arg, out);
            }
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => collect_value_sources_into(hir, taint, inner, out),
        ExprKind::Index(base, index) => {
            collect_value_sources_into(hir, taint, base, out);
            if let Some(index) = index {
                collect_value_sources_into(hir, taint, index, out);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_value_sources_into(hir, taint, base, out);
            if let Some(start) = start {
                collect_value_sources_into(hir, taint, start, out);
            }
            if let Some(end) = end {
                collect_value_sources_into(hir, taint, end, out);
            }
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            collect_value_sources_into(hir, taint, cond, out);
            collect_value_sources_into(hir, taint, true_expr, out);
            collect_value_sources_into(hir, taint, false_expr, out);
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_value_sources_into(hir, taint, expr, out);
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_value_sources_into(hir, taint, expr, out);
            }
        }
        ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
        ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => {}
    }
}

fn collect_lhs_index_sources(
    hir: &hir::Hir<'_>,
    taint: &HashMap<VariableId, WriteSources>,
    expr: &hir::Expr<'_>,
) -> WriteSources {
    let mut out = WriteSources::default();
    collect_lhs_index_sources_into(hir, taint, expr, &mut out);
    out
}

fn collect_lhs_index_sources_into(
    hir: &hir::Hir<'_>,
    taint: &HashMap<VariableId, WriteSources>,
    expr: &hir::Expr<'_>,
    out: &mut WriteSources,
) {
    match &expr.peel_parens().kind {
        ExprKind::Index(base, index) => {
            collect_lhs_index_sources_into(hir, taint, base, out);
            if let Some(index) = index {
                out.extend(collect_value_sources(hir, taint, index));
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_lhs_index_sources_into(hir, taint, base, out);
            if let Some(start) = start {
                out.extend(collect_value_sources(hir, taint, start));
            }
            if let Some(end) = end {
                out.extend(collect_value_sources(hir, taint, end));
            }
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) | ExprKind::Unary(_, base) => {
            collect_lhs_index_sources_into(hir, taint, base, out);
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_lhs_index_sources_into(hir, taint, expr, out);
            }
        }
        _ => {}
    }
}

fn expr_is_zero_value(expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => match lit.kind {
            LitKind::Number(value) => value.is_zero(),
            LitKind::Address(value) => value.is_zero(),
            LitKind::Bool(value) => !value,
            _ => false,
        },
        ExprKind::Call(_, args, _) => {
            let mut exprs = args.exprs();
            let Some(arg) = exprs.next() else { return false };
            exprs.next().is_none() && expr_is_zero_value(arg)
        }
        ExprKind::Unary(_, inner) | ExprKind::Payable(inner) => expr_is_zero_value(inner),
        _ => false,
    }
}

fn lhs_local_var(hir: &hir::Hir<'_>, lhs: &hir::Expr<'_>) -> Option<VariableId> {
    if let ExprKind::Ident(reses) = &lhs.peel_parens().kind {
        for res in *reses {
            if let Res::Item(ItemId::Variable(var_id)) = res
                && !hir.variable(*var_id).kind.is_state()
            {
                return Some(*var_id);
            }
        }
    }
    None
}

fn state_lhs_vars(
    hir: &hir::Hir<'_>,
    lhs: &hir::Expr<'_>,
    storage_aliases: &HashMap<VariableId, VariableId>,
) -> Vec<VariableId> {
    let mut vars = Vec::new();
    collect_state_lhs_vars(hir, lhs, storage_aliases, &mut vars);
    vars
}

fn collect_state_lhs_vars(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    storage_aliases: &HashMap<VariableId, VariableId>,
    vars: &mut Vec<VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            for res in *reses {
                if let Res::Item(ItemId::Variable(var_id)) = res {
                    let root = if hir.variable(*var_id).kind.is_state() {
                        Some(*var_id)
                    } else {
                        storage_aliases.get(var_id).copied()
                    };
                    if let Some(root) = root
                        && !vars.contains(&root)
                    {
                        vars.push(root);
                    }
                }
            }
        }
        ExprKind::Index(base, _) | ExprKind::Slice(base, ..) => {
            collect_state_lhs_vars(hir, base, storage_aliases, vars);
        }
        ExprKind::Member(base, _)
        | ExprKind::Payable(base)
        | ExprKind::Unary(_, base)
        | ExprKind::Delete(base) => collect_state_lhs_vars(hir, base, storage_aliases, vars),
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_state_lhs_vars(hir, expr, storage_aliases, vars);
            }
        }
        _ => {}
    }
}

fn is_protected(hir: &hir::Hir<'_>, func_id: FunctionId, func: &hir::Function<'_>) -> bool {
    for modifier in func.modifiers {
        if let Some(modifier_id) = modifier.id.as_function()
            && modifier_has_access_control(hir, modifier_id)
        {
            return true;
        }
    }

    function_has_access_guard(hir, func_id, &mut HashSet::new())
}

fn update_sender_alias_from_decl(
    hir: &hir::Hir<'_>,
    var_id: VariableId,
    seen: &HashSet<FunctionId>,
    sender_aliases: &mut HashSet<VariableId>,
) {
    let var = hir.variable(var_id);
    if var.kind.is_state() {
        return;
    }

    let mut sender_seen = seen.clone();
    if var
        .initializer
        .is_some_and(|init| expr_reads_sender(hir, init, &mut sender_seen, sender_aliases))
    {
        sender_aliases.insert(var_id);
    } else {
        sender_aliases.remove(&var_id);
    }
}

fn update_sender_aliases_from_assignment(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    seen: &HashSet<FunctionId>,
    sender_aliases: &mut HashSet<VariableId>,
) {
    let ExprKind::Assign(lhs, _, rhs) = &expr.peel_parens().kind else { return };
    let Some(local) = lhs_local_var(hir, lhs) else { return };

    let mut sender_seen = seen.clone();
    if expr_reads_sender(hir, rhs, &mut sender_seen, sender_aliases) {
        sender_aliases.insert(local);
    } else {
        sender_aliases.remove(&local);
    }
}

fn modifier_has_access_control(hir: &hir::Hir<'_>, modifier_id: FunctionId) -> bool {
    let modifier = hir.function(modifier_id);
    if let Some(body) = modifier.body {
        let mut sender_aliases = HashSet::new();
        for stmt in body.stmts {
            if matches!(stmt.kind, StmtKind::Placeholder) {
                break;
            }
            if stmt_has_access_guard(hir, stmt, &mut HashSet::new(), &mut sender_aliases) {
                return true;
            }
        }
        return false;
    }

    modifier.name.is_some_and(|name| name_looks_like_access_control(name.as_str()))
}

fn function_has_access_guard(
    hir: &hir::Hir<'_>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    if !seen.insert(func_id) {
        return false;
    }

    let func = hir.function(func_id);
    let Some(body) = func.body else {
        return func.name.is_some_and(|name| name_looks_like_access_control(name.as_str()));
    };

    let mut sender_aliases = HashSet::new();
    for stmt in body.stmts {
        if stmt_has_access_guard(hir, stmt, seen, &mut sender_aliases) {
            return true;
        }
    }
    false
}

fn stmt_has_access_guard(
    hir: &hir::Hir<'_>,
    stmt: &hir::Stmt<'_>,
    seen: &mut HashSet<FunctionId>,
    sender_aliases: &mut HashSet<VariableId>,
) -> bool {
    match stmt.kind {
        StmtKind::If(cond, then_stmt, else_stmt) => {
            (expr_looks_like_access_check(hir, cond, sender_aliases)
                && (stmt_exits_or_reverts(then_stmt)
                    || else_stmt.is_some_and(stmt_exits_or_reverts)))
                || {
                    let mut then_aliases = sender_aliases.clone();
                    stmt_has_access_guard(hir, then_stmt, seen, &mut then_aliases)
                }
                || else_stmt.is_some_and(|stmt| {
                    let mut else_aliases = sender_aliases.clone();
                    stmt_has_access_guard(hir, stmt, seen, &mut else_aliases)
                })
        }
        StmtKind::Expr(expr) => {
            let has_guard = expr_has_access_guard(hir, expr, seen, sender_aliases);
            update_sender_aliases_from_assignment(hir, expr, seen, sender_aliases);
            has_guard
        }
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            for stmt in block.stmts {
                if stmt_has_access_guard(hir, stmt, seen, sender_aliases) {
                    return true;
                }
            }
            false
        }
        StmtKind::Try(try_stmt) => try_stmt.clauses.iter().any(|clause| {
            let mut clause_aliases = sender_aliases.clone();
            clause
                .block
                .stmts
                .iter()
                .any(|stmt| stmt_has_access_guard(hir, stmt, seen, &mut clause_aliases))
        }),
        StmtKind::Return(Some(expr)) | StmtKind::Emit(expr) | StmtKind::Revert(expr) => {
            expr_has_access_guard(hir, expr, seen, sender_aliases)
        }
        StmtKind::DeclSingle(var_id) => {
            let has_guard = hir
                .variable(var_id)
                .initializer
                .is_some_and(|init| expr_has_access_guard(hir, init, seen, sender_aliases));
            update_sender_alias_from_decl(hir, var_id, seen, sender_aliases);
            has_guard
        }
        StmtKind::DeclMulti(_, expr) => expr_has_access_guard(hir, expr, seen, sender_aliases),
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::AssemblyBlock(_)
        | StmtKind::Switch(_)
        | StmtKind::Err(_) => false,
    }
}

fn stmt_exits_or_reverts(stmt: &hir::Stmt<'_>) -> bool {
    branch_always_exits(stmt)
}

fn expr_has_access_guard(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    seen: &mut HashSet<FunctionId>,
    sender_aliases: &HashSet<VariableId>,
) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => args
            .exprs()
            .next()
            .is_some_and(|cond| expr_looks_like_access_check(hir, cond, sender_aliases)),
        ExprKind::Call(callee, args, opts) => {
            for callee_id in resolved_function_ids(callee) {
                let func = hir.function(callee_id);
                if func.name.is_some_and(|name| name_looks_like_access_control(name.as_str()))
                    || function_has_access_guard(hir, callee_id, seen)
                {
                    return true;
                }
            }

            expr_has_access_guard(hir, callee, seen, sender_aliases)
                || opts.is_some_and(|opts| {
                    opts.args
                        .iter()
                        .any(|opt| expr_has_access_guard(hir, &opt.value, seen, sender_aliases))
                })
                || args.exprs().any(|arg| expr_has_access_guard(hir, arg, seen, sender_aliases))
        }
        ExprKind::Binary(lhs, _, rhs) => {
            expr_has_access_guard(hir, lhs, seen, sender_aliases)
                || expr_has_access_guard(hir, rhs, seen, sender_aliases)
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => expr_has_access_guard(hir, inner, seen, sender_aliases),
        ExprKind::Index(base, index) => {
            expr_has_access_guard(hir, base, seen, sender_aliases)
                || index
                    .is_some_and(|index| expr_has_access_guard(hir, index, seen, sender_aliases))
        }
        ExprKind::Slice(base, start, end) => {
            expr_has_access_guard(hir, base, seen, sender_aliases)
                || start
                    .is_some_and(|start| expr_has_access_guard(hir, start, seen, sender_aliases))
                || end.is_some_and(|end| expr_has_access_guard(hir, end, seen, sender_aliases))
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            expr_has_access_guard(hir, cond, seen, sender_aliases)
                || expr_has_access_guard(hir, true_expr, seen, sender_aliases)
                || expr_has_access_guard(hir, false_expr, seen, sender_aliases)
        }
        ExprKind::Array(exprs) => {
            exprs.iter().any(|expr| expr_has_access_guard(hir, expr, seen, sender_aliases))
        }
        ExprKind::Tuple(exprs) => exprs
            .iter()
            .copied()
            .flatten()
            .any(|expr| expr_has_access_guard(hir, expr, seen, sender_aliases)),
        ExprKind::Assign(_, _, rhs) => expr_has_access_guard(hir, rhs, seen, sender_aliases),
        _ => false,
    }
}

fn expr_looks_like_access_check(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    sender_aliases: &HashSet<VariableId>,
) -> bool {
    expr_reads_sender(hir, expr, &mut HashSet::new(), sender_aliases)
        && expr_reads_state_variable_transitively(hir, expr)
}

fn expr_reads_state_variable_transitively(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let mut vars = HashSet::new();
    collect_state_vars_read_in_expr(hir, expr, &mut HashSet::new(), &mut vars);
    !vars.is_empty()
}

fn expr_reads_sender(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    seen: &mut HashSet<FunctionId>,
    sender_aliases: &HashSet<VariableId>,
) -> bool {
    if is_sender_member(expr) {
        return true;
    }

    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().any(|res| {
            let Res::Item(ItemId::Variable(var_id)) = res else { return false };
            sender_aliases.contains(var_id)
        }),
        ExprKind::Call(callee, args, opts) => {
            for callee_id in resolved_function_ids(callee) {
                if function_reads_sender(hir, callee_id, seen) {
                    return true;
                }
            }

            expr_reads_sender(hir, callee, seen, sender_aliases)
                || opts.is_some_and(|opts| {
                    opts.args
                        .iter()
                        .any(|opt| expr_reads_sender(hir, &opt.value, seen, sender_aliases))
                })
                || args.exprs().any(|arg| expr_reads_sender(hir, arg, seen, sender_aliases))
        }
        ExprKind::Binary(lhs, _, rhs) => {
            expr_reads_sender(hir, lhs, seen, sender_aliases)
                || expr_reads_sender(hir, rhs, seen, sender_aliases)
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => expr_reads_sender(hir, inner, seen, sender_aliases),
        ExprKind::Index(base, index) => {
            expr_reads_sender(hir, base, seen, sender_aliases)
                || index.is_some_and(|index| expr_reads_sender(hir, index, seen, sender_aliases))
        }
        ExprKind::Slice(base, start, end) => {
            expr_reads_sender(hir, base, seen, sender_aliases)
                || start.is_some_and(|start| expr_reads_sender(hir, start, seen, sender_aliases))
                || end.is_some_and(|end| expr_reads_sender(hir, end, seen, sender_aliases))
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            expr_reads_sender(hir, cond, seen, sender_aliases)
                || expr_reads_sender(hir, true_expr, seen, sender_aliases)
                || expr_reads_sender(hir, false_expr, seen, sender_aliases)
        }
        ExprKind::Array(exprs) => {
            exprs.iter().any(|expr| expr_reads_sender(hir, expr, seen, sender_aliases))
        }
        ExprKind::Tuple(exprs) => exprs
            .iter()
            .copied()
            .flatten()
            .any(|expr| expr_reads_sender(hir, expr, seen, sender_aliases)),
        ExprKind::Assign(_, _, rhs) => expr_reads_sender(hir, rhs, seen, sender_aliases),
        _ => false,
    }
}

fn function_reads_sender(
    hir: &hir::Hir<'_>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    if !seen.insert(func_id) {
        return false;
    }

    let func = hir.function(func_id);
    let Some(body) = func.body else { return false };
    let mut sender_aliases = HashSet::new();
    body.stmts.iter().any(|stmt| stmt_reads_sender(hir, stmt, seen, &mut sender_aliases))
}

fn stmt_reads_sender(
    hir: &hir::Hir<'_>,
    stmt: &hir::Stmt<'_>,
    seen: &mut HashSet<FunctionId>,
    sender_aliases: &mut HashSet<VariableId>,
) -> bool {
    match stmt.kind {
        StmtKind::DeclSingle(var_id) => {
            let reads = hir
                .variable(var_id)
                .initializer
                .is_some_and(|init| expr_reads_sender(hir, init, seen, sender_aliases));
            update_sender_alias_from_decl(hir, var_id, seen, sender_aliases);
            reads
        }
        StmtKind::Expr(expr) => {
            let reads = expr_reads_sender(hir, expr, seen, sender_aliases);
            update_sender_aliases_from_assignment(hir, expr, seen, sender_aliases);
            reads
        }
        StmtKind::DeclMulti(_, expr) | StmtKind::Emit(expr) | StmtKind::Revert(expr) => {
            expr_reads_sender(hir, expr, seen, sender_aliases)
        }
        StmtKind::Return(Some(expr)) => expr_reads_sender(hir, expr, seen, sender_aliases),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            block.stmts.iter().any(|stmt| stmt_reads_sender(hir, stmt, seen, sender_aliases))
        }
        StmtKind::If(cond, then_stmt, else_stmt) => {
            expr_reads_sender(hir, cond, seen, sender_aliases)
                || {
                    let mut then_aliases = sender_aliases.clone();
                    stmt_reads_sender(hir, then_stmt, seen, &mut then_aliases)
                }
                || else_stmt.is_some_and(|stmt| {
                    let mut else_aliases = sender_aliases.clone();
                    stmt_reads_sender(hir, stmt, seen, &mut else_aliases)
                })
        }
        StmtKind::Try(try_stmt) => {
            expr_reads_sender(hir, &try_stmt.expr, seen, sender_aliases)
                || try_stmt.clauses.iter().any(|clause| {
                    let mut clause_aliases = sender_aliases.clone();
                    clause
                        .block
                        .stmts
                        .iter()
                        .any(|stmt| stmt_reads_sender(hir, stmt, seen, &mut clause_aliases))
                })
        }
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::AssemblyBlock(_)
        | StmtKind::Switch(_)
        | StmtKind::Err(_) => false,
    }
}

fn is_sender_member(expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Member(base, member) = &expr.peel_parens().kind else { return false };
    let ExprKind::Ident(reses) = &base.peel_parens().kind else { return false };

    reses.iter().any(|res| {
        let Res::Builtin(builtin) = res else { return false };
        matches!((builtin.name(), member.name), (sym::msg, sym::sender) | (sym::tx, kw::Origin))
    })
}

fn name_looks_like_access_control(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "auth"
        || lower == "requiresauth"
        || lower == "restricted"
        || lower.starts_with("onlyadmin")
        || lower.starts_with("onlyguardian")
        || lower.starts_with("onlymanager")
        || lower.starts_with("onlyowner")
        || lower.starts_with("onlyrole")
        || lower.starts_with("checkadmin")
        || lower.starts_with("_checkadmin")
        || lower.starts_with("checkguardian")
        || lower.starts_with("_checkguardian")
        || lower.starts_with("checkmanager")
        || lower.starts_with("_checkmanager")
        || lower.starts_with("checkowner")
        || lower.starts_with("_checkowner")
        || lower.starts_with("checkrole")
        || lower.starts_with("_checkrole")
}

fn emitted_event_id(expr: &hir::Expr<'_>) -> Option<EventId> {
    let ExprKind::Call(callee, _, _) = &expr.peel_parens().kind else { return None };
    resolved_event_id(callee)
}

fn resolved_event_id(expr: &hir::Expr<'_>) -> Option<EventId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|res| {
            let Res::Item(ItemId::Event(event_id)) = res else { return None };
            Some(*event_id)
        }),
        ExprKind::Member(base, _) => resolved_event_id(base),
        _ => None,
    }
}

fn event_mentions_state_var(hir: &hir::Hir<'_>, event_id: EventId, var_id: VariableId) -> bool {
    let Some(var_name) = hir.variable(var_id).name else { return false };
    let keywords = state_var_event_keywords(var_name.as_str());
    let event = hir.event(event_id);

    name_contains_event_keyword(event.name.as_str(), &keywords)
        || event.parameters.iter().any(|param_id| {
            hir.variable(*param_id)
                .name
                .is_some_and(|name| name_contains_event_keyword(name.as_str(), &keywords))
        })
}

fn state_var_event_keywords(name: &str) -> Vec<String> {
    let normalized = normalize_event_name_part(name);
    let mut out = Vec::from([normalized.clone()]);

    if let Some(singular) = normalized.strip_suffix('s')
        && !singular.is_empty()
    {
        out.push(singular.to_string());
    }

    for keyword in ["owner", "admin", "guardian", "manager", "role"] {
        if normalized.contains(keyword) {
            out.push(keyword.to_string());
        }
    }

    out
}

fn name_contains_event_keyword(name: &str, keywords: &[String]) -> bool {
    let normalized = normalize_event_name_part(name);
    keywords.iter().any(|keyword| !keyword.is_empty() && normalized.contains(keyword))
}

fn normalize_event_name_part(name: &str) -> String {
    name.chars().filter(|ch| ch.is_ascii_alphanumeric()).map(|ch| ch.to_ascii_lowercase()).collect()
}

fn called_function_ids(expr: &hir::Expr<'_>) -> HashSet<FunctionId> {
    let mut out = HashSet::new();
    collect_called_function_ids(expr, &mut out);
    out
}

fn collect_called_function_ids(expr: &hir::Expr<'_>, out: &mut HashSet<FunctionId>) {
    match &expr.peel_parens().kind {
        ExprKind::Call(callee, args, opts) => {
            out.extend(resolved_function_ids(callee));
            collect_called_function_ids(callee, out);
            if let Some(opts) = opts {
                for opt in opts.args {
                    collect_called_function_ids(&opt.value, out);
                }
            }
            for arg in args.exprs() {
                collect_called_function_ids(arg, out);
            }
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            collect_called_function_ids(lhs, out);
            collect_called_function_ids(rhs, out);
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => collect_called_function_ids(inner, out),
        ExprKind::Index(base, index) => {
            collect_called_function_ids(base, out);
            if let Some(index) = index {
                collect_called_function_ids(index, out);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_called_function_ids(base, out);
            if let Some(start) = start {
                collect_called_function_ids(start, out);
            }
            if let Some(end) = end {
                collect_called_function_ids(end, out);
            }
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            collect_called_function_ids(cond, out);
            collect_called_function_ids(true_expr, out);
            collect_called_function_ids(false_expr, out);
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_called_function_ids(expr, out);
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_called_function_ids(expr, out);
            }
        }
        ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::YulMember(..)
        | ExprKind::Err(_) => {}
    }
}

fn resolved_function_ids<'hir>(
    callee: &'hir hir::Expr<'hir>,
) -> impl Iterator<Item = FunctionId> + 'hir {
    let reses = match &callee.peel_parens().kind {
        ExprKind::Ident(reses) => *reses,
        _ => &[],
    };
    reses.iter().filter_map(|res| match res {
        Res::Item(ItemId::Function(func_id)) => Some(*func_id),
        _ => None,
    })
}
