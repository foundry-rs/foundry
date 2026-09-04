use super::MissingEventsAccessControl;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            branch_always_exits, for_each_lhs_var, function_ids, is_builtin, is_msg_sender,
            is_require_or_assert, lhs_local_var, referenced_item, underlying_var,
        },
    },
};
use solar::{
    ast::{ContractKind, DataLocation, LitKind, StateMutability, Visibility},
    interface::{Span, data_structures::Never, kw, sym},
    sema::hir::{
        self, EventId, Expr, ExprKind, FunctionId, ItemId, Stmt, StmtKind, VariableId, Visit,
    },
};
use std::{
    collections::{HashMap, HashSet},
    iter,
    ops::ControlFlow,
};

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

        // Every state variable some access check in the contract depends on.
        let functions: Vec<_> = contract.all_functions().collect();
        let targets: HashSet<_> = functions.iter().flat_map(|&id| guard_vars(hir, id)).collect();
        if targets.is_empty() {
            return;
        }

        for func_id in functions {
            let func = hir.function(func_id);
            let is_entry_point = func.kind.is_function()
                && matches!(func.visibility, Visibility::Public | Visibility::External)
                && !func.is_constructor()
                && !func.is_special()
                && !matches!(func.state_mutability, StateMutability::Pure | StateMutability::View);
            if !is_entry_point || !is_protected(hir, func_id) {
                continue;
            }

            let guard_targets = guard_vars(hir, func_id);
            let mut analyzer = WriteAnalyzer {
                hir,
                targets: &targets,
                guard_targets: &guard_targets,
                state: State {
                    taint: func
                        .parameters
                        .iter()
                        .map(|&p| (p, Sources::from([Source::Var(p)])))
                        .collect(),
                    ..Default::default()
                },
                call_stack: Vec::new(),
            };
            analyzer.analyze_function(func_id);

            let mut emitted = HashSet::new();
            for write in analyzer.state.writes {
                if write.evented || !emitted.insert(write.var_id) {
                    continue;
                }
                let name = hir
                    .variable(write.var_id)
                    .name
                    .map_or_else(|| "state variable".to_string(), |name| name.to_string());
                ctx.emit_with_msg(
                    &MISSING_EVENTS_ACCESS_CONTROL,
                    write.span,
                    format!("`{name}` is changed without an event but is used for access control"),
                );
            }
        }
    }
}

// --- Generic HIR walking ----------------------------------------------------------------------

/// Runs `f` on every statement (pre-order, nested ones included) until it breaks.
struct StmtVisitor<'hir, F> {
    hir: &'hir hir::Hir<'hir>,
    f: F,
}

impl<'hir, F: FnMut(&'hir Stmt<'hir>) -> ControlFlow<()>> Visit<'hir> for StmtVisitor<'hir, F> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<()> {
        (self.f)(stmt)?;
        self.walk_stmt(stmt)
    }
}

fn visit_stmts<'hir>(
    hir: &'hir hir::Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
    f: impl FnMut(&'hir Stmt<'hir>) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let mut visitor = StmtVisitor { hir, f };
    stmts.iter().try_for_each(|stmt| visitor.visit_stmt(stmt))
}

/// The expression directly owned by `stmt` (nested statements excluded).
fn stmt_expr<'hir>(hir: &'hir hir::Hir<'hir>, stmt: &'hir Stmt<'hir>) -> Option<&'hir Expr<'hir>> {
    match stmt.kind {
        StmtKind::DeclSingle(var_id) => hir.variable(var_id).initializer,
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Expr(expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr)
        | StmtKind::Return(Some(expr))
        | StmtKind::If(expr, ..) => Some(expr),
        StmtKind::Try(try_stmt) => Some(&try_stmt.expr),
        _ => None,
    }
}

/// The statements of `body` that run before the `_` placeholder (all of them for functions).
fn prefix_stmts<'hir>(body: hir::Block<'hir>) -> &'hir [Stmt<'hir>] {
    let end = body
        .stmts
        .iter()
        .position(|stmt| matches!(stmt.kind, StmtKind::Placeholder))
        .unwrap_or(body.stmts.len());
    &body.stmts[..end]
}

