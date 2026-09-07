use super::FunctionSelectorCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            arg_for_param, branch_always_exits, expr_is_address, is_address_cast, is_builtin,
            is_require_or_assert, ty_contract_id,
        },
    },
};
use alloy_primitives::Selector;
use solar::{
    ast::{LitKind, UnOpKind},
    interface::{Symbol, data_structures::Never, kw, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, BinOpKind, ContractId, ContractKind, Expr, ExprKind, LoopSource, Res, Stmt,
            StmtKind, VariableId, Visit,
        },
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

/// Path-state cap above which selector constraints are widened to "any selector".
const MAX_LOOP_PATH_STATES: usize = 128;

declare_forge_lint!(
    FUNCTION_SELECTOR_COLLISION,
    Severity::High,
    "function-selector-collision",
    "proxy and implementation functions have colliding selectors"
);

impl<'gcx> LateLintPass<'gcx> for FunctionSelectorCollision {
    fn check_nested_contract(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, proxy_id: ContractId) {
        let proxy = gcx.hir.contract(proxy_id);
        if proxy.kind != ContractKind::Contract || proxy.linearization_failed() {
            return;
        }
        let Some(fallback_id) = proxy.fallback else { return };
        let fallback = gcx.hir.function(fallback_id);
        let Some(body) = fallback.body else { return };

        let mut collector = DelegateTargetCollector {
            gcx,
            current_inputs: Vec::new(),
            paths: vec![PathState::default()],
            placeholder: None,
            return_controls: vec![Vec::new()],
            continuation_cache: HashMap::new(),
            loop_controls: Vec::new(),
            targets: Vec::new(),
        };
        collector.visit_modifier_chain(Continuation {
            modifiers: fallback.modifiers,
            index: 0,
            body,
            body_input: fallback
                .parameters
                .first()
                .map(|&var| CalldataInput { var, modifier: None }),
        });

        let proxy_functions = gcx.interface_functions(proxy_id);
        for target in collector.targets {
            let implementation = gcx.hir.contract(target.contract);
            if target.contract == proxy_id
                || implementation.kind == ContractKind::Library
                || implementation.linearization_failed()
            {
                continue;
            }

            for proxy_function in proxy_functions.all() {
                for implementation_function in gcx.interface_functions(target.contract).all() {
                    let selector = proxy_function.selector;
                    if selector != implementation_function.selector
                        || !target.filters.iter().any(|filter| filter.allows(selector))
                    {
                        continue;
                    }
                    let proxy_signature = gcx.item_signature(proxy_function.id.into());
                    let implementation_signature =
                        gcx.item_signature(implementation_function.id.into());
                    if proxy_signature == implementation_signature {
                        continue;
                    }
                    let msg = format!(
                        "proxy function `{}.{proxy_signature}` collides with implementation function `{}.{implementation_signature}` at selector `{selector}`",
                        proxy.name.as_str(),
                        implementation.name.as_str(),
                    );
                    ctx.emit_with_msg(&FUNCTION_SELECTOR_COLLISION, proxy.name.span, msg);
                }
            }
        }
    }
}

/// The `msg.sig` constraints known to hold on a path.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct SelectorFilter {
    required: Option<Selector>,
    excluded: Vec<Selector>,
}

impl SelectorFilter {
    fn allows(&self, selector: Selector) -> bool {
        self.required.is_none_or(|required| required == selector)
            && !self.excluded.contains(&selector)
    }

    /// Narrows the filter by `msg.sig == selector` being `matches`; `None` if contradictory.
    fn with_guard(mut self, selector: Selector, matches: bool) -> Option<Self> {
        if matches {
            if self.excluded.contains(&selector)
                || self.required.is_some_and(|required| required != selector)
            {
                return None;
            }
            self.required = Some(selector);
        } else {
            if self.required == Some(selector) {
                return None;
            }
            if self.required.is_none() && !self.excluded.contains(&selector) {
                self.excluded.push(selector);
                self.excluded.sort_unstable();
            }
        }
        Some(self)
    }
}

struct DelegateTarget {
    contract: ContractId,
    filters: Vec<SelectorFilter>,
}

/// A parameter that initially holds the full `msg.data`: the fallback's own, or one of the
/// `modifier`-th applied modifier (the same modifier may be applied several times).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct CalldataInput {
    var: VariableId,
    modifier: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct PathState {
    selector_filter: SelectorFilter,
    modified_inputs: Vec<CalldataInput>,
}

