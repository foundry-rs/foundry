use super::MissingEventsArithmetic;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            branch_always_exits, builtins, function_ids, is_builtin, is_msg_sender,
            is_require_or_assert, lhs_local_var, state_lhs_vars, underlying_var,
        },
    },
};
use solar::{
    ast::{ContractKind, StateMutability, Visibility},
    interface::{Span, kw, sym},
    sema::{
        builtins::Builtin,
        hir::{
            self, BinOpKind, ElementaryType, Expr, ExprKind, FunctionId, StmtKind, TypeKind,
            UnOpKind, VariableId, Visit,
        },
    },
};
use std::{
    collections::{HashMap, HashSet},
    iter,
    ops::ControlFlow,
};

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

        let candidates: HashSet<_> = contract
            .variables()
            .filter(|&id| {
                let var = hir.variable(id);
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

        let (protected, unprotected): (Vec<_>, Vec<_>) = contract
            .all_functions()
            .filter(|&id| is_external_function(hir.function(id)))
            .partition(|&id| is_protected(hir, id));
        let entry_points: Vec<_> = protected
            .into_iter()
            .filter(|&id| {
                !matches!(
                    hir.function(id).state_mutability,
                    StateMutability::Pure | StateMutability::View
                )
            })
            .collect();
        if entry_points.is_empty() {
            return;
        }

        // Candidates that flow into arithmetic reachable from an unprotected function.
        let mut uses = UseAnalyzer {
            hir,
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
            let mut analyzer = WriteAnalyzer { hir, targets: &uses.used, call_stack: Vec::new() };
            let mut emitted = HashSet::new();
            for write in analyzer.analyze_entry_point(func_id) {
                if !emitted.insert(write.var_id) {
                    continue;
                }
                let name = hir
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

fn is_external_function(func: &hir::Function<'_>) -> bool {
    func.kind.is_function()
        && matches!(func.visibility, Visibility::Public | Visibility::External)
        && !func.is_constructor()
        && !func.is_special()
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

/// Runs `f` on every top-level expression (statement expressions, conditions, initializers, ...)
/// reachable from the visited statements, stopping when `f` returns `true`.
struct ExprVisitor<'hir, F> {
    hir: &'hir hir::Hir<'hir>,
    f: F,
}

impl<'hir, F: FnMut(&'hir Expr<'hir>) -> bool> Visit<'hir> for ExprVisitor<'hir, F> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<()> {
        if (self.f)(expr) { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
    }
}

// --- Access-control detection -----------------------------------------------------------------

/// True when the function or one of its modifiers contains a dominating access check.
fn is_protected<'hir>(hir: &'hir hir::Hir<'hir>, func_id: FunctionId) -> bool {
    let func = hir.function(func_id);
    func.modifiers
        .iter()
        .filter_map(|modifier| modifier.id.as_function())
        .chain(iter::once(func_id))
        .any(|id| has_access_guard(hir, id, &mut HashSet::new()))
}

/// Whether the top-level statements of `func_id` contain an access check. Bodyless declarations
/// (interface functions, virtual modifiers) fall back to a name heuristic.
fn has_access_guard<'hir>(
    hir: &'hir hir::Hir<'hir>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    if !seen.insert(func_id) {
        return false;
    }
    let func = hir.function(func_id);
    match func.body {
        Some(body) => body.stmts.iter().any(|stmt| stmt_is_access_guard(hir, stmt, seen)),
        None => {
            func.returns.is_empty()
                && func.name.is_some_and(|name| name_looks_like_access_control(name.as_str()))
        }
    }
}

fn stmt_is_access_guard<'hir>(
    hir: &'hir hir::Hir<'hir>,
    stmt: &hir::Stmt<'_>,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    match stmt.kind {
        StmtKind::If(cond, then_stmt, else_stmt) => match access_check_polarity(hir, cond) {
            Some(false) => branch_always_exits(then_stmt),
            Some(true) => else_stmt.is_some_and(branch_always_exits),
            None => false,
        },
        StmtKind::Expr(expr) => match &expr.peel_parens().kind {
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                let cond = args.exprs().next();
                cond.is_some_and(|cond| access_check_polarity(hir, cond) == Some(true))
            }
            ExprKind::Call(callee, ..) => {
                function_ids(callee).any(|id| has_access_guard(hir, id, seen))
            }
            _ => false,
        },
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            block.stmts.iter().any(|stmt| stmt_is_access_guard(hir, stmt, seen))
        }
        _ => false,
    }
}

