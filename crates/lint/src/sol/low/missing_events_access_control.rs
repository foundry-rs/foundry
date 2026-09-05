use super::MissingEventsAccessControl;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            branch_always_exits, for_each_lhs_var, function_ids, guard_vars, is_protected,
            is_sender_member, is_zero_value, lhs_local_var, referenced_item, underlying_var,
        },
    },
};
use solar::{
    ast::{ContractKind, DataLocation, StateMutability, Visibility},
    interface::{Span, data_structures::Never},
    sema::{
        Gcx,
        hir::{
            self, EventId, Expr, ExprKind, FunctionId, ItemId, Stmt, StmtKind, VariableId, Visit,
        },
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

impl<'gcx> LateLintPass<'gcx> for MissingEventsAccessControl {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract: &'gcx hir::Contract<'gcx>,
    ) {
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract) {
            return;
        }

        // Every state variable some access check in the contract depends on.
        let functions: Vec<_> = contract.all_functions().collect();
        let targets: HashSet<_> =
            functions.iter().flat_map(|&id| guard_vars(&gcx.hir, id)).collect();
        if targets.is_empty() {
            return;
        }

        for func_id in functions {
            let func = gcx.hir.function(func_id);
            let is_entry_point = func.kind.is_function()
                && matches!(func.visibility, Visibility::Public | Visibility::External)
                && !func.is_constructor()
                && !func.is_special()
                && !matches!(func.state_mutability, StateMutability::Pure | StateMutability::View);
            if !is_entry_point || !is_protected(&gcx.hir, func_id) {
                continue;
            }

            let guard_targets = guard_vars(&gcx.hir, func_id);
            let mut analyzer = WriteAnalyzer {
                gcx,
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
                let name = gcx
                    .hir
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

/// Calls `f` on every index and slice bound along the spine of an lvalue.
fn for_each_lhs_index<'gcx>(expr: &'gcx Expr<'gcx>, f: &mut impl FnMut(&'gcx Expr<'gcx>)) {
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
struct WriteAnalyzer<'a, 'gcx> {
    gcx: Gcx<'gcx>,
    targets: &'a HashSet<VariableId>,
    /// Targets checked by this entry point's own guards; clearing one of them is reportable even
    /// when the written value carries no source.
    guard_targets: &'a HashSet<VariableId>,
    state: State,
    call_stack: Vec<FunctionId>,
}

impl<'gcx> WriteAnalyzer<'_, 'gcx> {
    fn analyze_function(&mut self, func_id: FunctionId) {
        if self.call_stack.contains(&func_id) {
            return;
        }
        let func = self.gcx.hir.function(func_id);
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
    fn analyze_call(&mut self, callee_id: FunctionId, args: &hir::CallArgs<'gcx>) {
        let params = self
            .gcx
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
                if self.gcx.hir.variable(var_id).kind.is_state() {
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
            let root = if self.gcx.hir.variable(var_id).kind.is_state() {
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
        let root = (self.gcx.hir.variable(var_id).data_location == Some(DataLocation::Storage))
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
        for write in &mut self.state.writes {
            if !write.evented
                && (write.fixed_clear || !write.sources.is_disjoint(&event_sources))
                && event_mentions_state_var(self.gcx, event_id, write.var_id)
            {
                write.evented = true;
            }
        }
    }
}

impl<'gcx> Visit<'gcx> for WriteAnalyzer<'_, 'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Never> {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.gcx.hir.variable(var_id).initializer {
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

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Never> {
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
                if let Some(local) = lhs_local_var(&self.gcx.hir, lhs) {
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

fn emitted_event_id(expr: &Expr<'_>) -> Option<EventId> {
    let ExprKind::Call(callee, ..) = &expr.peel_parens().kind else { return None };
    match referenced_item(callee)? {
        ItemId::Event(event_id) => Some(event_id),
        _ => None,
    }
}

/// Whether the event name or one of its parameter names mentions the state variable: its
/// normalized name, its singular form, or a role keyword it contains.
fn event_mentions_state_var(gcx: Gcx<'_>, event_id: EventId, var_id: VariableId) -> bool {
    let Some(var_name) = gcx.hir.variable(var_id).name else { return false };
    let var_name = normalize(var_name.as_str());
    let mut keywords = vec![var_name.as_str()];
    keywords.extend(var_name.strip_suffix('s').filter(|singular| !singular.is_empty()));
    let roles = ["owner", "admin", "guardian", "manager", "role"];
    keywords.extend(roles.into_iter().filter(|role| var_name.contains(role)));

    let event = gcx.hir.event(event_id);
    let param_names = event.parameters.iter().filter_map(|&p| gcx.hir.variable(p).name);
    iter::once(event.name).chain(param_names).any(|name| {
        let name = normalize(name.as_str());
        keywords.iter().any(|keyword| !keyword.is_empty() && name.contains(keyword))
    })
}

fn normalize(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}