impl PathState {
    fn input_unmodified(&self, input: CalldataInput) -> bool {
        !self.modified_inputs.contains(&input)
    }

    fn mark_input_modified(&mut self, input: CalldataInput) {
        if !self.modified_inputs.contains(&input) {
            self.modified_inputs.push(input);
            self.modified_inputs.sort_unstable();
        }
    }

    fn clear_inputs(&mut self, inputs: &[CalldataInput]) {
        self.modified_inputs.retain(|input| !inputs.contains(input));
    }
}

#[derive(Default)]
struct LoopControl {
    breaks: Vec<PathState>,
    continues: Vec<PathState>,
}

/// What `_` resumes: the rest of the modifier chain and the function body.
#[derive(Clone, Copy)]
struct Continuation<'gcx> {
    modifiers: &'gcx [hir::Modifier<'gcx>],
    index: usize,
    body: hir::Block<'gcx>,
    body_input: Option<CalldataInput>,
}

fn lvalue_contains_var(expr: &Expr<'_>, target: VariableId) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().any(|res| res.as_variable() == Some(target)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().any(|expr| lvalue_contains_var(expr, target))
        }
        _ => false,
    }
}

fn extend_unique<T: PartialEq>(items: &mut Vec<T>, new_items: impl IntoIterator<Item = T>) {
    for item in new_items {
        if !items.contains(&item) {
            items.push(item);
        }
    }
}

fn dedup(paths: &mut Vec<PathState>) {
    let mut seen = HashSet::with_capacity(paths.len());
    paths.retain(|path| seen.insert(path.clone()));
}

struct DelegateTargetCollector<'gcx> {
    gcx: Gcx<'gcx>,
    /// Full-calldata inputs visible in the block being visited.
    current_inputs: Vec<CalldataInput>,
    /// Live path states; empty means the current point is unreachable.
    paths: Vec<PathState>,
    placeholder: Option<Continuation<'gcx>>,
    /// Per function/modifier frame, the states at each `return`.
    return_controls: Vec<Vec<PathState>>,
    continuation_cache: HashMap<(usize, PathState), Vec<PathState>>,
    loop_controls: Vec<LoopControl>,
    targets: Vec<DelegateTarget>,
}

impl<'gcx> DelegateTargetCollector<'gcx> {
    fn visit_modifier_chain(&mut self, cont: Continuation<'gcx>) {
        let previous_inputs =
            std::mem::replace(&mut self.current_inputs, cont.body_input.into_iter().collect());
        if let Some(invocation) = cont.modifiers.get(cont.index) {
            for arg in invocation.args.exprs() {
                let _ = self.visit_expr(arg);
            }
            if let Some(modifier_id) = invocation.id.as_function()
                && let Some(modifier_body) = self.gcx.hir.function(modifier_id).body
            {
                let modifier = self.gcx.hir.function(modifier_id);
                let params: Vec<_> = modifier
                    .parameters
                    .iter()
                    .map(|&var| CalldataInput { var, modifier: Some(cont.index) })
                    .collect();
                // Parameters bound to a full-calldata argument inherit its provenance.
                let bindings: Vec<_> = params
                    .iter()
                    .filter_map(|&param| {
                        let arg =
                            arg_for_param(&self.gcx.hir, modifier, param.var, &invocation.args)?;
                        Some((param, full_calldata_source(arg, &self.current_inputs)?))
                    })
                    .collect();
                for path in &mut self.paths {
                    path.clear_inputs(&params);
                    for &(param, source) in &bindings {
                        if source.is_some_and(|source| !path.input_unmodified(source)) {
                            path.mark_input_modified(param);
                        }
                    }
                }
                let inputs = bindings.iter().map(|&(input, _)| input).collect();
                let next = Continuation { index: cont.index + 1, ..cont };
                self.visit_block(modifier_body, Some(next), inputs);
                for path in &mut self.paths {
                    path.clear_inputs(&params);
                }
                if let Some(returns) = self.return_controls.last_mut() {
                    for path in returns {
                        path.clear_inputs(&params);
                    }
                }
            } else {
                self.visit_modifier_chain(Continuation { index: cont.index + 1, ..cont });
            }
        } else {
            self.visit_block(cont.body, None, self.current_inputs.clone());
        }
        self.current_inputs = previous_inputs;
    }

