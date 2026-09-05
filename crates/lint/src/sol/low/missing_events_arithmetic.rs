use super::MissingEventsArithmetic;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            builtins, dispatched_function, is_protected, lhs_local_var, loop_stmts, state_lhs_vars,
            underlying_var,
        },
    },
};
use solar::{
    ast::{ContractKind, StateMutability},
    interface::Span,
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, BinOpKind, ContractId, ElementaryType, Expr, ExprKind, FunctionId, StmtKind,
            TypeKind, UnOpKind, VariableId, Visit,
        },
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

declare_forge_lint!(
    MISSING_EVENTS_ARITHMETIC,
    Severity::Low,
    "missing-events-arithmetic",
    "critical arithmetic state changes should emit events"
);

impl<'gcx> LateLintPass<'gcx> for MissingEventsArithmetic {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract_id: ContractId,
    ) {
        let contract = gcx.hir.contract(contract_id);
        if contract.kind != ContractKind::Contract || contract.linearization_failed() {
            return;
        }

        // State variables and their entry points are commonly split across a base/derived pair,
        // so candidates come from the whole inheritance chain.
        let candidates: HashSet<_> = contract
            .linearized_bases
            .iter()
            .flat_map(|&cid| gcx.hir.contract(cid).variables())
            .filter(|&id| {
                let var = gcx.hir.variable(id);
                var.kind.is_state()
                    && !var.is_constant()
                    && !var.is_immutable()
                    && matches!(
                        var.ty.kind,
                        TypeKind::Elementary(ElementaryType::Int(_) | ElementaryType::UInt(_))
                    )
            })
            .collect();
        if candidates.is_empty() {
            return;
        }

        // The externally reachable functions, with overridden base implementations resolved to
        // the most derived one.
        let (protected, unprotected): (Vec<_>, Vec<_>) = gcx
            .interface_functions(contract_id)
            .all()
            .iter()
            .map(|func| func.id)
            .partition(|&id| is_protected(&gcx.hir, id));
        let entry_points: Vec<_> = protected
            .into_iter()
            .filter(|&id| {
                !matches!(
                    gcx.hir.function(id).state_mutability,
                    StateMutability::Pure | StateMutability::View
                )
            })
            .collect();
        if entry_points.is_empty() {
            return;
        }

        // Candidates that flow into arithmetic reachable from an unprotected function.
        let mut uses = UseAnalyzer {
            gcx,
            contract_id,
            targets: &candidates,
            mode: Mode::Uses,
            taint: HashMap::new(),
            used: HashSet::new(),
            returned: HashSet::new(),
            call_stack: Vec::new(),
        };
        for func_id in unprotected {
            uses.taint.clear();
            uses.analyze_function(func_id);
        }
        if uses.used.is_empty() {
            return;
        }

        for func_id in entry_points {
            let mut analyzer =
                WriteAnalyzer { gcx, contract_id, targets: &uses.used, call_stack: Vec::new() };
            let mut emitted = HashSet::new();
            for write in analyzer.analyze_entry_point(func_id) {
                if !emitted.insert(write.var_id) {
                    continue;
                }
                let name = gcx
                    .hir
                    .variable(write.var_id)
                    .name
                    .map_or_else(|| "state variable".to_string(), |name| name.to_string());
                ctx.emit_with_msg(
                    &MISSING_EVENTS_ARITHMETIC,
                    write.span,
                    format!("`{name}` is changed without an event but is used in arithmetic"),
                );
            }
        }
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

// --- Arithmetic uses --------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Record target variables that reach arithmetic operators.
    Uses,
    /// Record target variables that reach `return` statements.
    Returns,
}

/// Finds target state variables that flow into arithmetic, following locals and internal calls.
struct UseAnalyzer<'a, 'gcx> {
    gcx: Gcx<'gcx>,
    contract_id: ContractId,
    targets: &'a HashSet<VariableId>,
    mode: Mode,
    /// Target state variables each local may currently hold.
    taint: HashMap<VariableId, HashSet<VariableId>>,
    used: HashSet<VariableId>,
    returned: HashSet<VariableId>,
    call_stack: Vec<FunctionId>,
}