/// Calls `f` on every index and slice bound along the spine of an lvalue.
fn for_each_lhs_index<'hir>(expr: &'hir Expr<'hir>, f: &mut impl FnMut(&'hir Expr<'hir>)) {
    match &expr.peel_parens().kind {
        ExprKind::Index(base, index) => {
            for_each_lhs_index(base, f);
            if let Some(index) = index {
                f(index);
            }
        }
        ExprKind::Slice(base, start, end) => {
            for_each_lhs_index(base, f);
            for bound in start.iter().chain(end) {
                f(bound);
            }
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) | ExprKind::Unary(_, base) => {
            for_each_lhs_index(base, f)
        }
        ExprKind::Tuple(exprs) => exprs.iter().flatten().for_each(|e| for_each_lhs_index(e, f)),
        _ => {}
    }
}

// --- Access-control detection -----------------------------------------------------------------

/// `msg.sender` or `tx.origin`.
fn is_sender_member(expr: &Expr<'_>) -> bool {
    is_msg_sender(expr)
        || matches!(&expr.peel_parens().kind, ExprKind::Member(base, name)
            if name.name == kw::Origin && is_builtin(base, sym::tx))
}

fn looks_like_access_control(func: &hir::Function<'_>) -> bool {
    let Some(name) = func.name else { return false };
    let lower = name.as_str().to_ascii_lowercase();
    matches!(lower.as_str(), "auth" | "requiresauth" | "restricted")
        || ["only", "check", "_check"].iter().any(|prefix| {
            ["admin", "guardian", "manager", "owner", "role"]
                .iter()
                .any(|role| lower.starts_with(&format!("{prefix}{role}")))
        })
}

/// Locals initialized or assigned from a value that reads `msg.sender`.
fn sender_aliases<'hir>(
    hir: &'hir hir::Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
) -> HashSet<VariableId> {
    let mut aliases = HashSet::new();
    let _ = visit_stmts(hir, stmts, |stmt| {
        let (var_id, value) = match stmt.kind {
            StmtKind::DeclSingle(var_id) => (Some(var_id), hir.variable(var_id).initializer),
            StmtKind::Expr(expr) => match &expr.peel_parens().kind {
                ExprKind::Assign(lhs, _, rhs) => (lhs_local_var(hir, lhs), Some(*rhs)),
                _ => (None, None),
            },
            _ => (None, None),
        };
        if let Some(var_id) = var_id
            && let Some(value) = value
            && expr_reads_sender(hir, value, &mut HashSet::new(), &aliases)
        {
            aliases.insert(var_id);
        }
        ControlFlow::Continue(())
    });
    aliases
}

fn expr_reads_sender<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &Expr<'_>,
    seen: &mut HashSet<FunctionId>,
    aliases: &HashSet<VariableId>,
) -> bool {
    expr.visit(&mut |e| {
        let reads = is_sender_member(e)
            || underlying_var(e).is_some_and(|v| aliases.contains(&v))
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
    seen.insert(func_id)
        && hir.function(func_id).body.is_some_and(|body| {
            visit_stmts(hir, body.stmts, |stmt| {
                let reads = stmt_expr(hir, stmt)
                    .is_some_and(|expr| expr_reads_sender(hir, expr, seen, &HashSet::new()));
                if reads { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
            })
            .is_break()
        })
}

/// State variables read by `expr`, following calls into user functions.
fn expr_state_vars<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &Expr<'_>,
    seen: &mut HashSet<FunctionId>,
    out: &mut HashSet<VariableId>,
) {
    let _ = expr.visit(&mut |e| {
        if let Some(var_id) = underlying_var(e)
            && hir.variable(var_id).kind.is_state()
        {
            out.insert(var_id);
        }
        if let ExprKind::Call(callee, ..) = &e.kind {
            for callee_id in function_ids(callee) {
                function_state_vars(hir, callee_id, seen, out);
            }
        }
        ControlFlow::<()>::Continue(())
    });
}

fn function_state_vars<'hir>(
    hir: &'hir hir::Hir<'hir>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
    out: &mut HashSet<VariableId>,
) {
    if seen.insert(func_id)
        && let Some(body) = hir.function(func_id).body
    {
        let _ = visit_stmts(hir, body.stmts, |stmt| {
            if let Some(expr) = stmt_expr(hir, stmt) {
                expr_state_vars(hir, expr, seen, out);
            }
            ControlFlow::Continue(())
        });
    }
}

/// A condition comparing the caller against state.
fn is_access_check<'hir>(
    hir: &'hir hir::Hir<'hir>,
    cond: &Expr<'_>,
    aliases: &HashSet<VariableId>,
) -> bool {
    let mut state_vars = HashSet::new();
    expr_state_vars(hir, cond, &mut HashSet::new(), &mut state_vars);
    !state_vars.is_empty() && expr_reads_sender(hir, cond, &mut HashSet::new(), aliases)
}