    fn visit_block(
        &mut self,
        block: hir::Block<'gcx>,
        placeholder: Option<Continuation<'gcx>>,
        inputs: Vec<CalldataInput>,
    ) {
        let previous = std::mem::replace(&mut self.placeholder, placeholder);
        let previous_inputs = std::mem::replace(&mut self.current_inputs, inputs);
        for stmt in block.stmts {
            let _ = self.visit_stmt(stmt);
        }
        self.placeholder = previous;
        self.current_inputs = previous_inputs;
    }

    /// Runs the continuation once per distinct incoming path state, memoizing the outcome.
    fn visit_continuation(&mut self, cont: Continuation<'gcx>) {
        let mut output_paths = Vec::new();
        for input in std::mem::take(&mut self.paths) {
            let key = (cont.index, input);
            if let Some(cached) = self.continuation_cache.get(&key) {
                extend_unique(&mut output_paths, cached.iter().cloned());
                continue;
            }
            self.paths.push(key.1.clone());
            self.return_controls.push(Vec::new());
            self.visit_modifier_chain(cont);
            let mut result = std::mem::take(&mut self.paths);
            let returns = self.return_controls.pop().expect("return control stack is not empty");
            extend_unique(&mut result, returns);
            self.continuation_cache.insert(key, result.clone());
            extend_unique(&mut output_paths, result);
        }
        self.paths = output_paths;
    }

    /// Records `contract` as a delegatecall target reachable under the current paths' selector
    /// filters (only paths on which `required_input` still holds the full calldata count).
    fn record_target(&mut self, contract: ContractId, required_input: Option<CalldataInput>) {
        let mut filters = Vec::new();
        extend_unique(
            &mut filters,
            self.paths
                .iter()
                .filter(|path| required_input.is_none_or(|input| path.input_unmodified(input)))
                .map(|path| path.selector_filter.clone()),
        );
        if filters.is_empty() {
            return;
        }
        let target = match self.targets.iter().position(|target| target.contract == contract) {
            Some(index) => &mut self.targets[index],
            None => {
                self.targets.push(DelegateTarget { contract, filters: Vec::new() });
                self.targets.last_mut().expect("target was just pushed")
            }
        };
        if target.filters.contains(&SelectorFilter::default()) {
            return;
        }
        extend_unique(&mut target.filters, filters);
        if target.filters.len() > MAX_LOOP_PATH_STATES {
            target.filters = vec![SelectorFilter::default()];
        }
    }

