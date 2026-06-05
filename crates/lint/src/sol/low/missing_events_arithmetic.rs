use super::MissingEventsArithmetic;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::primitives::{branch_always_exits, is_require_or_assert},
    },
};
use solar::{
    ast::{ContractKind, StateMutability, Visibility},
    interface::{Span, kw, sym},
    sema::hir::{
        self, BinOpKind, ElementaryType, ExprKind, FunctionId, ItemId, Res, StmtKind, TypeKind,
        UnOpKind, VariableId,
    },
};
use std::collections::{HashMap, HashSet};

declare_forge_lint!(
    MISSING_EVENTS_ARITHMETIC,
    Severity::Low,
    "missing-events-arithmetic",
    "critical arithmetic state changes should emit events"
);

impl<'hir> LateLintPass<'hir> for MissingEventsArithmetic {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        _gcx: solar::sema::Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        contract: &'hir hir::Contract<'hir>,
    ) {
        if contract.kind != ContractKind::Contract {
            return;
        }

        let candidate_vars: HashSet<_> =
            contract.variables().filter(|&var_id| is_candidate_state_var(hir, var_id)).collect();
        if candidate_vars.is_empty() {
            return;
        }

        let mut protected_funcs = HashSet::new();
        let mut protected_entry_points = Vec::new();
        for func_id in contract.all_functions() {
            let func = hir.function(func_id);
            if !is_external_function(func) || !is_protected(hir, func_id, func) {
                continue;
            }

            protected_funcs.insert(func_id);
            if !matches!(func.state_mutability, StateMutability::Pure | StateMutability::View) {
                protected_entry_points.push(func_id);
            }
        }
        if protected_entry_points.is_empty() {
            return;
        }

        let arithmetic_vars =
            vars_used_in_unprotected_arithmetic(hir, contract, &candidate_vars, &protected_funcs);
        if arithmetic_vars.is_empty() {
            return;
        }

        for func_id in protected_entry_points {
            let mut analyzer = WriteAnalyzer::new(hir, &arithmetic_vars);
            let writes = analyzer.analyze_entry_point(func_id);
            let mut emitted = HashSet::new();

            for write in writes {
                if !emitted.insert(write.var_id) {
                    continue;
                }

                let name = hir
                    .variable(write.var_id)
                    .name
                    .map(|name| name.as_str().to_string())
                    .unwrap_or_else(|| "state variable".to_string());
                ctx.emit_with_msg(
                    &MISSING_EVENTS_ARITHMETIC,
                    write.span,
                    format!("`{name}` is changed without an event but is used in arithmetic"),
                );
            }
        }
    }
}

fn is_candidate_state_var(hir: &hir::Hir<'_>, var_id: VariableId) -> bool {
    let var = hir.variable(var_id);
    var.kind.is_state()
        && !var.is_constant()
        && !var.is_immutable()
        && matches!(
            var.ty.kind,
            TypeKind::Elementary(ElementaryType::Int(_) | ElementaryType::UInt(_))
        )
}

fn is_external_function(func: &hir::Function<'_>) -> bool {
    func.kind.is_function()
        && matches!(func.visibility, Visibility::Public | Visibility::External)
        && !func.is_constructor()
        && !func.is_special()
}

fn vars_used_in_unprotected_arithmetic<'hir>(
    hir: &'hir hir::Hir<'hir>,
    contract: &hir::Contract<'hir>,
    candidate_vars: &HashSet<VariableId>,
    protected_funcs: &HashSet<FunctionId>,
) -> HashSet<VariableId> {
    let mut used = HashSet::new();

    for func_id in contract.all_functions() {
        let func = hir.function(func_id);
        if !is_external_function(func) || protected_funcs.contains(&func_id) {
            continue;
        }

        let mut analyzer = ArithmeticUseAnalyzer::new(hir, candidate_vars);
        used.extend(analyzer.analyze_entry_point(func_id));
    }

    used
}

#[derive(Clone, Copy, Debug)]
struct StateWrite {
    var_id: VariableId,
    span: Span,
}

#[derive(Clone, Default)]
struct WriteState {
    taint: HashMap<VariableId, HashSet<VariableId>>,
    dynamic_taint: HashSet<VariableId>,
    pending_writes: Vec<StateWrite>,
}

impl WriteState {
    fn record_write(&mut self, write: StateWrite) {
        self.pending_writes.push(write);
    }

    fn record_event(&mut self) {
        self.pending_writes.clear();
    }
}

#[derive(Default)]
struct WriteFlow {
    fallthrough: Vec<WriteState>,
    returned: Vec<WriteState>,
}

impl WriteFlow {
    fn fallthrough(state: WriteState) -> Self {
        Self { fallthrough: vec![state], returned: Vec::new() }
    }

    fn returned(state: WriteState) -> Self {
        Self { fallthrough: Vec::new(), returned: vec![state] }
    }
}

struct WriteAnalyzer<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    targets: &'a HashSet<VariableId>,
    call_stack: Vec<FunctionId>,
}

impl<'a, 'hir> WriteAnalyzer<'a, 'hir> {
    const fn new(hir: &'hir hir::Hir<'hir>, targets: &'a HashSet<VariableId>) -> Self {
        Self { hir, targets, call_stack: Vec::new() }
    }

    fn analyze_entry_point(&mut self, func_id: FunctionId) -> Vec<StateWrite> {
        let mut state = WriteState::default();
        let func = self.hir.function(func_id);
        for &param in func.parameters {
            state.taint.insert(param, HashSet::from([param]));
        }
        let modifier_ids: Vec<_> =
            func.modifiers.iter().filter_map(|modifier| modifier.id.as_function()).collect();

        let flow = self.analyze_function(func_id, state);
        let flow = self.analyze_modifier_suffixes(&modifier_ids, flow);
        flow.fallthrough
            .iter()
            .chain(&flow.returned)
            .flat_map(|state| state.pending_writes.iter().copied())
            .collect()
    }