/// `Some(true)` when `expr` holding means the caller is authorized, `Some(false)` when it means
/// the caller is *not* authorized, `None` when `expr` is not an access check.
fn access_check_polarity<'hir>(hir: &'hir hir::Hir<'hir>, expr: &Expr<'_>) -> Option<bool> {
    match &expr.peel_parens().kind {
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            access_check_polarity(hir, inner).map(|polarity| !polarity)
        }
        ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
            // `a && b` is authorized as soon as one side is; `a || b` is unauthorized as soon as
            // one side is. The opposite polarity needs both sides.
            let dominant = op.kind == BinOpKind::And;
            let (lhs, rhs) = (access_check_polarity(hir, lhs), access_check_polarity(hir, rhs));
            if lhs == Some(dominant) || rhs == Some(dominant) {
                Some(dominant)
            } else if lhs == Some(!dominant) && rhs == Some(!dominant) {
                Some(!dominant)
            } else {
                None
            }
        }
        ExprKind::Binary(lhs, op, rhs)
            if matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne)
                && (compares_sender_to_authority(hir, lhs, rhs)
                    || compares_sender_to_authority(hir, rhs, lhs)) =>
        {
            Some(op.kind == BinOpKind::Eq)
        }
        _ => compares_sender_to_authority(hir, expr, expr).then_some(true),
    }
}

/// `sender` reads `msg.sender`/`tx.origin` (possibly through a helper) and `authority` reads
/// state or calls a user function other than a sender accessor.
fn compares_sender_to_authority<'hir>(
    hir: &'hir hir::Hir<'hir>,
    sender: &Expr<'_>,
    authority: &Expr<'_>,
) -> bool {
    expr_reads_sender(hir, sender, &mut HashSet::new())
        && authority
            .visit(&mut |e| {
                let is_authority = match &e.kind {
                    ExprKind::Call(callee, ..) => function_ids(callee).any(|id| {
                        hir.function(id).name.is_some_and(|name| {
                            !matches!(
                                name.as_str().to_ascii_lowercase().as_str(),
                                "_msgsender" | "msgsender" | "sender"
                            )
                        })
                    }),
                    _ => underlying_var(e).is_some_and(|v| hir.variable(v).kind.is_state()),
                };
                if is_authority { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
            })
            .is_break()
}

fn expr_reads_sender<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &Expr<'_>,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    expr.visit(&mut |e| {
        let reads = is_sender_member(e)
            || matches!(&e.kind, ExprKind::Call(callee, ..)
                if function_ids(callee).any(|id| function_reads_sender(hir, id, seen)));
        if reads { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
    })
    .is_break()
}

fn function_reads_sender<'hir>(
    hir: &'hir hir::Hir<'hir>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    if !seen.insert(func_id) {
        return false;
    }
    let mut visitor = ExprVisitor { hir, f: |expr| expr_reads_sender(hir, expr, seen) };
    hir.function(func_id).body.is_some_and(|body| {
        body.stmts.iter().any(|stmt| visitor.visit_stmt(stmt).is_break())
    })
}

/// `msg.sender` or `tx.origin`.
fn is_sender_member(expr: &Expr<'_>) -> bool {
    is_msg_sender(expr)
        || matches!(&expr.peel_parens().kind, ExprKind::Member(base, name)
            if name.name == kw::Origin && is_builtin(base, sym::tx))
}

fn name_looks_like_access_control(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "auth" | "requiresauth" | "restricted")
        || ["onlyowner", "onlyrole", "checkowner", "_checkowner", "checkrole", "_checkrole"]
            .iter()
            .any(|prefix| lower.starts_with(prefix))
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
struct UseAnalyzer<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    targets: &'a HashSet<VariableId>,
    mode: Mode,
    /// Target state variables each local may currently hold.
    taint: HashMap<VariableId, HashSet<VariableId>>,
    used: HashSet<VariableId>,
    returned: HashSet<VariableId>,
    call_stack: Vec<FunctionId>,
}