/// An `if` whose access-check condition sends one branch into a revert/return.
fn is_guarding_if<'hir>(
    hir: &'hir hir::Hir<'hir>,
    stmt: &'hir Stmt<'hir>,
    aliases: &HashSet<VariableId>,
) -> Option<&'hir Expr<'hir>> {
    let StmtKind::If(cond, then_stmt, else_stmt) = stmt.kind else { return None };
    (is_access_check(hir, cond, aliases)
        && (branch_always_exits(then_stmt) || else_stmt.is_some_and(branch_always_exits)))
    .then_some(cond)
}

/// True when the function or one of its modifiers contains an access check.
fn is_protected<'hir>(hir: &'hir hir::Hir<'hir>, func_id: FunctionId) -> bool {
    modifiers_and_self(hir, func_id).any(|id| has_access_guard(hir, id, &mut HashSet::new()))
}

fn modifiers_and_self<'hir>(
    hir: &'hir hir::Hir<'hir>,
    func_id: FunctionId,
) -> impl Iterator<Item = FunctionId> + 'hir {
    hir.function(func_id)
        .modifiers
        .iter()
        .filter_map(|modifier| modifier.id.as_function())
        .chain(iter::once(func_id))
}

/// Whether `func_id` (up to its `_` placeholder, if any) checks the caller: a guarding `if`, a
/// `require`/`assert` on an access check, or a call into a function that does. Bodyless
/// declarations fall back to a name heuristic.
fn has_access_guard<'hir>(
    hir: &'hir hir::Hir<'hir>,
    func_id: FunctionId,
    seen: &mut HashSet<FunctionId>,
) -> bool {
    if !seen.insert(func_id) {
        return false;
    }
    let func = hir.function(func_id);
    let Some(body) = func.body else { return looks_like_access_control(func) };
    let stmts = prefix_stmts(body);
    let aliases = sender_aliases(hir, stmts);
    visit_stmts(hir, stmts, |stmt| {
        let guard = match stmt.kind {
            StmtKind::If(..) => is_guarding_if(hir, stmt, &aliases).is_some(),
            _ => stmt_expr(hir, stmt).is_some_and(|expr| {
                expr.visit(&mut |e| {
                    let guard = match &e.kind {
                        ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                            let cond = args.exprs().next();
                            cond.is_some_and(|cond| is_access_check(hir, cond, &aliases))
                        }
                        ExprKind::Call(callee, ..) => function_ids(callee).any(|id| {
                            looks_like_access_control(hir.function(id))
                                || has_access_guard(hir, id, seen)
                        }),
                        _ => false,
                    };
                    if guard { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
                })
                .is_break()
            }),
        };
        if guard { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
    })
    .is_break()
}

/// State variables the access checks of `func_id` and its modifiers (up to `_`) depend on.
fn guard_vars<'hir>(hir: &'hir hir::Hir<'hir>, func_id: FunctionId) -> HashSet<VariableId> {
    let mut out = HashSet::new();
    for id in modifiers_and_self(hir, func_id) {
        let Some(body) = hir.function(id).body else { continue };
        let stmts = prefix_stmts(body);
        let aliases = sender_aliases(hir, stmts);
        let mut seen = HashSet::from([id]);
        let _ = visit_stmts(hir, stmts, |stmt| {
            if let Some(cond) = is_guarding_if(hir, stmt, &aliases) {
                expr_state_vars(hir, cond, &mut seen, &mut out);
            } else if let Some(expr) = stmt_expr(hir, stmt) {
                let _ = expr.visit(&mut |e| {
                    match &e.kind {
                        ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                            if let Some(cond) = args.exprs().next()
                                && is_access_check(hir, cond, &aliases)
                            {
                                expr_state_vars(hir, cond, &mut seen, &mut out);
                            }
                        }
                        ExprKind::Call(callee, ..) => {
                            for callee_id in function_ids(callee) {
                                if looks_like_access_control(hir.function(callee_id))
                                    || has_access_guard(hir, callee_id, &mut HashSet::new())
                                {
                                    function_state_vars(hir, callee_id, &mut seen, &mut out);
                                }
                            }
                        }
                        _ => {}
                    }
                    ControlFlow::<()>::Continue(())
                });
            }
            ControlFlow::Continue(())
        });
    }
    out
}