    fn analyze_function(&mut self, func_id: FunctionId, state: WriteState) -> WriteFlow {
        if self.call_stack.contains(&func_id) {
            return WriteFlow::fallthrough(state);
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else {
            return WriteFlow::fallthrough(state);
        };

        self.call_stack.push(func_id);
        let flow = self.analyze_stmts(body.stmts, vec![state]);
        self.call_stack.pop();
        flow
    }

    fn analyze_modifier_suffixes(
        &mut self,
        modifier_ids: &[FunctionId],
        mut flow: WriteFlow,
    ) -> WriteFlow {
        for &modifier_id in modifier_ids.iter().rev() {
            flow = self.analyze_modifier_suffix(modifier_id, flow);
        }
        flow
    }

    fn analyze_modifier_suffix(&mut self, modifier_id: FunctionId, flow: WriteFlow) -> WriteFlow {
        let modifier = self.hir.function(modifier_id);
        let Some(body) = modifier.body else { return flow };
        let Some(placeholder_pos) =
            body.stmts.iter().position(|stmt| matches!(stmt.kind, StmtKind::Placeholder))
        else {
            return flow;
        };
        let suffix = &body.stmts[placeholder_pos + 1..];
        if suffix.is_empty() {
            return flow;
        }

        let fallthrough_flow = self.analyze_stmts(suffix, flow.fallthrough);
        let returned_flow = self.analyze_stmts(suffix, flow.returned);

        let mut returned = fallthrough_flow.returned;
        returned.extend(returned_flow.fallthrough);
        returned.extend(returned_flow.returned);
        WriteFlow { fallthrough: fallthrough_flow.fallthrough, returned }
    }

    fn analyze_stmts(
        &mut self,
        stmts: &'hir [hir::Stmt<'hir>],
        mut states: Vec<WriteState>,
    ) -> WriteFlow {
        let mut returned = Vec::new();

        for stmt in stmts {
            let mut next_states = Vec::new();
            for state in states {
                let flow = self.analyze_stmt(stmt, state);
                next_states.extend(flow.fallthrough);
                returned.extend(flow.returned);
            }
            states = merge_write_states(next_states);
            if states.is_empty() {
                break;
            }
        }

        WriteFlow { fallthrough: states, returned }
    }