    /// Splits the live paths into those where `expr` is true and those where it is false.
    fn visit_condition(&mut self, expr: &'gcx Expr<'gcx>) -> (Vec<PathState>, Vec<PathState>) {
        match &expr.peel_parens().kind {
            ExprKind::Lit(lit) => {
                let paths = std::mem::take(&mut self.paths);
                match lit.kind {
                    LitKind::Bool(true) => (paths, Vec::new()),
                    LitKind::Bool(false) => (Vec::new(), paths),
                    _ => (paths.clone(), paths),
                }
            }
            ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
                let (true_paths, false_paths) = self.visit_condition(inner);
                (false_paths, true_paths)
            }
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let (lhs_true, lhs_false) = self.visit_condition(lhs);
                if op.kind == BinOpKind::And {
                    self.paths = lhs_true;
                    let (rhs_true, mut rhs_false) = self.visit_condition(rhs);
                    extend_unique(&mut rhs_false, lhs_false);
                    (rhs_true, rhs_false)
                } else {
                    self.paths = lhs_false;
                    let (mut rhs_true, rhs_false) = self.visit_condition(rhs);
                    extend_unique(&mut rhs_true, lhs_true);
                    (rhs_true, rhs_false)
                }
            }
            ExprKind::Ternary(condition, true_expr, false_expr) => {
                let (condition_true, condition_false) = self.visit_condition(condition);
                self.paths = condition_true;
                let (mut true_paths, mut false_paths) = self.visit_condition(true_expr);
                self.paths = condition_false;
                let (false_arm_true, false_arm_false) = self.visit_condition(false_expr);
                extend_unique(&mut true_paths, false_arm_true);
                extend_unique(&mut false_paths, false_arm_false);
                (true_paths, false_paths)
            }
            _ => {
                let _ = self.visit_expr(expr);
                let paths = std::mem::take(&mut self.paths);
                let guard = selector_guard(self.gcx, expr);
                let branch = |condition_is_true: bool| {
                    paths
                        .iter()
                        .filter_map(|path| {
                            let mut path = path.clone();
                            if let Some((selector, matches)) = guard {
                                path.selector_filter = path
                                    .selector_filter
                                    .with_guard(selector, matches == condition_is_true)?;
                            }
                            Some(path)
                        })
                        .collect()
                };
                (branch(true), branch(false))
            }
        }
    }

    /// Visits loop-body statements: `break` paths are added to `exits`, and the paths reaching
    /// the next iteration (fall-through and `continue`) are returned.
    fn visit_loop_stmts(
        &mut self,
        stmts: &'gcx [Stmt<'gcx>],
        exits: &mut Vec<PathState>,
    ) -> Vec<PathState> {
        self.loop_controls.push(LoopControl::default());
        for stmt in stmts {
            let _ = self.visit_stmt(stmt);
        }
        let mut next = std::mem::take(&mut self.paths);
        let control = self.loop_controls.pop().expect("loop control stack is not empty");
        extend_unique(exits, control.breaks);
        extend_unique(&mut next, control.continues);
        next
    }

    /// Collapses `paths` to one unconstrained state keeping only the inputs modified on all of
    /// them.
    fn widen_loop_paths(paths: &mut Vec<PathState>) {
        let Some(first) = paths.first() else { return };
        let mut modified_inputs = first.modified_inputs.clone();
        modified_inputs
            .retain(|input| paths.iter().skip(1).all(|path| path.modified_inputs.contains(input)));
        *paths = vec![PathState { selector_filter: SelectorFilter::default(), modified_inputs }];
    }

    /// One iteration of a `for` loop with an update statement: `if (cond) { body } else break`
    /// followed by the update, which `continue` also reaches. Returns the back-edge paths and the
    /// loop-exit paths.
    fn visit_for_iteration(
        &mut self,
        block: &hir::Block<'gcx>,
        update: &'gcx Stmt<'gcx>,
    ) -> Option<(Vec<PathState>, Vec<PathState>)> {
        let [stmt] = block.stmts else { return None };
        let (condition, body, else_stmt) = match &stmt.kind {
            StmtKind::If(condition, then_stmt, else_stmt) => {
                (Some(*condition), *then_stmt, *else_stmt)
            }
            _ => (None, stmt, None),
        };

        let mut exits = Vec::new();
        if let Some(condition) = condition {
            let (true_paths, false_paths) = self.visit_condition(condition);
            self.paths = false_paths;
            if let Some(else_stmt) = else_stmt {
                let fallthrough =
                    self.visit_loop_stmts(std::slice::from_ref(else_stmt), &mut exits);
                extend_unique(&mut exits, fallthrough);
            } else {
                extend_unique(&mut exits, std::mem::take(&mut self.paths));
            }
            self.paths = true_paths;
        }

        self.paths = self.visit_loop_stmts(std::slice::from_ref(body), &mut exits);
        let _ = self.visit_stmt(update);
        Some((std::mem::take(&mut self.paths), exits))
    }

    /// Iterates the loop body to a fixpoint over the set of distinct entry states.
    fn visit_loop(&mut self, block: &hir::Block<'gcx>, source: LoopSource<'gcx>) {
        let mut pending = std::mem::take(&mut self.paths);
        let mut seen = HashSet::new();
        let mut exits = Vec::new();

        loop {
            pending.retain(|path| seen.insert(path.clone()));
            if pending.is_empty() {
                break;
            }

            self.paths = std::mem::take(&mut pending);
            let next = if let LoopSource::For { update: Some(update) } = source
                && let Some((next, for_exits)) = self.visit_for_iteration(block, update)
            {
                extend_unique(&mut exits, for_exits);
                next
            } else if matches!(source, LoopSource::DoWhile)
                && let Some((condition, body)) = block.stmts.split_last()
            {
                // `continue` in a do-while body still evaluates the condition.
                self.paths = self.visit_loop_stmts(body, &mut exits);
                self.visit_loop_stmts(std::slice::from_ref(condition), &mut exits)
            } else {
                self.visit_loop_stmts(block.stmts, &mut exits)
            };
            extend_unique(&mut pending, next);
            if seen.len() + pending.len() > MAX_LOOP_PATH_STATES {
                Self::widen_loop_paths(&mut pending);
            }
            if exits.len() > MAX_LOOP_PATH_STATES {
                Self::widen_loop_paths(&mut exits);
            }
        }

        self.paths = exits;
    }

    /// Visits two alternative branches from the given entry paths and joins their outcomes.
    fn visit_branches(
        &mut self,
        true_paths: Vec<PathState>,
        true_branch: impl FnOnce(&mut Self),
        false_paths: Vec<PathState>,
        false_branch: impl FnOnce(&mut Self),
    ) {
        self.paths = true_paths;
        true_branch(self);
        let mut joined = std::mem::take(&mut self.paths);
        self.paths = false_paths;
        false_branch(self);
        joined.append(&mut self.paths);
        self.paths = joined;
        dedup(&mut self.paths);
    }
}