// --- Writes without events --------------------------------------------------------------------

/// Where a written value may come from: an entry-point parameter or state variable, or the caller.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Source {
    Var(VariableId),
    Sender,
}

type Sources = HashSet<Source>;

#[derive(Clone)]
struct StateWrite {
    var_id: VariableId,
    span: Span,
    sources: Sources,
    /// The written value is a literal zero/false, so no source is needed for an event to match.
    fixed_clear: bool,
    evented: bool,
}

#[derive(Clone, Default)]
struct State {
    /// Sources each local may currently hold.
    taint: HashMap<VariableId, Sources>,
    /// Storage-pointer locals and the state variable they alias.
    storage_aliases: HashMap<VariableId, VariableId>,
    writes: Vec<StateWrite>,
}

/// Collects writes to `targets` reachable from an entry point and marks those an `emit` covers.
struct WriteAnalyzer<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    targets: &'a HashSet<VariableId>,
    /// Targets checked by this entry point's own guards; clearing one of them is reportable even
    /// when the written value carries no source.
    guard_targets: &'a HashSet<VariableId>,
    state: State,
    call_stack: Vec<FunctionId>,
}

impl<'hir> WriteAnalyzer<'_, 'hir> {
    fn analyze_function(&mut self, func_id: FunctionId) {
        if self.call_stack.contains(&func_id) {
            return;
        }
        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };
        self.call_stack.push(func_id);
        for modifier in func.modifiers {
            if let Some(modifier_id) = modifier.id.as_function() {
                let _ = self.visit_call_args(&modifier.args);
                self.analyze_call(modifier_id, &modifier.args);
            }
        }
        for stmt in body.stmts {
            let _ = self.visit_stmt(stmt);
        }
        self.call_stack.pop();
    }

    /// Inlines `callee_id` with its parameters bound to the sources of `args`; locals and storage
    /// aliases are callee-private, pending writes flow back to the caller.
    fn analyze_call(&mut self, callee_id: FunctionId, args: &hir::CallArgs<'hir>) {
        let params = self
            .hir
            .function(callee_id)
            .parameters
            .iter()
            .zip(args.exprs())
            .filter_map(|(&param, arg)| {
                let sources = self.value_sources(arg);
                (!sources.is_empty()).then_some((param, sources))
            })
            .collect();
        let saved_taint = std::mem::replace(&mut self.state.taint, params);
        let saved_aliases = std::mem::take(&mut self.state.storage_aliases);
        self.analyze_function(callee_id);
        self.state.taint = saved_taint;
        self.state.storage_aliases = saved_aliases;
    }

    /// Sources flowing into `expr`: `msg.sender`, state variables and tainted locals.
    fn value_sources(&self, expr: &Expr<'_>) -> Sources {
        let mut out = Sources::new();
        let _ = expr.visit(&mut |e| {
            if is_sender_member(e) {
                out.insert(Source::Sender);
            }
            if let Some(var_id) = underlying_var(e) {
                if self.hir.variable(var_id).kind.is_state() {
                    out.insert(Source::Var(var_id));
                }
                if let Some(sources) = self.state.taint.get(&var_id) {
                    out.extend(sources);
                }
            }
            ControlFlow::<()>::Continue(())
        });
        out
    }

    /// State variables written through `lhs`, resolving storage pointers to their roots.
    fn lhs_state_vars(&self, lhs: &Expr<'_>) -> Vec<VariableId> {
        let mut vars = Vec::new();
        for_each_lhs_var(lhs, &mut |var_id| {
            let root = if self.hir.variable(var_id).kind.is_state() {
                Some(var_id)
            } else {
                self.state.storage_aliases.get(&var_id).copied()
            };
            if let Some(root) = root
                && !vars.contains(&root)
            {
                vars.push(root);
            }
        });
        vars
    }

    fn record_writes(&mut self, lhs: &Expr<'_>, sources: &Sources, fixed_clear: bool) {
        for var_id in self.lhs_state_vars(lhs) {
            if self.targets.contains(&var_id)
                && (!sources.is_empty() || (fixed_clear && self.guard_targets.contains(&var_id)))
            {
                self.state.writes.push(StateWrite {
                    var_id,
                    span: lhs.span,
                    sources: sources.clone(),
                    fixed_clear,
                    evented: false,
                });
            }
        }
    }

    fn set_taint(&mut self, var_id: VariableId, sources: Sources) {
        if sources.is_empty() {
            self.state.taint.remove(&var_id);
        } else {
            self.state.taint.insert(var_id, sources);
        }
    }

    /// Records `var_id = value`, tracking which state variable a storage pointer aliases.
    fn set_local(&mut self, var_id: VariableId, sources: Sources, value: &Expr<'_>) {
        self.set_taint(var_id, sources);
        let root = (self.hir.variable(var_id).data_location == Some(DataLocation::Storage))
            .then(|| self.lhs_state_vars(value).into_iter().next())
            .flatten();
        match root {
            Some(root) => self.state.storage_aliases.insert(var_id, root),
            None => self.state.storage_aliases.remove(&var_id),
        };
    }

    /// Marks pending writes covered by `emit`: the event must mention the variable and share a
    /// source with the write (or the write must be a fixed clear).
    fn mark_event(&mut self, expr: &Expr<'_>) {
        let Some(event_id) = emitted_event_id(expr) else { return };
        let event_sources = self.value_sources(expr);
        let hir = self.hir;
        for write in &mut self.state.writes {
            if !write.evented
                && (write.fixed_clear || !write.sources.is_disjoint(&event_sources))
                && event_mentions_state_var(hir, event_id, write.var_id)
            {
                write.evented = true;
            }
        }
    }
}