impl<'hir> UseAnalyzer<'_, 'hir> {
    fn analyze_function(&mut self, func_id: FunctionId) {
        if self.call_stack.contains(&func_id) {
            return;
        }
        let Some(body) = self.hir.function(func_id).body else { return };
        self.call_stack.push(func_id);
        for stmt in body.stmts {
            let _ = self.visit_stmt(stmt);
        }
        self.call_stack.pop();
    }

    /// Analyzes `callee_id` with its parameters bound to the sources of `args`, restoring the
    /// caller's taint afterwards.
    fn analyze_call(&mut self, callee_id: FunctionId, args: &hir::CallArgs<'hir>) {
        if self.call_stack.contains(&callee_id) {
            return;
        }
        let params = self
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
    fn sources(&mut self, expr: &Expr<'hir>) -> HashSet<VariableId> {
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
            if let ExprKind::Call(callee, args, _) = &e.kind {
                for callee_id in function_ids(callee) {
                    out.extend(self.return_sources(callee_id, args));
                }
            }
            ControlFlow::<()>::Continue(())
        });
        out
    }

    fn return_sources(
        &mut self,
        callee_id: FunctionId,
        args: &hir::CallArgs<'hir>,
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

impl<'hir> Visit<'hir> for UseAnalyzer<'_, 'hir> {
    type BreakValue = solar::interface::data_structures::Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.hir.variable(var_id).initializer {
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

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            ExprKind::Assign(lhs, _, rhs) => {
                if let Some(local) = lhs_local_var(self.hir, lhs) {
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
                for callee_id in function_ids(callee) {
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
struct WriteAnalyzer<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    targets: &'a HashSet<VariableId>,
    call_stack: Vec<FunctionId>,
}

impl<'hir> WriteAnalyzer<'_, 'hir> {
    fn analyze_entry_point(&mut self, func_id: FunctionId) -> Vec<StateWrite> {
        let func = self.hir.function(func_id);
        let state = WriteState {
            dynamic: func.parameters.iter().copied().collect(),
            writes: Vec::new(),
        };
        let mut state = self.analyze_function(func_id, state).merged();
        // Modifier code after `_` runs once the body finished, innermost modifier first, and may
        // still emit for the body's writes.
        for modifier in func.modifiers.iter().rev() {
            let Some(body) = modifier.id.as_function().and_then(|id| self.hir.function(id).body)
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
        let Some(body) = self.hir.function(func_id).body else { return Flow::fallthrough(state) };
        self.call_stack.push(func_id);
        let flow = self.analyze_stmts(body.stmts, state);
        self.call_stack.pop();
        flow
    }

    fn analyze_stmts(&mut self, stmts: &'hir [hir::Stmt<'hir>], state: WriteState) -> Flow {
        let mut flow = Flow::fallthrough(state);
        for stmt in stmts {
            let Some(state) = flow.fallthrough.take() else { break };
            let next = self.analyze_stmt(stmt, state);
            flow.fallthrough = next.fallthrough;
            flow.returned = merge(flow.returned, next.returned);
        }
        flow
    }

    fn analyze_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>, mut state: WriteState) -> Flow {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.hir.variable(var_id).initializer {
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
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                self.analyze_stmts(block.stmts, state)
            }
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

    fn analyze_expr(&mut self, expr: &'hir Expr<'hir>, state: &mut WriteState) {
        let _ = expr.visit(&mut |e| {
            match &e.kind {
                ExprKind::Assign(lhs, op, rhs) => {
                    let dynamic = self.is_dynamic(state, rhs);
                    if dynamic || op.is_some_and(|op| is_arithmetic_op(op.kind)) {
                        self.record_writes(state, lhs);
                    }
                    if let Some(local) = lhs_local_var(self.hir, lhs) {
                        self.set_dynamic(state, local, rhs);
                    }
                }
                ExprKind::Unary(op, inner) if is_inc_dec_op(op.kind) => {
                    self.record_writes(state, inner);
                }
                ExprKind::Call(callee, args, _) => {
                    for callee_id in function_ids(callee) {
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
        args: &hir::CallArgs<'hir>,
        state: &mut WriteState,
    ) {
        let callee_state = WriteState {
            dynamic: self
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
        for var_id in state_lhs_vars(self.hir, lhs) {
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
                    let var = self.hir.variable(var_id);
                    state.dynamic.contains(&var_id)
                        || (var.kind.is_state() && !var.is_constant() && !var.is_immutable())
                }),
            };
            if dynamic { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
        })
        .is_break()
    }
}