impl<'gcx> Visit<'gcx> for DelegateTargetCollector<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Never> {
        if self.paths.is_empty() {
            return ControlFlow::Continue(());
        }
        match &expr.kind {
            ExprKind::Ternary(condition, true_expr, false_expr) => {
                let (true_paths, false_paths) = self.visit_condition(condition);
                self.visit_branches(
                    true_paths,
                    |this| _ = this.visit_expr(true_expr),
                    false_paths,
                    |this| _ = this.visit_expr(false_expr),
                );
            }
            ExprKind::Binary(_, op, _) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let (mut true_paths, false_paths) = self.visit_condition(expr);
                extend_unique(&mut true_paths, false_paths);
                self.paths = true_paths;
                dedup(&mut self.paths);
            }
            ExprKind::Call(callee, args, opts) => {
                let _ = self.visit_expr(callee);
                for arg in opts.iter().flat_map(|opts| opts.args) {
                    let _ = self.visit_expr(&arg.value);
                }
                let mut args = args.exprs();
                if is_require_or_assert(callee) {
                    let Some(condition) = args.next() else { return ControlFlow::Continue(()) };
                    let args: Vec<_> = args.collect();
                    let (true_paths, false_paths) = self.visit_condition(condition);
                    // Remaining arguments are evaluated before `require`/`assert` decides
                    // whether to revert, so their targets and side effects apply on both paths;
                    // only the passing paths continue.
                    self.paths = true_paths;
                    for &arg in &args {
                        let _ = self.visit_expr(arg);
                    }
                    let continuing_paths = std::mem::take(&mut self.paths);
                    self.paths = false_paths;
                    for arg in args {
                        let _ = self.visit_expr(arg);
                    }
                    self.paths = continuing_paths;
                } else {
                    for arg in args {
                        let _ = self.visit_expr(arg);
                    }
                    if let Some((target, required_input)) =
                        delegated_contract(self.gcx, &self.current_inputs, expr)
                    {
                        self.record_target(target, required_input);
                    }
                }
            }
            _ => {
                let mutated_inputs: Vec<_> = match &expr.peel_parens().kind {
                    ExprKind::Assign(lhs, _, _) => self
                        .current_inputs
                        .iter()
                        .copied()
                        .filter(|input| lvalue_contains_var(lhs, input.var))
                        .collect(),
                    _ => Vec::new(),
                };
                let _ = self.walk_expr(expr);
                for path in &mut self.paths {
                    for &input in &mutated_inputs {
                        path.mark_input_modified(input);
                    }
                }
            }
        }
        ControlFlow::Continue(())
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Never> {
        if self.paths.is_empty() {
            return ControlFlow::Continue(());
        }
        match &stmt.kind {
            StmtKind::If(condition, then_stmt, else_stmt) => {
                let (true_paths, false_paths) = self.visit_condition(condition);
                self.visit_branches(
                    true_paths,
                    |this| _ = this.visit_stmt(then_stmt),
                    false_paths,
                    |this| {
                        if let Some(else_stmt) = else_stmt {
                            let _ = this.visit_stmt(else_stmt);
                        }
                    },
                );
            }
            StmtKind::Try(try_) => {
                let _ = self.visit_expr(&try_.expr);
                let paths = std::mem::take(&mut self.paths);
                let mut joined = Vec::new();
                for clause in try_.clauses {
                    self.paths = paths.clone();
                    for &var in clause.args {
                        let _ = self.visit_nested_var(var);
                    }
                    for stmt in clause.block.stmts {
                        let _ = self.visit_stmt(stmt);
                    }
                    joined.append(&mut self.paths);
                }
                self.paths = joined;
                dedup(&mut self.paths);
            }
            StmtKind::Loop(block, source) => self.visit_loop(block, *source),
            StmtKind::Break | StmtKind::Continue => {
                let paths = std::mem::take(&mut self.paths);
                if let Some(control) = self.loop_controls.last_mut() {
                    let destination = if matches!(stmt.kind, StmtKind::Break) {
                        &mut control.breaks
                    } else {
                        &mut control.continues
                    };
                    extend_unique(destination, paths);
                }
            }
            StmtKind::Placeholder => {
                if let Some(cont) = self.placeholder {
                    self.visit_continuation(cont);
                }
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    let _ = self.visit_expr(expr);
                }
                let paths = std::mem::take(&mut self.paths);
                let returns =
                    self.return_controls.last_mut().expect("return control stack is not empty");
                extend_unique(returns, paths);
            }
            StmtKind::AssemblyBlock(_) => {
                // Inline assembly may rewrite any calldata parameter in scope.
                for path in &mut self.paths {
                    for &input in &self.current_inputs {
                        path.mark_input_modified(input);
                    }
                }
            }
            _ => {
                let _ = self.walk_stmt(stmt);
                if branch_always_exits(stmt) {
                    self.paths.clear();
                }
            }
        }
        ControlFlow::Continue(())
    }
}