    fn analyze_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>, mut state: WriteState) -> WriteFlow {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                let var = self.hir.variable(var_id);
                if let Some(init) = var.initializer
                    && !var.kind.is_state()
                {
                    self.analyze_expr(init, &mut state);
                    let sources = self.taint_sources(&state, init);
                    let is_dynamic = self.expr_has_dynamic_value(&state, init);
                    self.set_local_taint(&mut state, var_id, sources, is_dynamic);
                }
                WriteFlow::fallthrough(state)
            }
            StmtKind::DeclMulti(vars, expr) => {
                self.analyze_expr(expr, &mut state);
                let sources = self.taint_sources(&state, expr);
                let is_dynamic = self.expr_has_dynamic_value(&state, expr);
                for var_id in vars.iter().flatten().copied() {
                    if !self.hir.variable(var_id).kind.is_state() {
                        self.set_local_taint(&mut state, var_id, sources.clone(), is_dynamic);
                    }
                }
                WriteFlow::fallthrough(state)
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                self.analyze_stmts(block.stmts, vec![state])
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.analyze_expr(cond, &mut state);

                let then_flow = self.analyze_stmt(then_stmt, state.clone());

                let else_flow = if let Some(else_stmt) = else_stmt {
                    self.analyze_stmt(else_stmt, state)
                } else {
                    WriteFlow::fallthrough(state)
                };

                let mut returned = then_flow.returned;
                returned.extend(else_flow.returned);
                let mut fallthrough = then_flow.fallthrough;
                fallthrough.extend(else_flow.fallthrough);
                WriteFlow { fallthrough: merge_write_states(fallthrough), returned }
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr, &mut state);
                let mut fallthrough = Vec::new();
                let mut returned = Vec::new();
                for clause in try_stmt.clauses {
                    let flow = self.analyze_stmts(clause.block.stmts, vec![state.clone()]);
                    fallthrough.extend(flow.fallthrough);
                    returned.extend(flow.returned);
                }
                WriteFlow { fallthrough: merge_write_states(fallthrough), returned }
            }
            StmtKind::Expr(expr) => {
                self.analyze_expr(expr, &mut state);
                WriteFlow::fallthrough(state)
            }
            StmtKind::Revert(expr) => {
                self.analyze_expr(expr, &mut state);
                WriteFlow::default()
            }
            StmtKind::Emit(expr) => {
                self.analyze_expr(expr, &mut state);
                state.record_event();
                WriteFlow::fallthrough(state)
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.analyze_expr(expr, &mut state);
                }
                WriteFlow::returned(state)
            }
            StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Placeholder
            | StmtKind::AssemblyBlock(_)
            | StmtKind::Switch(_)
            | StmtKind::Err(_) => WriteFlow::fallthrough(state),
        }
    }

    fn analyze_expr(&mut self, expr: &'hir hir::Expr<'hir>, state: &mut WriteState) {
        match &expr.kind {
            ExprKind::Assign(lhs, op, rhs) => {
                self.analyze_expr(rhs, state);

                let sources = self.taint_sources(state, rhs);
                let is_dynamic = self.expr_has_dynamic_value(state, rhs);
                let is_arithmetic_assignment = op.is_some_and(|op| is_arithmetic_op(op.kind));
                for var_id in state_lhs_vars(self.hir, lhs) {
                    if self.targets.contains(&var_id) && (is_arithmetic_assignment || is_dynamic) {
                        state.record_write(StateWrite { var_id, span: lhs.span });
                    }
                }

                if let Some(local) = lhs_local_var(self.hir, lhs) {
                    self.set_local_taint(state, local, sources, is_dynamic);
                } else {
                    self.analyze_lhs_indices(lhs, state);
                }
            }
            ExprKind::Call(callee, args, opts) => {
                self.analyze_expr(callee, state);
                if let Some(opts) = opts {
                    for opt in opts.args {
                        self.analyze_expr(&opt.value, state);
                    }
                }
                for arg in args.exprs() {
                    self.analyze_expr(arg, state);
                }

                for callee_id in resolved_function_ids(callee) {
                    self.analyze_internal_call(callee_id, args, state);
                }
            }
            ExprKind::Binary(lhs, _, rhs) => {
                self.analyze_expr(lhs, state);
                self.analyze_expr(rhs, state);
            }
            ExprKind::Unary(op, inner) if is_inc_dec_op(op.kind) => {
                for var_id in state_lhs_vars(self.hir, inner) {
                    if self.targets.contains(&var_id) {
                        state.record_write(StateWrite { var_id, span: inner.span });
                    }
                }
                self.analyze_lhs_indices(inner, state);
            }
            ExprKind::Unary(_, inner)
            | ExprKind::Delete(inner)
            | ExprKind::Member(inner, _)
            | ExprKind::Payable(inner) => self.analyze_expr(inner, state),
            ExprKind::Index(base, index) => {
                self.analyze_expr(base, state);
                if let Some(index) = index {
                    self.analyze_expr(index, state);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_expr(base, state);
                if let Some(start) = start {
                    self.analyze_expr(start, state);
                }
                if let Some(end) = end {
                    self.analyze_expr(end, state);
                }
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                self.analyze_expr(cond, state);

                let mut true_state = state.clone();
                self.analyze_expr(true_expr, &mut true_state);

                let mut false_state = state.clone();
                self.analyze_expr(false_expr, &mut false_state);

                if let Some(merged) = merge_write_states(vec![true_state, false_state]).pop() {
                    *state = merged;
                }
            }
            ExprKind::Array(exprs) => {
                for expr in *exprs {
                    self.analyze_expr(expr, state);
                }
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().copied().flatten() {
                    self.analyze_expr(expr, state);
                }
            }
            ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
            ExprKind::Ident(_) | ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => {}
        }
    }

    fn analyze_internal_call(
        &mut self,
        callee_id: FunctionId,
        args: &hir::CallArgs<'hir>,
        state: &mut WriteState,
    ) {
        if self.call_stack.contains(&callee_id) {
            return;
        }

        let callee = self.hir.function(callee_id);
        let Some(_) = callee.body else { return };

        let saved_state = state.clone();
        let mut callee_state = state.clone();
        callee_state.taint.clear();
        callee_state.dynamic_taint.clear();
        for (param, arg) in callee.parameters.iter().copied().zip(args.exprs()) {
            let sources = self.taint_sources(&saved_state, arg);
            let is_dynamic = self.expr_has_dynamic_value(&saved_state, arg);
            self.set_local_taint(&mut callee_state, param, sources, is_dynamic);
        }

        let flow = self.analyze_function(callee_id, callee_state);
        let mut states = flow.fallthrough;
        states.extend(flow.returned);
        if let Some(mut merged) = merge_write_states(states).pop() {
            merged.taint = saved_state.taint;
            merged.dynamic_taint = saved_state.dynamic_taint;
            *state = merged;
        }
    }

    fn analyze_lhs_indices(&mut self, expr: &'hir hir::Expr<'hir>, state: &mut WriteState) {
        match &expr.kind {
            ExprKind::Index(base, index) => {
                self.analyze_lhs_indices(base, state);
                if let Some(index) = index {
                    self.analyze_expr(index, state);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_lhs_indices(base, state);
                if let Some(start) = start {
                    self.analyze_expr(start, state);
                }
                if let Some(end) = end {
                    self.analyze_expr(end, state);
                }
            }
            ExprKind::Member(base, _) | ExprKind::Payable(base) => {
                self.analyze_lhs_indices(base, state);
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().copied().flatten() {
                    self.analyze_lhs_indices(expr, state);
                }
            }
            _ => {}
        }
    }

    fn taint_sources(&self, state: &WriteState, expr: &hir::Expr<'_>) -> HashSet<VariableId> {
        collect_write_taint_sources(self.hir, &state.taint, expr)
    }

    fn expr_has_dynamic_value(&self, state: &WriteState, expr: &hir::Expr<'_>) -> bool {
        expr_has_dynamic_value(self.hir, &state.taint, &state.dynamic_taint, expr)
    }

    fn set_local_taint(
        &mut self,
        state: &mut WriteState,
        var_id: VariableId,
        sources: HashSet<VariableId>,
        is_dynamic: bool,
    ) {
        if sources.is_empty() {
            state.taint.remove(&var_id);
        } else {
            state.taint.insert(var_id, sources);
        }
        if is_dynamic {
            state.dynamic_taint.insert(var_id);
        } else {
            state.dynamic_taint.remove(&var_id);
        }
    }
}

fn merge_write_states(mut states: Vec<WriteState>) -> Vec<WriteState> {
    let Some(mut merged) = states.pop() else {
        return Vec::new();
    };

    for state in states {
        merged.taint = merge_taint(&merged.taint, &state.taint);
        merged.dynamic_taint.extend(state.dynamic_taint);
        merged.pending_writes.extend(state.pending_writes);
    }

    vec![merged]
}

fn merge_taint(
    lhs: &HashMap<VariableId, HashSet<VariableId>>,
    rhs: &HashMap<VariableId, HashSet<VariableId>>,
) -> HashMap<VariableId, HashSet<VariableId>> {
    let mut merged = lhs.clone();
    for (&var_id, sources) in rhs {
        merged.entry(var_id).or_default().extend(sources.iter().copied());
    }
    merged
}

fn set_taint_entry(
    taint: &mut HashMap<VariableId, HashSet<VariableId>>,
    var_id: VariableId,
    sources: HashSet<VariableId>,
) {
    if sources.is_empty() {
        taint.remove(&var_id);
    } else {
        taint.insert(var_id, sources);
    }
}

struct ArithmeticUseAnalyzer<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    targets: &'a HashSet<VariableId>,
    taint: HashMap<VariableId, HashSet<VariableId>>,
    used: HashSet<VariableId>,
    call_stack: Vec<FunctionId>,
}