impl<'gcx> UseAnalyzer<'_, 'gcx> {
    fn analyze_function(&mut self, func_id: FunctionId) {
        if self.call_stack.contains(&func_id) {
            return;
        }
        let Some(body) = self.gcx.hir.function(func_id).body else { return };
        self.call_stack.push(func_id);
        for stmt in body.stmts {
            let _ = self.visit_stmt(stmt);
        }
        self.call_stack.pop();
    }

    /// Analyzes `callee_id` with its parameters bound to the sources of `args`, restoring the
    /// caller's taint afterwards.
    fn analyze_call(&mut self, callee_id: FunctionId, args: &hir::CallArgs<'gcx>) {
        if self.call_stack.contains(&callee_id) {
            return;
        }
        let params = self
            .gcx
            .hir
            .function(callee_id)
            .parameters
            .iter()
            .zip(args.exprs())
            .filter_map(|(&param, arg)| {
                let sources = self.sources(arg);
                (!sources.is_empty()).then_some((param, sources))
            })
            .collect();
        let saved = std::mem::replace(&mut self.taint, params);
        self.analyze_function(callee_id);
        self.taint = saved;
    }

    /// Target state variables `expr` may evaluate to, including through helper return values.
    fn sources(&mut self, expr: &Expr<'gcx>) -> HashSet<VariableId> {
        let mut out = HashSet::new();
        let _ = expr.visit(&mut |e| {
            if let Some(var_id) = underlying_var(e) {
                if self.targets.contains(&var_id) {
                    out.insert(var_id);
                }
                if let Some(sources) = self.taint.get(&var_id) {
                    out.extend(sources);
                }
            }
            if let ExprKind::Call(callee, args, _) = &e.kind
                && let Some(callee_id) = dispatched_function(self.gcx, self.contract_id, callee)
            {
                out.extend(self.return_sources(callee_id, args));
            }
            ControlFlow::<()>::Continue(())
        });
        out
    }

    fn return_sources(
        &mut self,
        callee_id: FunctionId,
        args: &hir::CallArgs<'gcx>,
    ) -> HashSet<VariableId> {
        let outer_mode = std::mem::replace(&mut self.mode, Mode::Returns);
        let outer_returned = std::mem::take(&mut self.returned);
        self.analyze_call(callee_id, args);
        self.mode = outer_mode;
        std::mem::replace(&mut self.returned, outer_returned)
    }

    fn set_taint(&mut self, var_id: VariableId, sources: HashSet<VariableId>) {
        if sources.is_empty() {
            self.taint.remove(&var_id);
        } else {
            self.taint.insert(var_id, sources);
        }
    }
}