impl<'hir> Visit<'hir> for WriteAnalyzer<'_, 'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Never> {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.hir.variable(var_id).initializer {
                    self.visit_expr(init)?;
                    let sources = self.value_sources(init);
                    self.set_local(var_id, sources, init);
                }
            }
            StmtKind::DeclMulti(vars, expr) => {
                self.visit_expr(expr)?;
                let sources = self.value_sources(expr);
                for var_id in vars.iter().flatten() {
                    self.set_taint(*var_id, sources.clone());
                }
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.visit_expr(cond)?;
                let base = self.state.clone();
                self.visit_stmt(then_stmt)?;
                let then_state = std::mem::replace(&mut self.state, base.clone());
                if let Some(else_stmt) = else_stmt {
                    self.visit_stmt(else_stmt)?;
                }
                let else_state = std::mem::take(&mut self.state);
                self.state = merge_branches(
                    base,
                    then_state,
                    else_state,
                    branch_always_exits(then_stmt),
                    else_stmt.is_some_and(branch_always_exits),
                );
            }
            StmtKind::Emit(expr) => {
                self.visit_expr(expr)?;
                self.mark_event(expr);
            }
            _ => return self.walk_stmt(stmt),
        }
        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Never> {
        match &expr.kind {
            ExprKind::Assign(lhs, op, rhs) => {
                self.visit_expr(rhs)?;
                self.visit_expr(lhs)?;
                let mut sources = self.value_sources(rhs);
                for_each_lhs_index(lhs, &mut |index| sources.extend(self.value_sources(index)));
                if op.is_some() {
                    sources.extend(self.value_sources(lhs));
                }
                self.record_writes(lhs, &sources, is_zero_value(rhs));
                if let Some(local) = lhs_local_var(self.hir, lhs) {
                    self.set_local(local, sources, rhs);
                }
                ControlFlow::Continue(())
            }
            ExprKind::Delete(inner) => {
                let mut sources = Sources::new();
                for_each_lhs_index(inner, &mut |index| sources.extend(self.value_sources(index)));
                self.record_writes(inner, &sources, true);
                self.walk_expr(expr)
            }
            ExprKind::Call(callee, args, _) => {
                self.walk_expr(expr)?;
                for callee_id in function_ids(callee) {
                    self.analyze_call(callee_id, args);
                }
                ControlFlow::Continue(())
            }
            _ => self.walk_expr(expr),
        }
    }
}