impl<'a, 'hir> ArithmeticUseAnalyzer<'a, 'hir> {
    fn new(hir: &'hir hir::Hir<'hir>, targets: &'a HashSet<VariableId>) -> Self {
        Self { hir, targets, taint: HashMap::new(), used: HashSet::new(), call_stack: Vec::new() }
    }

    fn analyze_entry_point(&mut self, func_id: FunctionId) -> HashSet<VariableId> {
        self.taint.clear();
        self.analyze_function(func_id);
        std::mem::take(&mut self.used)
    }

    fn analyze_function(&mut self, func_id: FunctionId) {
        if self.call_stack.contains(&func_id) {
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        self.call_stack.push(func_id);
        for stmt in body.stmts {
            self.analyze_stmt(stmt);
        }
        self.call_stack.pop();
    }

    fn analyze_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                let var = self.hir.variable(var_id);
                if let Some(init) = var.initializer
                    && !var.kind.is_state()
                {
                    self.analyze_expr(init);
                    let sources = self.taint_sources(init);
                    self.set_local_taint(var_id, sources);
                }
            }
            StmtKind::DeclMulti(vars, expr) => {
                self.analyze_expr(expr);
                let sources = self.taint_sources(expr);
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
                self.analyze_stmt(then_stmt);
                if let Some(else_stmt) = else_stmt {
                    self.analyze_stmt(else_stmt);
                }
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr);
                for clause in try_stmt.clauses {
                    for stmt in clause.block.stmts {
                        self.analyze_stmt(stmt);
                    }
                }
            }
            StmtKind::Expr(expr) | StmtKind::Emit(expr) | StmtKind::Revert(expr) => {
                self.analyze_expr(expr);
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
        match &expr.kind {
            ExprKind::Assign(lhs, _, rhs) => {
                self.analyze_expr(rhs);
                if let Some(local) = lhs_local_var(self.hir, lhs) {
                    let sources = self.taint_sources(rhs);
                    self.set_local_taint(local, sources);
                } else {
                    self.analyze_lhs_indices(lhs);
                }
            }
            ExprKind::Binary(lhs, op, rhs) => {
                if is_arithmetic_op(op.kind) {
                    let lhs_sources = self.taint_sources(lhs);
                    let rhs_sources = self.taint_sources(rhs);
                    self.used.extend(lhs_sources);
                    self.used.extend(rhs_sources);
                }
                self.analyze_expr(lhs);
                self.analyze_expr(rhs);
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
            ExprKind::Unary(_, inner)
            | ExprKind::Delete(inner)
            | ExprKind::Member(inner, _)
            | ExprKind::Payable(inner) => self.analyze_expr(inner),
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
        for (param, arg) in callee.parameters.iter().copied().zip(args.exprs()) {
            let sources = collect_state_sources(self.hir, self.targets, &saved_taint, arg);
            if !sources.is_empty() {
                self.taint.insert(param, sources);
            }
        }

        self.analyze_function(callee_id);
        self.taint = saved_taint;
    }

    fn analyze_lhs_indices(&mut self, expr: &'hir hir::Expr<'hir>) {
        match &expr.kind {
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

    fn taint_sources(&self, expr: &hir::Expr<'_>) -> HashSet<VariableId> {
        let mut sources = collect_state_sources(self.hir, self.targets, &self.taint, expr);
        self.collect_call_return_sources(expr, &mut sources);
        sources
    }

    fn collect_call_return_sources(&self, expr: &hir::Expr<'_>, out: &mut HashSet<VariableId>) {
        match &expr.peel_parens().kind {
            ExprKind::Call(callee, args, opts) => {
                self.collect_call_return_sources(callee, out);
                if let Some(opts) = opts {
                    for opt in opts.args {
                        self.collect_call_return_sources(&opt.value, out);
                    }
                }
                for arg in args.exprs() {
                    self.collect_call_return_sources(arg, out);
                }
                for callee_id in resolved_function_ids(callee) {
                    self.collect_function_return_sources(callee_id, args, out);
                }
            }
            ExprKind::Assign(_, _, rhs) => self.collect_call_return_sources(rhs, out),
            ExprKind::Binary(lhs, _, rhs) => {
                self.collect_call_return_sources(lhs, out);
                self.collect_call_return_sources(rhs, out);
            }
            ExprKind::Unary(_, inner)
            | ExprKind::Delete(inner)
            | ExprKind::Member(inner, _)
            | ExprKind::Payable(inner) => self.collect_call_return_sources(inner, out),
            ExprKind::Index(base, index) => {
                self.collect_call_return_sources(base, out);
                if let Some(index) = index {
                    self.collect_call_return_sources(index, out);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.collect_call_return_sources(base, out);
                if let Some(start) = start {
                    self.collect_call_return_sources(start, out);
                }
                if let Some(end) = end {
                    self.collect_call_return_sources(end, out);
                }
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                self.collect_call_return_sources(cond, out);
                self.collect_call_return_sources(true_expr, out);
                self.collect_call_return_sources(false_expr, out);
            }
            ExprKind::Array(exprs) => {
                for expr in *exprs {
                    self.collect_call_return_sources(expr, out);
                }
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().copied().flatten() {
                    self.collect_call_return_sources(expr, out);
                }
            }
            ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
            ExprKind::Ident(_) | ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => {}
        }
    }

    fn collect_function_return_sources(
        &self,
        callee_id: FunctionId,
        args: &hir::CallArgs<'_>,
        out: &mut HashSet<VariableId>,
    ) {
        if self.call_stack.contains(&callee_id) {
            return;
        }

        let callee = self.hir.function(callee_id);
        let Some(body) = callee.body else { return };

        let mut taint = HashMap::new();
        for (param, arg) in callee.parameters.iter().copied().zip(args.exprs()) {
            let sources = collect_state_sources(self.hir, self.targets, &self.taint, arg);
            if !sources.is_empty() {
                taint.insert(param, sources);
            }
        }

        for stmt in body.stmts {
            self.collect_stmt_return_sources(stmt, &mut taint, out);
        }
    }

    fn collect_stmt_return_sources(
        &self,
        stmt: &hir::Stmt<'_>,
        taint: &mut HashMap<VariableId, HashSet<VariableId>>,
        out: &mut HashSet<VariableId>,
    ) {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                let var = self.hir.variable(var_id);
                if let Some(init) = var.initializer
                    && !var.kind.is_state()
                {
                    let sources = collect_state_sources(self.hir, self.targets, taint, init);
                    set_taint_entry(taint, var_id, sources);
                }
            }
            StmtKind::DeclMulti(vars, expr) => {
                let sources = collect_state_sources(self.hir, self.targets, taint, expr);
                for var_id in vars.iter().flatten().copied() {
                    if !self.hir.variable(var_id).kind.is_state() {
                        set_taint_entry(taint, var_id, sources.clone());
                    }
                }
            }
            StmtKind::Return(Some(expr)) => {
                out.extend(collect_state_sources(self.hir, self.targets, taint, expr));
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                for stmt in block.stmts {
                    self.collect_stmt_return_sources(stmt, taint, out);
                }
            }
            StmtKind::If(_, then_stmt, else_stmt) => {
                let mut then_taint = taint.clone();
                self.collect_stmt_return_sources(then_stmt, &mut then_taint, out);
                if let Some(else_stmt) = else_stmt {
                    let mut else_taint = taint.clone();
                    self.collect_stmt_return_sources(else_stmt, &mut else_taint, out);
                    *taint = merge_taint(&then_taint, &else_taint);
                } else {
                    *taint = merge_taint(taint, &then_taint);
                }
            }
            StmtKind::Try(try_stmt) => {
                for clause in try_stmt.clauses {
                    let mut clause_taint = taint.clone();
                    for stmt in clause.block.stmts {
                        self.collect_stmt_return_sources(stmt, &mut clause_taint, out);
                    }
                    *taint = merge_taint(taint, &clause_taint);
                }
            }
            StmtKind::Expr(_)
            | StmtKind::Emit(_)
            | StmtKind::Revert(_)
            | StmtKind::Return(None)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Placeholder
            | StmtKind::AssemblyBlock(_)
            | StmtKind::Switch(_)
            | StmtKind::Err(_) => {}
        }
    }

    fn set_local_taint(&mut self, var_id: VariableId, sources: HashSet<VariableId>) {
        if sources.is_empty() {
            self.taint.remove(&var_id);
        } else {
            self.taint.insert(var_id, sources);
        }
    }
}

fn collect_write_taint_sources(
    hir: &hir::Hir<'_>,
    taint: &HashMap<VariableId, HashSet<VariableId>>,
    expr: &hir::Expr<'_>,
) -> HashSet<VariableId> {
    let mut out = HashSet::new();
    collect_write_taint_sources_into(hir, taint, expr, &mut out);
    out
}

fn collect_write_taint_sources_into(
    hir: &hir::Hir<'_>,
    taint: &HashMap<VariableId, HashSet<VariableId>>,
    expr: &hir::Expr<'_>,
    out: &mut HashSet<VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            for res in *reses {
                if let Res::Item(ItemId::Variable(var_id)) = res {
                    let var = hir.variable(*var_id);
                    if var.kind.is_state() && !var.is_constant() && !var.is_immutable() {
                        out.insert(*var_id);
                    }
                    if let Some(sources) = taint.get(var_id) {
                        out.extend(sources.iter().copied());
                    }
                }
            }
        }
        ExprKind::Assign(_, _, rhs) => collect_write_taint_sources_into(hir, taint, rhs, out),
        ExprKind::Binary(lhs, _, rhs) => {
            collect_write_taint_sources_into(hir, taint, lhs, out);
            collect_write_taint_sources_into(hir, taint, rhs, out);
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => collect_write_taint_sources_into(hir, taint, inner, out),
        ExprKind::Index(base, index) => {
            collect_write_taint_sources_into(hir, taint, base, out);
            if let Some(index) = index {
                collect_write_taint_sources_into(hir, taint, index, out);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_write_taint_sources_into(hir, taint, base, out);
            if let Some(start) = start {
                collect_write_taint_sources_into(hir, taint, start, out);
            }
            if let Some(end) = end {
                collect_write_taint_sources_into(hir, taint, end, out);
            }
        }
        ExprKind::Call(callee, args, opts) => {
            collect_write_taint_sources_into(hir, taint, callee, out);
            if let Some(opts) = opts {
                for opt in opts.args {
                    collect_write_taint_sources_into(hir, taint, &opt.value, out);
                }
            }
            for arg in args.exprs() {
                collect_write_taint_sources_into(hir, taint, arg, out);
            }
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            collect_write_taint_sources_into(hir, taint, cond, out);
            collect_write_taint_sources_into(hir, taint, true_expr, out);
            collect_write_taint_sources_into(hir, taint, false_expr, out);
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_write_taint_sources_into(hir, taint, expr, out);
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_write_taint_sources_into(hir, taint, expr, out);
            }
        }
        ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
        ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => {}
    }
}

fn expr_has_dynamic_value(
    hir: &hir::Hir<'_>,
    taint: &HashMap<VariableId, HashSet<VariableId>>,
    dynamic_taint: &HashSet<VariableId>,
    expr: &hir::Expr<'_>,
) -> bool {
    if !collect_write_taint_sources(hir, taint, expr).is_empty() {
        return true;
    }

    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().any(|res| {
            let Res::Item(ItemId::Variable(var_id)) = res else { return false };
            dynamic_taint.contains(var_id)
        }),
        ExprKind::Call(..) => true,
        ExprKind::Member(_, _) if is_dynamic_builtin_member(expr) => true,
        ExprKind::Assign(_, _, rhs) => expr_has_dynamic_value(hir, taint, dynamic_taint, rhs),
        ExprKind::Binary(lhs, _, rhs) => {
            expr_has_dynamic_value(hir, taint, dynamic_taint, lhs)
                || expr_has_dynamic_value(hir, taint, dynamic_taint, rhs)
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => expr_has_dynamic_value(hir, taint, dynamic_taint, inner),
        ExprKind::Index(base, index) => {
            expr_has_dynamic_value(hir, taint, dynamic_taint, base)
                || index
                    .is_some_and(|index| expr_has_dynamic_value(hir, taint, dynamic_taint, index))
        }
        ExprKind::Slice(base, start, end) => {
            expr_has_dynamic_value(hir, taint, dynamic_taint, base)
                || start
                    .is_some_and(|start| expr_has_dynamic_value(hir, taint, dynamic_taint, start))
                || end.is_some_and(|end| expr_has_dynamic_value(hir, taint, dynamic_taint, end))
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            expr_has_dynamic_value(hir, taint, dynamic_taint, cond)
                || expr_has_dynamic_value(hir, taint, dynamic_taint, true_expr)
                || expr_has_dynamic_value(hir, taint, dynamic_taint, false_expr)
        }
        ExprKind::Array(exprs) => {
            exprs.iter().any(|expr| expr_has_dynamic_value(hir, taint, dynamic_taint, expr))
        }
        ExprKind::Tuple(exprs) => exprs
            .iter()
            .copied()
            .flatten()
            .any(|expr| expr_has_dynamic_value(hir, taint, dynamic_taint, expr)),
        ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => false,
        ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => false,
    }
}

fn is_dynamic_builtin_member(expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Member(base, _) = &expr.peel_parens().kind else { return false };
    let ExprKind::Ident(reses) = &base.peel_parens().kind else { return false };
    reses.iter().any(|res| {
        let Res::Builtin(builtin) = res else { return false };
        matches!(builtin.name(), sym::block | sym::msg | sym::tx)
    })
}

fn collect_state_sources(
    hir: &hir::Hir<'_>,
    targets: &HashSet<VariableId>,
    taint: &HashMap<VariableId, HashSet<VariableId>>,
    expr: &hir::Expr<'_>,
) -> HashSet<VariableId> {
    let mut out = HashSet::new();
    collect_state_sources_into(hir, targets, taint, expr, &mut out);
    out
}

fn collect_state_sources_into(
    hir: &hir::Hir<'_>,
    targets: &HashSet<VariableId>,
    taint: &HashMap<VariableId, HashSet<VariableId>>,
    expr: &hir::Expr<'_>,
    out: &mut HashSet<VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            for res in *reses {
                if let Res::Item(ItemId::Variable(var_id)) = res {
                    if targets.contains(var_id) && hir.variable(*var_id).kind.is_state() {
                        out.insert(*var_id);
                    }
                    if let Some(sources) = taint.get(var_id) {
                        out.extend(
                            sources.iter().copied().filter(|source| targets.contains(source)),
                        );
                    }
                }
            }
        }
        ExprKind::Assign(_, _, rhs) => collect_state_sources_into(hir, targets, taint, rhs, out),
        ExprKind::Binary(lhs, _, rhs) => {
            collect_state_sources_into(hir, targets, taint, lhs, out);
            collect_state_sources_into(hir, targets, taint, rhs, out);
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => collect_state_sources_into(hir, targets, taint, inner, out),
        ExprKind::Index(base, index) => {
            collect_state_sources_into(hir, targets, taint, base, out);
            if let Some(index) = index {
                collect_state_sources_into(hir, targets, taint, index, out);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_state_sources_into(hir, targets, taint, base, out);
            if let Some(start) = start {
                collect_state_sources_into(hir, targets, taint, start, out);
            }
            if let Some(end) = end {
                collect_state_sources_into(hir, targets, taint, end, out);
            }
        }
        ExprKind::Call(callee, args, opts) => {
            collect_state_sources_into(hir, targets, taint, callee, out);
            if let Some(opts) = opts {
                for opt in opts.args {
                    collect_state_sources_into(hir, targets, taint, &opt.value, out);
                }
            }
            for arg in args.exprs() {
                collect_state_sources_into(hir, targets, taint, arg, out);
            }
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            collect_state_sources_into(hir, targets, taint, cond, out);
            collect_state_sources_into(hir, targets, taint, true_expr, out);
            collect_state_sources_into(hir, targets, taint, false_expr, out);
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_state_sources_into(hir, targets, taint, expr, out);
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_state_sources_into(hir, targets, taint, expr, out);
            }
        }
        ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
        ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => {}
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

fn state_lhs_vars(hir: &hir::Hir<'_>, lhs: &hir::Expr<'_>) -> Vec<VariableId> {
    let mut vars = Vec::new();
    collect_state_lhs_vars(hir, lhs, &mut vars);
    vars
}

fn collect_state_lhs_vars(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>, vars: &mut Vec<VariableId>) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            for res in *reses {
                if let Res::Item(ItemId::Variable(var_id)) = res
                    && hir.variable(*var_id).kind.is_state()
                    && !vars.contains(var_id)
                {
                    vars.push(*var_id);
                }
            }
        }
        ExprKind::Index(base, _) | ExprKind::Slice(base, ..) => {
            collect_state_lhs_vars(hir, base, vars);
        }
        ExprKind::Member(base, _)
        | ExprKind::Payable(base)
        | ExprKind::Unary(_, base)
        | ExprKind::Delete(base) => collect_state_lhs_vars(hir, base, vars),
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_state_lhs_vars(hir, expr, vars);
            }
        }
        _ => {}
    }
}

