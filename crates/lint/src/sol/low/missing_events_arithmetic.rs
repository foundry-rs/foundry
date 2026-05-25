use super::MissingEventsArithmetic;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::primitives::is_require_or_assert},
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

        let arithmetic_vars = vars_used_in_unprotected_arithmetic(hir, contract, &candidate_vars);
        if arithmetic_vars.is_empty() {
            return;
        }

        for func_id in contract.all_functions() {
            let func = hir.function(func_id);
            if !is_protected_entry_point(hir, func_id, func) {
                continue;
            }

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

fn is_unprotected_user_function(
    hir: &hir::Hir<'_>,
    func_id: FunctionId,
    func: &hir::Function<'_>,
) -> bool {
    is_external_function(func) && !is_protected(hir, func_id, func)
}

fn vars_used_in_unprotected_arithmetic(
    hir: &hir::Hir<'_>,
    contract: &hir::Contract<'_>,
    candidate_vars: &HashSet<VariableId>,
) -> HashSet<VariableId> {
    let mut used = HashSet::new();

    for func_id in contract.all_functions() {
        let func = hir.function(func_id);
        if !is_unprotected_user_function(hir, func_id, func) {
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
    pending_writes: Vec<StateWrite>,
    emitted_event: bool,
}

impl WriteState {
    fn record_write(&mut self, write: StateWrite) {
        if !self.emitted_event {
            self.pending_writes.push(write);
        }
    }

    fn record_event(&mut self) {
        self.emitted_event = true;
        self.pending_writes.clear();
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

        self.analyze_function(func_id, &mut state);
        state.pending_writes
    }

    fn analyze_function(&mut self, func_id: FunctionId, state: &mut WriteState) {
        if self.call_stack.contains(&func_id) {
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        self.call_stack.push(func_id);
        for stmt in body.stmts {
            self.analyze_stmt(stmt, state);
        }
        self.call_stack.pop();
    }

    fn analyze_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>, state: &mut WriteState) {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                let var = self.hir.variable(var_id);
                if let Some(init) = var.initializer
                    && !var.kind.is_state()
                {
                    self.analyze_expr(init, state);
                    self.set_local_taint(state, var_id, self.taint_sources(state, init));
                }
            }
            StmtKind::DeclMulti(vars, expr) => {
                self.analyze_expr(expr, state);
                let sources = self.taint_sources(state, expr);
                for var_id in vars.iter().flatten().copied() {
                    if !self.hir.variable(var_id).kind.is_state() {
                        self.set_local_taint(state, var_id, sources.clone());
                    }
                }
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                for stmt in block.stmts {
                    self.analyze_stmt(stmt, state);
                }
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.analyze_expr(cond, state);

                let mut then_state = state.clone();
                self.analyze_stmt(then_stmt, &mut then_state);

                let mut else_state = state.clone();
                if let Some(else_stmt) = else_stmt {
                    self.analyze_stmt(else_stmt, &mut else_state);
                }

                state.taint = merge_taint(&then_state.taint, &else_state.taint);
                state.pending_writes = then_state.pending_writes;
                state.pending_writes.extend(else_state.pending_writes);
                state.emitted_event = then_state.emitted_event && else_state.emitted_event;
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr, state);
                for clause in try_stmt.clauses {
                    for stmt in clause.block.stmts {
                        self.analyze_stmt(stmt, state);
                    }
                }
            }
            StmtKind::Expr(expr) | StmtKind::Revert(expr) => {
                self.analyze_expr(expr, state);
            }
            StmtKind::Emit(expr) => {
                self.analyze_expr(expr, state);
                state.record_event();
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.analyze_expr(expr, state);
                }
            }
            StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Err(_) => {}
        }
    }

    fn analyze_expr(&mut self, expr: &'hir hir::Expr<'hir>, state: &mut WriteState) {
        match &expr.kind {
            ExprKind::Assign(lhs, op, rhs) => {
                self.analyze_expr(rhs, state);

                let sources = self.taint_sources(state, rhs);
                let is_arithmetic_assignment = op.is_some_and(|op| is_arithmetic_op(op.kind));
                for var_id in state_lhs_vars(self.hir, lhs) {
                    if self.targets.contains(&var_id)
                        && (is_arithmetic_assignment || !sources.is_empty())
                    {
                        state.record_write(StateWrite { var_id, span: lhs.span });
                    }
                }

                if let Some(local) = lhs_local_var(self.hir, lhs) {
                    self.set_local_taint(state, local, sources);
                } else {
                    self.analyze_lhs_indices(lhs, state);
                }
            }
            ExprKind::Call(callee, args, opts) => {
                self.analyze_expr(callee, state);
                if let Some(opts) = opts {
                    for opt in *opts {
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

                state.taint = merge_taint(&true_state.taint, &false_state.taint);
                state.pending_writes = true_state.pending_writes;
                state.pending_writes.extend(false_state.pending_writes);
                state.emitted_event = true_state.emitted_event && false_state.emitted_event;
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
            ExprKind::Ident(_) | ExprKind::Lit(_) | ExprKind::Err(_) => {}
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

        let saved_taint = std::mem::take(&mut state.taint);
        for (param, arg) in callee.parameters.iter().copied().zip(args.exprs()) {
            let sources = collect_input_taint_sources(&saved_taint, arg);
            if !sources.is_empty() {
                state.taint.insert(param, sources);
            }
        }

        self.analyze_function(callee_id, state);
        state.taint = saved_taint;
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
        collect_input_taint_sources(&state.taint, expr)
    }

    fn set_local_taint(
        &mut self,
        state: &mut WriteState,
        var_id: VariableId,
        sources: HashSet<VariableId>,
    ) {
        if sources.is_empty() {
            state.taint.remove(&var_id);
        } else {
            state.taint.insert(var_id, sources);
        }
    }
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
                    self.set_local_taint(var_id, self.taint_sources(init));
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
            StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Err(_) => {}
        }
    }

    fn analyze_expr(&mut self, expr: &'hir hir::Expr<'hir>) {
        match &expr.kind {
            ExprKind::Assign(lhs, _, rhs) => {
                self.analyze_expr(rhs);
                if let Some(local) = lhs_local_var(self.hir, lhs) {
                    self.set_local_taint(local, self.taint_sources(rhs));
                } else {
                    self.analyze_lhs_indices(lhs);
                }
            }
            ExprKind::Binary(lhs, op, rhs) => {
                if is_arithmetic_op(op.kind) {
                    self.used.extend(self.taint_sources(lhs));
                    self.used.extend(self.taint_sources(rhs));
                }
                self.analyze_expr(lhs);
                self.analyze_expr(rhs);
            }
            ExprKind::Call(callee, args, opts) => {
                self.analyze_expr(callee);
                if let Some(opts) = opts {
                    for opt in *opts {
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
            ExprKind::Ident(_) | ExprKind::Lit(_) | ExprKind::Err(_) => {}
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
        collect_state_sources(self.hir, self.targets, &self.taint, expr)
    }

    fn set_local_taint(&mut self, var_id: VariableId, sources: HashSet<VariableId>) {
        if sources.is_empty() {
            self.taint.remove(&var_id);
        } else {
            self.taint.insert(var_id, sources);
        }
    }
}

fn collect_input_taint_sources(
    taint: &HashMap<VariableId, HashSet<VariableId>>,
    expr: &hir::Expr<'_>,
) -> HashSet<VariableId> {
    let mut out = HashSet::new();
    collect_input_taint_sources_into(taint, expr, &mut out);
    out
}

fn collect_input_taint_sources_into(
    taint: &HashMap<VariableId, HashSet<VariableId>>,
    expr: &hir::Expr<'_>,
    out: &mut HashSet<VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            for res in *reses {
                if let Res::Item(ItemId::Variable(var_id)) = res
                    && let Some(sources) = taint.get(var_id)
                {
                    out.extend(sources.iter().copied());
                }
            }
        }
        ExprKind::Assign(_, _, rhs) => collect_input_taint_sources_into(taint, rhs, out),
        ExprKind::Binary(lhs, _, rhs) => {
            collect_input_taint_sources_into(taint, lhs, out);
            collect_input_taint_sources_into(taint, rhs, out);
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => collect_input_taint_sources_into(taint, inner, out),
        ExprKind::Index(base, index) => {
            collect_input_taint_sources_into(taint, base, out);
            if let Some(index) = index {
                collect_input_taint_sources_into(taint, index, out);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_input_taint_sources_into(taint, base, out);
            if let Some(start) = start {
                collect_input_taint_sources_into(taint, start, out);
            }
            if let Some(end) = end {
                collect_input_taint_sources_into(taint, end, out);
            }
        }
        ExprKind::Call(callee, args, opts) => {
            collect_input_taint_sources_into(taint, callee, out);
            if let Some(opts) = opts {
                for opt in *opts {
                    collect_input_taint_sources_into(taint, &opt.value, out);
                }
            }
            for arg in args.exprs() {
                collect_input_taint_sources_into(taint, arg, out);
            }
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            collect_input_taint_sources_into(taint, cond, out);
            collect_input_taint_sources_into(taint, true_expr, out);
            collect_input_taint_sources_into(taint, false_expr, out);
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_input_taint_sources_into(taint, expr, out);
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_input_taint_sources_into(taint, expr, out);
            }
        }
        ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
        ExprKind::Lit(_) | ExprKind::Err(_) => {}
    }
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
                for opt in *opts {
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
        ExprKind::Lit(_) | ExprKind::Err(_) => {}
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
            if stmt_has_access_guard(hir, stmt, &mut HashSet::new()) {
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

    for stmt in body.stmts {
        if stmt_has_access_guard(hir, stmt, seen) {
            return true;
        }
    }
    false
}

fn stmt_has_access_guard(
    hir: &hir::Hir<'_>,
    stmt: &hir::Stmt<'_>,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    match stmt.kind {
        StmtKind::If(cond, then_stmt, else_stmt) => {
            (expr_is_unauthorized_access_check(hir, cond) && stmt_exits_or_reverts(then_stmt))
                || (expr_is_authorized_access_check(hir, cond)
                    && else_stmt.is_some_and(stmt_exits_or_reverts))
                || stmt_has_access_guard(hir, then_stmt, seen)
                || else_stmt.is_some_and(|stmt| stmt_has_access_guard(hir, stmt, seen))
        }
        StmtKind::Expr(expr) => expr_has_access_guard(hir, expr, seen),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            block.stmts.iter().any(|stmt| stmt_has_access_guard(hir, stmt, seen))
        }
        StmtKind::Try(try_stmt) => try_stmt.clauses.iter().any(|clause| {
            clause.block.stmts.iter().any(|stmt| stmt_has_access_guard(hir, stmt, seen))
        }),
        StmtKind::Return(Some(expr)) | StmtKind::Emit(expr) | StmtKind::Revert(expr) => {
            expr_has_access_guard(hir, expr, seen)
        }
        StmtKind::DeclSingle(var_id) => hir
            .variable(var_id)
            .initializer
            .is_some_and(|init| expr_has_access_guard(hir, init, seen)),
        StmtKind::DeclMulti(_, expr) => expr_has_access_guard(hir, expr, seen),
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::Err(_) => false,
    }
}

fn stmt_exits_or_reverts(stmt: &hir::Stmt<'_>) -> bool {
    match stmt.kind {
        StmtKind::Return(_) | StmtKind::Revert(_) => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.iter().any(stmt_exits_or_reverts)
        }
        StmtKind::If(_, then_stmt, else_stmt) => {
            stmt_exits_or_reverts(then_stmt) || else_stmt.is_some_and(stmt_exits_or_reverts)
        }
        _ => false,
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
        ExprKind::Call(callee, args, opts) => {
            for callee_id in resolved_function_ids(callee) {
                let func = hir.function(callee_id);
                if func.name.is_some_and(|name| name_looks_like_access_control(name.as_str()))
                    || function_has_access_guard(hir, callee_id, seen)
                {
                    return true;
                }
            }

            expr_has_access_guard(hir, callee, seen)
                || opts.is_some_and(|opts| {
                    opts.iter().any(|opt| expr_has_access_guard(hir, &opt.value, seen))
                })
                || args.exprs().any(|arg| expr_has_access_guard(hir, arg, seen))
        }
        ExprKind::Binary(lhs, _, rhs) => {
            expr_has_access_guard(hir, lhs, seen) || expr_has_access_guard(hir, rhs, seen)
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => expr_has_access_guard(hir, inner, seen),
        ExprKind::Index(base, index) => {
            expr_has_access_guard(hir, base, seen)
                || index.is_some_and(|index| expr_has_access_guard(hir, index, seen))
        }
        ExprKind::Slice(base, start, end) => {
            expr_has_access_guard(hir, base, seen)
                || start.is_some_and(|start| expr_has_access_guard(hir, start, seen))
                || end.is_some_and(|end| expr_has_access_guard(hir, end, seen))
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            expr_has_access_guard(hir, cond, seen)
                || expr_has_access_guard(hir, true_expr, seen)
                || expr_has_access_guard(hir, false_expr, seen)
        }
        ExprKind::Array(exprs) => exprs.iter().any(|expr| expr_has_access_guard(hir, expr, seen)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().copied().flatten().any(|expr| expr_has_access_guard(hir, expr, seen))
        }
        ExprKind::Assign(_, _, rhs) => expr_has_access_guard(hir, rhs, seen),
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
                    opts.iter().any(|opt| expr_reads_state_variable(hir, &opt.value))
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
                    opts.iter().any(|opt| expr_calls_non_sender_user_function(hir, &opt.value))
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
                    opts.iter().any(|opt| expr_reads_sender(hir, &opt.value, seen))
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