/// Joins the two arms of an `if`: a pending write stays covered only if both arms emitted for it,
/// while taint and aliases come from whichever arms can continue past the `if`.
fn merge_branches(
    base: State,
    then_state: State,
    else_state: State,
    then_exits: bool,
    else_exits: bool,
) -> State {
    let mut writes = base.writes;
    for (i, write) in writes.iter_mut().enumerate() {
        write.evented = then_state.writes[i].evented && else_state.writes[i].evented;
    }
    let n = writes.len();
    writes.extend_from_slice(&then_state.writes[n..]);
    writes.extend_from_slice(&else_state.writes[n..]);

    let (taint, storage_aliases) = match (then_exits, else_exits) {
        (true, true) => (base.taint, base.storage_aliases),
        (true, false) => (else_state.taint, else_state.storage_aliases),
        (false, true) => (then_state.taint, then_state.storage_aliases),
        (false, false) => {
            let mut taint = then_state.taint;
            for (var_id, sources) in else_state.taint {
                taint.entry(var_id).or_default().extend(sources);
            }
            let storage_aliases = then_state
                .storage_aliases
                .into_iter()
                .filter(|(alias, root)| else_state.storage_aliases.get(alias) == Some(root))
                .collect();
            (taint, storage_aliases)
        }
    };
    State { taint, storage_aliases, writes }
}

/// `0`, `false`, `address(0)`, `payable(0)` and casts/negations thereof.
fn is_zero_value(expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => match &lit.kind {
            LitKind::Number(value) => value.is_zero(),
            LitKind::Address(value) => value.is_zero(),
            LitKind::Bool(value) => !value,
            _ => false,
        },
        ExprKind::Call(_, args, _) => {
            let mut exprs = args.exprs();
            exprs.len() == 1 && exprs.next().is_some_and(is_zero_value)
        }
        ExprKind::Unary(_, inner) | ExprKind::Payable(inner) => is_zero_value(inner),
        _ => false,
    }
}

fn emitted_event_id(expr: &Expr<'_>) -> Option<EventId> {
    let ExprKind::Call(callee, ..) = &expr.peel_parens().kind else { return None };
    match referenced_item(callee)? {
        ItemId::Event(event_id) => Some(event_id),
        _ => None,
    }
}

/// Whether the event name or one of its parameter names mentions the state variable: its
/// normalized name, its singular form, or a role keyword it contains.
fn event_mentions_state_var(hir: &hir::Hir<'_>, event_id: EventId, var_id: VariableId) -> bool {
    let Some(var_name) = hir.variable(var_id).name else { return false };
    let var_name = normalize(var_name.as_str());
    let mut keywords = vec![var_name.as_str()];
    keywords.extend(var_name.strip_suffix('s').filter(|singular| !singular.is_empty()));
    let roles = ["owner", "admin", "guardian", "manager", "role"];
    keywords.extend(roles.into_iter().filter(|role| var_name.contains(role)));

    let event = hir.event(event_id);
    let param_names = event.parameters.iter().filter_map(|&p| hir.variable(p).name);
    iter::once(event.name).chain(param_names).any(|name| {
        let name = normalize(name.as_str());
        keywords.iter().any(|keyword| !keyword.is_empty() && name.contains(keyword))
    })
}

fn normalize(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}