const fn is_arithmetic_op(kind: BinOpKind) -> bool {
    matches!(
        kind,
        BinOpKind::Add
            | BinOpKind::Sub
            | BinOpKind::Mul
            | BinOpKind::Div
            | BinOpKind::Rem
            | BinOpKind::Pow
    )
}

const fn is_inc_dec_op(kind: UnOpKind) -> bool {
    matches!(kind, UnOpKind::PreInc | UnOpKind::PostInc | UnOpKind::PreDec | UnOpKind::PostDec)
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

fn modifier_has_access_control(hir: &hir::Hir<'_>, modifier_id: FunctionId) -> bool {
    let modifier = hir.function(modifier_id);
    if let Some(body) = modifier.body {
        for stmt in body.stmts {
            if stmt_is_access_guard(hir, stmt, &mut HashSet::new()) {
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
    let Some(body) = func.body else { return false };

    for stmt in body.stmts {
        if stmt_is_access_guard(hir, stmt, seen) {
            return true;
        }
    }
    false
}

fn stmt_is_access_guard(
    hir: &hir::Hir<'_>,
    stmt: &hir::Stmt<'_>,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    match stmt.kind {
        StmtKind::If(cond, then_stmt, else_stmt) => {
            (expr_is_unauthorized_access_check(hir, cond) && branch_always_exits(then_stmt))
                || (expr_is_authorized_access_check(hir, cond)
                    && else_stmt.is_some_and(branch_always_exits))
        }
        StmtKind::Expr(expr) => expr_has_access_guard(hir, expr, seen),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            block.stmts.iter().any(|stmt| stmt_is_access_guard(hir, stmt, seen))
        }
        StmtKind::Try(_)
        | StmtKind::Return(Some(_))
        | StmtKind::Emit(_)
        | StmtKind::Revert(_)
        | StmtKind::DeclSingle(_)
        | StmtKind::DeclMulti(_, _) => false,
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::AssemblyBlock(_)
        | StmtKind::Switch(_)
        | StmtKind::Err(_) => false,
    }
}

fn expr_has_access_guard(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
            args.exprs().next().is_some_and(|cond| expr_is_authorized_access_check(hir, cond))
        }
        ExprKind::Call(callee, _, _) => {
            for callee_id in resolved_function_ids(callee) {
                let func = hir.function(callee_id);
                let name_only_guard = func.body.is_none()
                    && func.returns.is_empty()
                    && func.name.is_some_and(|name| name_looks_like_access_control(name.as_str()));
                if name_only_guard || function_has_access_guard(hir, callee_id, seen) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccessCheckPolarity {
    Authorized,
    Unauthorized,
}

fn expr_is_authorized_access_check(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    expr_access_check_polarity(hir, expr)
        .is_some_and(|polarity| matches!(polarity, AccessCheckPolarity::Authorized))
}

fn expr_is_unauthorized_access_check(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    expr_access_check_polarity(hir, expr)
        .is_some_and(|polarity| matches!(polarity, AccessCheckPolarity::Unauthorized))
}

fn expr_access_check_polarity(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
) -> Option<AccessCheckPolarity> {
    match &expr.peel_parens().kind {
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            Some(match expr_access_check_polarity(hir, inner)? {
                AccessCheckPolarity::Authorized => AccessCheckPolarity::Unauthorized,
                AccessCheckPolarity::Unauthorized => AccessCheckPolarity::Authorized,
            })
        }
        ExprKind::Binary(lhs, op, rhs) if op.kind == BinOpKind::And => {
            let lhs = expr_access_check_polarity(hir, lhs);
            let rhs = expr_access_check_polarity(hir, rhs);
            if matches!(lhs, Some(AccessCheckPolarity::Authorized))
                || matches!(rhs, Some(AccessCheckPolarity::Authorized))
            {
                Some(AccessCheckPolarity::Authorized)
            } else if matches!(lhs, Some(AccessCheckPolarity::Unauthorized))
                && matches!(rhs, Some(AccessCheckPolarity::Unauthorized))
            {
                Some(AccessCheckPolarity::Unauthorized)
            } else {
                None
            }
        }
        ExprKind::Binary(lhs, op, rhs) if op.kind == BinOpKind::Or => {
            let lhs = expr_access_check_polarity(hir, lhs);
            let rhs = expr_access_check_polarity(hir, rhs);
            if matches!(lhs, Some(AccessCheckPolarity::Authorized))
                && matches!(rhs, Some(AccessCheckPolarity::Authorized))
            {
                Some(AccessCheckPolarity::Authorized)
            } else if matches!(lhs, Some(AccessCheckPolarity::Unauthorized))
                || matches!(rhs, Some(AccessCheckPolarity::Unauthorized))
            {
                Some(AccessCheckPolarity::Unauthorized)
            } else {
                None
            }
        }
        ExprKind::Binary(lhs, op, rhs)
            if matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne)
                && expr_compares_sender_to_authority(hir, lhs, rhs) =>
        {
            Some(if op.kind == BinOpKind::Eq {
                AccessCheckPolarity::Authorized
            } else {
                AccessCheckPolarity::Unauthorized
            })
        }
        _ if expr_looks_like_access_check(hir, expr) => Some(AccessCheckPolarity::Authorized),
        _ => None,
    }
}

fn expr_compares_sender_to_authority(
    hir: &hir::Hir<'_>,
    lhs: &hir::Expr<'_>,
    rhs: &hir::Expr<'_>,
) -> bool {
    let mut seen = HashSet::new();
    (expr_reads_sender(hir, lhs, &mut seen)
        && (expr_reads_state_variable(hir, rhs) || expr_calls_non_sender_user_function(hir, rhs)))
        || {
            let mut seen = HashSet::new();
            expr_reads_sender(hir, rhs, &mut seen)
                && (expr_reads_state_variable(hir, lhs)
                    || expr_calls_non_sender_user_function(hir, lhs))
        }
}

fn expr_looks_like_access_check(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    expr_reads_sender(hir, expr, &mut HashSet::new())
        && (expr_reads_state_variable(hir, expr) || expr_calls_non_sender_user_function(hir, expr))
}

fn expr_reads_state_variable(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().any(|res| {
            let Res::Item(ItemId::Variable(var_id)) = res else { return false };
            hir.variable(*var_id).kind.is_state()
        }),
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            expr_reads_state_variable(hir, lhs) || expr_reads_state_variable(hir, rhs)
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => expr_reads_state_variable(hir, inner),
        ExprKind::Index(base, index) => {
            expr_reads_state_variable(hir, base)
                || index.is_some_and(|index| expr_reads_state_variable(hir, index))
        }
        ExprKind::Slice(base, start, end) => {
            expr_reads_state_variable(hir, base)
                || start.is_some_and(|start| expr_reads_state_variable(hir, start))
                || end.is_some_and(|end| expr_reads_state_variable(hir, end))
        }
        ExprKind::Call(callee, args, opts) => {
            expr_reads_state_variable(hir, callee)
                || opts.is_some_and(|opts| {
                    opts.args.iter().any(|opt| expr_reads_state_variable(hir, &opt.value))
                })
                || args.exprs().any(|arg| expr_reads_state_variable(hir, arg))
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            expr_reads_state_variable(hir, cond)
                || expr_reads_state_variable(hir, true_expr)
                || expr_reads_state_variable(hir, false_expr)
        }
        ExprKind::Array(exprs) => exprs.iter().any(|expr| expr_reads_state_variable(hir, expr)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().copied().flatten().any(|expr| expr_reads_state_variable(hir, expr))
        }
        _ => false,
    }
}

fn expr_calls_non_sender_user_function(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Call(callee, args, opts) => {
            resolved_function_ids(callee).any(|func_id| {
                hir.function(func_id)
                    .name
                    .is_some_and(|name| !name_looks_like_sender_accessor(name.as_str()))
            }) || expr_calls_non_sender_user_function(hir, callee)
                || opts.is_some_and(|opts| {
                    opts.args.iter().any(|opt| expr_calls_non_sender_user_function(hir, &opt.value))
                })
                || args.exprs().any(|arg| expr_calls_non_sender_user_function(hir, arg))
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            expr_calls_non_sender_user_function(hir, lhs)
                || expr_calls_non_sender_user_function(hir, rhs)
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => expr_calls_non_sender_user_function(hir, inner),
        ExprKind::Index(base, index) => {
            expr_calls_non_sender_user_function(hir, base)
                || index.is_some_and(|index| expr_calls_non_sender_user_function(hir, index))
        }
        ExprKind::Slice(base, start, end) => {
            expr_calls_non_sender_user_function(hir, base)
                || start.is_some_and(|start| expr_calls_non_sender_user_function(hir, start))
                || end.is_some_and(|end| expr_calls_non_sender_user_function(hir, end))
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            expr_calls_non_sender_user_function(hir, cond)
                || expr_calls_non_sender_user_function(hir, true_expr)
                || expr_calls_non_sender_user_function(hir, false_expr)
        }
        ExprKind::Array(exprs) => {
            exprs.iter().any(|expr| expr_calls_non_sender_user_function(hir, expr))
        }
        ExprKind::Tuple(exprs) => exprs
            .iter()
            .copied()
            .flatten()
            .any(|expr| expr_calls_non_sender_user_function(hir, expr)),
        _ => false,
    }
}

fn expr_reads_sender(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    if is_sender_member(expr) {
        return true;
    }

    match &expr.peel_parens().kind {
        ExprKind::Call(callee, args, opts) => {
            for callee_id in resolved_function_ids(callee) {
                if function_reads_sender(hir, callee_id, seen) {
                    return true;
                }
            }

            expr_reads_sender(hir, callee, seen)
                || opts.is_some_and(|opts| {
                    opts.args.iter().any(|opt| expr_reads_sender(hir, &opt.value, seen))
                })
                || args.exprs().any(|arg| expr_reads_sender(hir, arg, seen))
        }
        ExprKind::Binary(lhs, _, rhs) => {
            expr_reads_sender(hir, lhs, seen) || expr_reads_sender(hir, rhs, seen)
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => expr_reads_sender(hir, inner, seen),
        ExprKind::Index(base, index) => {
            expr_reads_sender(hir, base, seen)
                || index.is_some_and(|index| expr_reads_sender(hir, index, seen))
        }
        ExprKind::Slice(base, start, end) => {
            expr_reads_sender(hir, base, seen)
                || start.is_some_and(|start| expr_reads_sender(hir, start, seen))
                || end.is_some_and(|end| expr_reads_sender(hir, end, seen))
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            expr_reads_sender(hir, cond, seen)
                || expr_reads_sender(hir, true_expr, seen)
                || expr_reads_sender(hir, false_expr, seen)
        }
        ExprKind::Array(exprs) => exprs.iter().any(|expr| expr_reads_sender(hir, expr, seen)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().copied().flatten().any(|expr| expr_reads_sender(hir, expr, seen))
        }
        ExprKind::Assign(_, _, rhs) => expr_reads_sender(hir, rhs, seen),
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
    body.stmts.iter().any(|stmt| stmt_reads_sender(hir, stmt, seen))
}

fn stmt_reads_sender(
    hir: &hir::Hir<'_>,
    stmt: &hir::Stmt<'_>,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    match stmt.kind {
        StmtKind::DeclSingle(var_id) => {
            hir.variable(var_id).initializer.is_some_and(|init| expr_reads_sender(hir, init, seen))
        }
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Expr(expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr) => expr_reads_sender(hir, expr, seen),
        StmtKind::Return(Some(expr)) => expr_reads_sender(hir, expr, seen),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            block.stmts.iter().any(|stmt| stmt_reads_sender(hir, stmt, seen))
        }
        StmtKind::If(cond, then_stmt, else_stmt) => {
            expr_reads_sender(hir, cond, seen)
                || stmt_reads_sender(hir, then_stmt, seen)
                || else_stmt.is_some_and(|stmt| stmt_reads_sender(hir, stmt, seen))
        }
        StmtKind::Try(try_stmt) => {
            expr_reads_sender(hir, &try_stmt.expr, seen)
                || try_stmt.clauses.iter().any(|clause| {
                    clause.block.stmts.iter().any(|stmt| stmt_reads_sender(hir, stmt, seen))
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
        || lower.starts_with("onlyowner")
        || lower.starts_with("onlyrole")
        || lower.starts_with("checkowner")
        || lower.starts_with("_checkowner")
        || lower.starts_with("checkrole")
        || lower.starts_with("_checkrole")
}

fn name_looks_like_sender_accessor(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "_msgsender" || lower == "msgsender" || lower == "sender"
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