/// `msg.sig == F.selector` / `msg.sig != F.selector` as `(selector, matches)`.
fn selector_guard(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<(Selector, bool)> {
    let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind else { return None };
    let matches = match op.kind {
        BinOpKind::Eq => true,
        BinOpKind::Ne => false,
        _ => return None,
    };
    let selector = if is_msg_member(lhs, sym::sig) {
        rhs
    } else if is_msg_member(rhs, sym::sig) {
        lhs
    } else {
        return None;
    };
    let selector = selector.peel_parens();
    let ExprKind::Member(function, member) = &selector.kind else { return None };
    if member.name != sym::selector
        || gcx.resolved_builtin(selector) != Some(Builtin::FunctionSelector)
    {
        return None;
    }
    let Res::Item(hir::ItemId::Function(function)) = gcx.resolved_expr(function)? else {
        return None;
    };
    Some((gcx.function_selector(function), matches))
}

/// `msg.<name>`.
fn is_msg_member(expr: &Expr<'_>, name: Symbol) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Member(base, member)
        if member.name == name && is_builtin(base, sym::msg))
}

/// The statically typed implementation contract of a proxy-style `<addr>.delegatecall(<full
/// calldata>)`, with the calldata input that must be unmodified for the forwarding to be complete.
fn delegated_contract<'gcx>(
    gcx: Gcx<'gcx>,
    full_calldata_inputs: &[CalldataInput],
    expr: &'gcx Expr<'gcx>,
) -> Option<(ContractId, Option<CalldataInput>)> {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return None };
    let ExprKind::Member(receiver, member) = &callee.peel_parens().kind else { return None };
    let required_input = full_calldata_source(args.exprs().next()?, full_calldata_inputs)?;
    if member.name != kw::Delegatecall
        || gcx.resolved_builtin(callee) != Some(Builtin::AddressDelegatecall)
        || !expr_is_address(gcx, receiver)
    {
        return None;
    }
    typed_contract_behind_address_cast(gcx, receiver).map(|contract| (contract, required_input))
}

fn typed_contract_behind_address_cast<'gcx>(
    gcx: Gcx<'gcx>,
    expr: &'gcx Expr<'gcx>,
) -> Option<ContractId> {
    let expr = expr.peel_parens();
    if let Some(id) = gcx.type_of_expr(expr.id).and_then(ty_contract_id) {
        return Some(id);
    }
    match &expr.kind {
        ExprKind::Call(callee, args, _) if is_address_cast(callee) => {
            args.exprs().next().and_then(|arg| typed_contract_behind_address_cast(gcx, arg))
        }
        ExprKind::Payable(inner) => typed_contract_behind_address_cast(gcx, inner),
        _ => None,
    }
}

/// `Some(None)` for `msg.data`, `Some(Some(input))` for a known full-calldata input, `None`
/// otherwise.
fn full_calldata_source(
    expr: &Expr<'_>,
    full_calldata_inputs: &[CalldataInput],
) -> Option<Option<CalldataInput>> {
    if is_msg_member(expr, sym::data) {
        return Some(None);
    }
    let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return None };
    reses
        .iter()
        .filter_map(Res::as_variable)
        .find_map(|id| full_calldata_inputs.iter().copied().find(|input| input.var == id).map(Some))
}