impl<'gcx> Visit<'gcx> for UseAnalyzer<'_, 'gcx> {
    type BreakValue = solar::interface::data_structures::Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx hir::Stmt<'gcx>) -> ControlFlow<Self::BreakValue> {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.gcx.hir.variable(var_id).initializer {
                    let sources = self.sources(init);
                    self.set_taint(var_id, sources);
                }
            }
            StmtKind::DeclMulti(vars, expr) => {
                let sources = self.sources(expr);
                for var_id in vars.iter().flatten() {
                    self.set_taint(*var_id, sources.clone());
                }
            }
            StmtKind::Return(Some(expr)) if self.mode == Mode::Returns => {
                let sources = self.sources(expr);
                self.returned.extend(sources);
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            ExprKind::Assign(lhs, _, rhs) => {
                if let Some(local) = lhs_local_var(&self.gcx.hir, lhs) {
                    let sources = self.sources(rhs);
                    self.set_taint(local, sources);
                }
            }
            ExprKind::Binary(lhs, op, rhs)
                if self.mode == Mode::Uses && is_arithmetic_op(op.kind) =>
            {
                let sources = self.sources(lhs);
                self.used.extend(sources);
                let sources = self.sources(rhs);
                self.used.extend(sources);
            }
            ExprKind::Call(callee, args, _) if self.mode == Mode::Uses => {
                self.walk_expr(expr)?;
                if let Some(callee_id) = dispatched_function(self.gcx, self.contract_id, callee) {
                    self.analyze_call(callee_id, args);
                }
                return ControlFlow::Continue(());
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

// --- Writes without events --------------------------------------------------------------------

#[derive(Clone, Copy)]
struct StateWrite {
    var_id: VariableId,
    span: Span,
}

/// Analysis state along one control-flow path.
#[derive(Clone, Default)]
struct WriteState {
    /// Locals holding a value that is not a compile-time constant.
    dynamic: HashSet<VariableId>,
    /// Target writes not yet followed by an `emit`.
    writes: Vec<StateWrite>,
}

fn merge(lhs: Option<WriteState>, rhs: Option<WriteState>) -> Option<WriteState> {
    match (lhs, rhs) {
        (Some(mut lhs), Some(rhs)) => {
            lhs.dynamic.extend(rhs.dynamic);
            lhs.writes.extend(rhs.writes);
            Some(lhs)
        }
        (lhs, rhs) => lhs.or(rhs),
    }
}

/// Paths leaving a statement: those continuing to the next statement and those that `return`ed
/// (which skip the rest of the body but still run the modifiers' trailing code).
#[derive(Default)]
struct Flow {
    fallthrough: Option<WriteState>,
    returned: Option<WriteState>,
}

impl Flow {
    const fn fallthrough(state: WriteState) -> Self {
        Self { fallthrough: Some(state), returned: None }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            fallthrough: merge(self.fallthrough, other.fallthrough),
            returned: merge(self.returned, other.returned),
        }
    }

    fn merged(self) -> Option<WriteState> {
        merge(self.fallthrough, self.returned)
    }
}

/// Collects writes to target variables that no later `emit` on the same path covers.
struct WriteAnalyzer<'a, 'gcx> {
    gcx: Gcx<'gcx>,
    contract_id: ContractId,
    targets: &'a HashSet<VariableId>,
    call_stack: Vec<FunctionId>,
}

impl<'gcx> WriteAnalyzer<'_, 'gcx> {
    fn analyze_entry_point(&mut self, func_id: FunctionId) -> Vec<StateWrite> {
        let func = self.gcx.hir.function(func_id);
        let state =
            WriteState { dynamic: func.parameters.iter().copied().collect(), writes: Vec::new() };
        let mut state = self.analyze_function(func_id, state).merged();
        // Modifier code after `_` runs once the body finished, innermost modifier first, and may
        // still emit for the body's writes.
        for modifier in func.modifiers.iter().rev() {
            let Some(body) =
                modifier.id.as_function().and_then(|id| self.gcx.hir.function(id).body)
            else {
                continue;
            };
            let Some(pos) = body.stmts.iter().position(|s| matches!(s.kind, StmtKind::Placeholder))
            else {
                continue;
            };
            let suffix = &body.stmts[pos + 1..];
            state = state.and_then(|state| self.analyze_stmts(suffix, state).merged());
        }
        state.map(|state| state.writes).unwrap_or_default()
    }

    fn analyze_function(&mut self, func_id: FunctionId, state: WriteState) -> Flow {
        if self.call_stack.contains(&func_id) {
            return Flow::fallthrough(state);
        }
        let Some(body) = self.gcx.hir.function(func_id).body else {
            return Flow::fallthrough(state);
        };
        self.call_stack.push(func_id);
        let flow = self.analyze_stmts(body.stmts, state);
        self.call_stack.pop();
        flow
    }

    fn analyze_stmts(
        &mut self,
        stmts: impl IntoIterator<Item = &'gcx hir::Stmt<'gcx>>,
        state: WriteState,
    ) -> Flow {
        let mut flow = Flow::fallthrough(state);
        for stmt in stmts {
            let Some(state) = flow.fallthrough.take() else { break };
            let next = self.analyze_stmt(stmt, state);
            flow.fallthrough = next.fallthrough;
            flow.returned = merge(flow.returned, next.returned);
        }
        flow
    }

    fn analyze_stmt(&mut self, stmt: &'gcx hir::Stmt<'gcx>, mut state: WriteState) -> Flow {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.gcx.hir.variable(var_id).initializer {
                    self.analyze_expr(init, &mut state);
                    self.set_dynamic(&mut state, var_id, init);
                }
                Flow::fallthrough(state)
            }
            StmtKind::DeclMulti(vars, expr) => {
                self.analyze_expr(expr, &mut state);
                for var_id in vars.iter().flatten() {
                    self.set_dynamic(&mut state, *var_id, expr);
                }
                Flow::fallthrough(state)
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.analyze_stmts(block.stmts, state)
            }
            StmtKind::Loop(block, source) => self.analyze_stmts(loop_stmts(block, source), state),
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.analyze_expr(cond, &mut state);
                let then_flow = self.analyze_stmt(then_stmt, state.clone());
                let else_flow = match else_stmt {
                    Some(else_stmt) => self.analyze_stmt(else_stmt, state),
                    None => Flow::fallthrough(state),
                };
                then_flow.merge(else_flow)
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr, &mut state);
                try_stmt.clauses.iter().fold(Flow::default(), |flow, clause| {
                    flow.merge(self.analyze_stmts(clause.block.stmts, state.clone()))
                })
            }
            StmtKind::Expr(expr) => {
                self.analyze_expr(expr, &mut state);
                Flow::fallthrough(state)
            }
            StmtKind::Revert(expr) => {
                self.analyze_expr(expr, &mut state);
                Flow::default()
            }
            StmtKind::Emit(expr) => {
                self.analyze_expr(expr, &mut state);
                state.writes.clear();
                Flow::fallthrough(state)
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.analyze_expr(expr, &mut state);
                }
                Flow { fallthrough: None, returned: Some(state) }
            }
            _ => Flow::fallthrough(state),
        }
    }

    fn analyze_expr(&mut self, expr: &'gcx Expr<'gcx>, state: &mut WriteState) {
        let _ = expr.visit(&mut |e| {
            match &e.kind {
                ExprKind::Assign(lhs, op, rhs) => {
                    let dynamic = self.is_dynamic(state, rhs);
                    if dynamic || op.is_some_and(|op| is_arithmetic_op(op.kind)) {
                        self.record_writes(state, lhs);
                    }
                    if let Some(local) = lhs_local_var(&self.gcx.hir, lhs) {
                        self.set_dynamic(state, local, rhs);
                    }
                }
                ExprKind::Unary(op, inner) if is_inc_dec_op(op.kind) => {
                    self.record_writes(state, inner);
                }
                ExprKind::Call(callee, args, _) => {
                    if let Some(callee_id) = dispatched_function(self.gcx, self.contract_id, callee)
                    {
                        self.analyze_call(callee_id, args, state);
                    }
                }
                _ => {}
            }
            ControlFlow::<()>::Continue(())
        });
    }

    /// Inlines `callee_id` with its parameters marked dynamic when the matching argument is; the
    /// callee's pending writes (and any `emit` clearing them) flow back into the caller.
    fn analyze_call(
        &mut self,
        callee_id: FunctionId,
        args: &hir::CallArgs<'gcx>,
        state: &mut WriteState,
    ) {
        let callee_state = WriteState {
            dynamic: self
                .gcx
                .hir
                .function(callee_id)
                .parameters
                .iter()
                .zip(args.exprs())
                .filter(|(_, arg)| self.is_dynamic(state, arg))
                .map(|(&param, _)| param)
                .collect(),
            writes: state.writes.clone(),
        };
        if let Some(merged) = self.analyze_function(callee_id, callee_state).merged() {
            state.writes = merged.writes;
        }
    }

    fn record_writes(&self, state: &mut WriteState, lhs: &Expr<'_>) {
        for var_id in state_lhs_vars(&self.gcx.hir, lhs) {
            if self.targets.contains(&var_id) {
                state.writes.push(StateWrite { var_id, span: lhs.span });
            }
        }
    }

    fn set_dynamic(&self, state: &mut WriteState, var_id: VariableId, value: &Expr<'_>) {
        if self.is_dynamic(state, value) {
            state.dynamic.insert(var_id);
        } else {
            state.dynamic.remove(&var_id);
        }
    }

    /// True unless `expr` is a compile-time constant: reads of mutable state, dynamic locals,
    /// calls and `block`/`msg`/`tx` members are all dynamic.
    fn is_dynamic(&self, state: &WriteState, expr: &Expr<'_>) -> bool {
        expr.visit(&mut |e| {
            let dynamic = match &e.kind {
                ExprKind::Call(..) => true,
                ExprKind::Member(base, _) => {
                    builtins(base).any(|b| matches!(b, Builtin::Block | Builtin::Msg | Builtin::Tx))
                }
                _ => underlying_var(e).is_some_and(|var_id| {
                    let var = self.gcx.hir.variable(var_id);
                    state.dynamic.contains(&var_id)
                        || (var.kind.is_state() && !var.is_constant() && !var.is_immutable())
                }),
            };
            if dynamic { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
        })
        .is_break()
    }
}
