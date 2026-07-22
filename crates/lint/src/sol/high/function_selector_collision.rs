use super::FunctionSelectorCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::primitives::{branch_always_exits, is_require_or_assert},
    },
};
use alloy_primitives::Selector;
use solar::{
    ast::{LitKind, UnOpKind},
    interface::{data_structures::Never, kw, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, BinOpKind, CallArgs, ContractId, ContractKind, Expr, ExprKind, ItemId, Stmt,
            StmtKind, TypeKind, Visit,
        },
        ty::{ResolvedMember, Ty, TyKind},
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

const MAX_LOOP_PATH_STATES: usize = 128;

declare_forge_lint!(
    FUNCTION_SELECTOR_COLLISION,
    Severity::High,
    "function-selector-collision",
    "proxy and implementation functions have colliding selectors"
);

impl<'hir> LateLintPass<'hir> for FunctionSelectorCollision {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        proxy_id: ContractId,
    ) {
        let proxy = hir.contract(proxy_id);
        if proxy.kind != ContractKind::Contract || proxy.linearization_failed() {
            return;
        }
        let Some(fallback_id) = proxy.fallback else { return };
        let fallback = hir.function(fallback_id);
        let Some(body) = fallback.body else { return };

        let mut collector = DelegateTargetCollector {
            gcx,
            hir,
            current_inputs: Vec::new(),
            paths: vec![PathState::initial()],
            placeholder: None,
            return_controls: vec![Vec::new()],
            continuation_cache: HashMap::new(),
            loop_controls: Vec::new(),
            targets: Vec::new(),
        };
        collector.visit_modifier_chain(
            fallback.modifiers,
            0,
            body,
            fallback.parameters.first().copied().map(CalldataInput::Fallback),
        );

        let proxy_functions = gcx.interface_functions(proxy_id);
        for target in collector.targets {
            let implementation_id = target.contract;
            if implementation_id == proxy_id {
                continue;
            }
            let implementation = hir.contract(implementation_id);
            if implementation.kind == ContractKind::Library || implementation.linearization_failed()
            {
                continue;
            }

            for proxy_function in proxy_functions.all() {
                for implementation_function in gcx.interface_functions(implementation_id).all() {
                    if proxy_function.selector != implementation_function.selector
                        || !target.allows(implementation_function.selector)
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
                        "proxy function `{}.{proxy_signature}` collides with implementation function `{}.{implementation_signature}` at selector `{}`",
                        proxy.name.as_str(),
                        implementation.name.as_str(),
                        proxy_function.selector,
                    );
                    ctx.emit_with_msg(&FUNCTION_SELECTOR_COLLISION, proxy.name.span, msg);
                }
            }
        }
    }
}

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

impl DelegateTarget {
    fn allows(&self, selector: Selector) -> bool {
        self.filters.iter().any(|filter| filter.allows(selector))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum CalldataInput {
    Fallback(hir::VariableId),
    Modifier { index: usize, param: hir::VariableId },
}

impl CalldataInput {
    const fn variable(self) -> hir::VariableId {
        match self {
            Self::Fallback(variable) => variable,
            Self::Modifier { param, .. } => param,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PathState {
    selector_filter: SelectorFilter,
    modified_inputs: Vec<CalldataInput>,
}

impl PathState {
    fn initial() -> Self {
        Self { selector_filter: SelectorFilter::default(), modified_inputs: Vec::new() }
    }

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

fn lvalue_contains_var(expr: &Expr<'_>, target: hir::VariableId) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses
            .iter()
            .any(|res| matches!(res, hir::Res::Item(ItemId::Variable(id)) if *id == target)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().any(|expr| lvalue_contains_var(expr, target))
        }
        _ => false,
    }
}

struct DelegateTargetCollector<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    current_inputs: Vec<CalldataInput>,
    paths: Vec<PathState>,
    placeholder:
        Option<(&'hir [hir::Modifier<'hir>], usize, hir::Block<'hir>, Option<CalldataInput>)>,
    return_controls: Vec<Vec<PathState>>,
    continuation_cache: HashMap<(usize, PathState), Vec<PathState>>,
    loop_controls: Vec<LoopControl>,
    targets: Vec<DelegateTarget>,
}

impl<'hir> DelegateTargetCollector<'hir> {
    fn visit_modifier_chain(
        &mut self,
        modifiers: &'hir [hir::Modifier<'hir>],
        index: usize,
        body: hir::Block<'hir>,
        body_input: Option<CalldataInput>,
    ) {
        let previous_inputs =
            std::mem::replace(&mut self.current_inputs, body_input.into_iter().collect());
        let Some(invocation) = modifiers.get(index) else {
            self.visit_block(body, None, self.current_inputs.clone());
            self.current_inputs = previous_inputs;
            return;
        };

        for arg in invocation.args.exprs() {
            let _ = self.visit_expr(arg);
        }

        if let Some(modifier_id) = invocation.id.as_function() {
            let modifier = self.hir.function(modifier_id);
            if let Some(modifier_body) = modifier.body {
                let bindings = modifier_input_bindings(
                    self.hir,
                    modifier,
                    &invocation.args,
                    &self.current_inputs,
                    index,
                );
                let params = modifier
                    .parameters
                    .iter()
                    .copied()
                    .map(|param| CalldataInput::Modifier { index, param })
                    .collect::<Vec<_>>();
                for path in &mut self.paths {
                    path.clear_inputs(&params);
                    for &(param, source) in &bindings {
                        if source.is_some_and(|source| !path.input_unmodified(source)) {
                            path.mark_input_modified(param);
                        }
                    }
                }

                let modifier_inputs = bindings.iter().map(|&(input, _)| input).collect();
                self.visit_block(
                    modifier_body,
                    Some((modifiers, index + 1, body, body_input)),
                    modifier_inputs,
                );
                self.clear_local_inputs(&params);
                self.current_inputs = previous_inputs;
                return;
            }
        }

        self.visit_modifier_chain(modifiers, index + 1, body, body_input);
        self.current_inputs = previous_inputs;
    }

    fn visit_block(
        &mut self,
        block: hir::Block<'hir>,
        placeholder: Option<(
            &'hir [hir::Modifier<'hir>],
            usize,
            hir::Block<'hir>,
            Option<CalldataInput>,
        )>,
        inputs: Vec<CalldataInput>,
    ) {
        let previous = self.placeholder;
        let previous_inputs = std::mem::replace(&mut self.current_inputs, inputs);
        self.placeholder = placeholder;
        for stmt in block.stmts {
            let _ = self.visit_stmt(stmt);
        }
        self.placeholder = previous;
        self.current_inputs = previous_inputs;
    }

    fn visit_continuation(
        &mut self,
        modifiers: &'hir [hir::Modifier<'hir>],
        index: usize,
        body: hir::Block<'hir>,
        body_input: Option<CalldataInput>,
    ) {
        let input_paths = std::mem::take(&mut self.paths);
        let mut output_paths = Vec::new();
        for input in input_paths {
            let key = (index, input.clone());
            if let Some(cached) = self.continuation_cache.get(&key) {
                Self::extend_unique(&mut output_paths, cached.iter().cloned());
                continue;
            }

            self.paths.push(input);
            self.return_controls.push(Vec::new());
            self.visit_modifier_chain(modifiers, index, body, body_input);
            let mut result = std::mem::take(&mut self.paths);
            let returns = self.return_controls.pop().expect("return control stack is not empty");
            Self::extend_unique(&mut result, returns);
            self.continuation_cache.insert(key, result.clone());
            Self::extend_unique(&mut output_paths, result);
        }
        self.paths = output_paths;
    }

    fn clear_local_inputs(&mut self, inputs: &[CalldataInput]) {
        for path in &mut self.paths {
            path.clear_inputs(inputs);
        }
        if let Some(returns) = self.return_controls.last_mut() {
            for path in returns {
                path.clear_inputs(inputs);
            }
        }
    }

    fn record_target(&mut self, contract: ContractId, required_input: Option<CalldataInput>) {
        let mut filters = Vec::new();
        for path in &self.paths {
            if required_input.is_none_or(|input| path.input_unmodified(input))
                && !filters.contains(&path.selector_filter)
            {
                filters.push(path.selector_filter.clone());
            }
        }
        if filters.is_empty() {
            return;
        }
        if filters.len() > MAX_LOOP_PATH_STATES {
            filters.clear();
            filters.push(SelectorFilter::default());
        }

        if let Some(target) = self.targets.iter_mut().find(|target| target.contract == contract) {
            if target.filters.contains(&SelectorFilter::default()) {
                return;
            }
            for filter in filters {
                if !target.filters.contains(&filter) {
                    target.filters.push(filter);
                }
            }
            if target.filters.len() > MAX_LOOP_PATH_STATES {
                target.filters.clear();
                target.filters.push(SelectorFilter::default());
            }
        } else {
            self.targets.push(DelegateTarget { contract, filters });
        }
    }

    fn branch_paths(
        paths: &[PathState],
        guard: Option<(Selector, bool)>,
        condition_is_true: bool,
    ) -> Vec<PathState> {
        paths
            .iter()
            .filter_map(|path| {
                let mut path = path.clone();
                if let Some((selector, matches)) = guard {
                    path.selector_filter =
                        path.selector_filter.with_guard(selector, matches == condition_is_true)?;
                }
                Some(path)
            })
            .collect()
    }

    fn dedup_paths(&mut self) {
        let mut seen = HashSet::with_capacity(self.paths.len());
        self.paths.retain(|path| seen.insert(path.clone()));
    }

    fn extend_unique(paths: &mut Vec<PathState>, new_paths: impl IntoIterator<Item = PathState>) {
        for path in new_paths {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }

    fn visit_condition(&mut self, expr: &'hir Expr<'hir>) -> (Vec<PathState>, Vec<PathState>) {
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
                    Self::extend_unique(&mut rhs_false, lhs_false);
                    (rhs_true, rhs_false)
                } else {
                    self.paths = lhs_false;
                    let (mut rhs_true, rhs_false) = self.visit_condition(rhs);
                    Self::extend_unique(&mut rhs_true, lhs_true);
                    (rhs_true, rhs_false)
                }
            }
            ExprKind::Ternary(condition, true_expr, false_expr) => {
                let (condition_true, condition_false) = self.visit_condition(condition);

                self.paths = condition_true;
                let (mut true_paths, mut false_paths) = self.visit_condition(true_expr);

                self.paths = condition_false;
                let (false_arm_true, false_arm_false) = self.visit_condition(false_expr);
                Self::extend_unique(&mut true_paths, false_arm_true);
                Self::extend_unique(&mut false_paths, false_arm_false);
                (true_paths, false_paths)
            }
            _ => {
                let _ = self.visit_expr(expr);
                let paths = std::mem::take(&mut self.paths);
                let guard = selector_guard(self.gcx, expr);
                (Self::branch_paths(&paths, guard, true), Self::branch_paths(&paths, guard, false))
            }
        }
    }

    fn visit_loop_stmts(&mut self, stmts: &'hir [Stmt<'hir>]) -> (Vec<PathState>, LoopControl) {
        self.loop_controls.push(LoopControl::default());
        for stmt in stmts {
            let _ = self.visit_stmt(stmt);
        }
        let paths = std::mem::take(&mut self.paths);
        let control = self.loop_controls.pop().expect("loop control stack is not empty");
        (paths, control)
    }

    fn widen_loop_paths(paths: &mut Vec<PathState>) {
        let Some(first) = paths.first() else { return };
        let mut modified_inputs = first.modified_inputs.clone();
        modified_inputs
            .retain(|input| paths.iter().skip(1).all(|path| path.modified_inputs.contains(input)));
        paths.clear();
        paths.push(PathState { selector_filter: SelectorFilter::default(), modified_inputs });
    }

    fn visit_for_iteration(
        &mut self,
        block: &hir::Block<'hir>,
    ) -> Option<(Vec<PathState>, Vec<PathState>)> {
        let [stmt] = block.stmts else { return None };
        let (condition, body, else_stmt) = match &stmt.kind {
            StmtKind::If(condition, then_stmt, else_stmt) => {
                let StmtKind::Block(body) = &then_stmt.kind else { return None };
                (Some(*condition), body, *else_stmt)
            }
            StmtKind::Block(body) => (None, body, None),
            _ => return None,
        };
        if body.span != block.span {
            return None;
        }
        let (update, body) = body.stmts.split_last()?;
        if !matches!(update.kind, StmtKind::Expr(_)) {
            return None;
        }

        let mut exits = Vec::new();
        if let Some(condition) = condition {
            let (true_paths, false_paths) = self.visit_condition(condition);

            self.paths = false_paths;
            if let Some(else_stmt) = else_stmt {
                let (fallthrough, control) = self.visit_loop_stmts(std::slice::from_ref(else_stmt));
                Self::extend_unique(&mut exits, control.breaks);
                Self::extend_unique(&mut exits, fallthrough);
            } else {
                Self::extend_unique(&mut exits, std::mem::take(&mut self.paths));
            }

            self.paths = true_paths;
        }

        let (mut update_paths, control) = self.visit_loop_stmts(body);
        Self::extend_unique(&mut exits, control.breaks);
        Self::extend_unique(&mut update_paths, control.continues);
        self.paths = update_paths;
        let _ = self.visit_stmt(update);
        Some((std::mem::take(&mut self.paths), exits))
    }

    fn visit_loop(&mut self, block: &hir::Block<'hir>, source: hir::LoopSource) {
        let mut pending = std::mem::take(&mut self.paths);
        let mut seen = HashSet::new();
        let mut exits = Vec::new();

        loop {
            pending.retain(|path| seen.insert(path.clone()));
            if pending.is_empty() {
                break;
            }

            self.paths = std::mem::take(&mut pending);
            let next = if source == hir::LoopSource::For
                && let Some((next, for_exits)) = self.visit_for_iteration(block)
            {
                Self::extend_unique(&mut exits, for_exits);
                next
            } else if source == hir::LoopSource::DoWhile
                && let Some((condition, body)) = block.stmts.split_last()
            {
                let (mut condition_paths, control) = self.visit_loop_stmts(body);
                Self::extend_unique(&mut exits, control.breaks);
                Self::extend_unique(&mut condition_paths, control.continues);

                self.paths = condition_paths;
                let (mut next, control) = self.visit_loop_stmts(std::slice::from_ref(condition));
                Self::extend_unique(&mut exits, control.breaks);
                Self::extend_unique(&mut next, control.continues);
                next
            } else {
                let (mut next, control) = self.visit_loop_stmts(block.stmts);
                Self::extend_unique(&mut exits, control.breaks);
                Self::extend_unique(&mut next, control.continues);
                next
            };
            Self::extend_unique(&mut pending, next);
            if seen.len() + pending.len() > MAX_LOOP_PATH_STATES {
                Self::widen_loop_paths(&mut pending);
            }
            if exits.len() > MAX_LOOP_PATH_STATES {
                Self::widen_loop_paths(&mut exits);
            }
        }

        self.paths = exits;
    }
}

impl<'hir> Visit<'hir> for DelegateTargetCollector<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if self.paths.is_empty() {
            return ControlFlow::Continue(());
        }

        if let ExprKind::Ternary(condition, true_expr, false_expr) = &expr.kind {
            let (true_paths, false_paths) = self.visit_condition(condition);

            self.paths = true_paths;
            let _ = self.visit_expr(true_expr);
            let mut joined = std::mem::take(&mut self.paths);

            self.paths = false_paths;
            let _ = self.visit_expr(false_expr);
            joined.append(&mut self.paths);
            self.paths = joined;
            self.dedup_paths();
            return ControlFlow::Continue(());
        }

        if let ExprKind::Binary(_, op, _) = &expr.kind
            && matches!(op.kind, BinOpKind::And | BinOpKind::Or)
        {
            let (mut true_paths, false_paths) = self.visit_condition(expr);
            Self::extend_unique(&mut true_paths, false_paths);
            self.paths = true_paths;
            self.dedup_paths();
            return ControlFlow::Continue(());
        }

        if let ExprKind::Call(callee, args, opts) = &expr.kind
            && is_require_or_assert(callee)
        {
            let _ = self.visit_expr(callee);
            if let Some(opts) = opts {
                for arg in opts.args {
                    let _ = self.visit_expr(&arg.value);
                }
            }

            let mut args = args.exprs();
            let Some(condition) = args.next() else { return ControlFlow::Continue(()) };
            let args = args.collect::<Vec<_>>();
            let (true_paths, false_paths) = self.visit_condition(condition);

            self.paths = true_paths;
            for &arg in &args {
                let _ = self.visit_expr(arg);
            }
            let continuing_paths = std::mem::take(&mut self.paths);

            // Remaining arguments are evaluated before `require`/`assert` decides whether to
            // revert, so preserve their targets and side effects on the failing paths too.
            self.paths = false_paths;
            for arg in args {
                let _ = self.visit_expr(arg);
            }
            self.paths = continuing_paths;
            return ControlFlow::Continue(());
        }

        if let ExprKind::Call(callee, args, opts) = &expr.kind {
            let _ = self.visit_expr(callee);
            if let Some(opts) = opts {
                for arg in opts.args {
                    let _ = self.visit_expr(&arg.value);
                }
            }
            for arg in args.exprs() {
                let _ = self.visit_expr(arg);
            }
            if let Some((target, required_input)) =
                delegated_contract(self.gcx, &self.current_inputs, expr)
            {
                self.record_target(target, required_input);
            }
            return ControlFlow::Continue(());
        }

        let mutated_inputs = match &expr.peel_parens().kind {
            ExprKind::Assign(lhs, _, _) => self
                .current_inputs
                .iter()
                .copied()
                .filter(|&input| lvalue_contains_var(lhs, input.variable()))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let flow = self.walk_expr(expr);
        if !mutated_inputs.is_empty() {
            for path in &mut self.paths {
                for &input in &mutated_inputs {
                    path.mark_input_modified(input);
                }
            }
        }
        flow
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        if self.paths.is_empty() {
            return ControlFlow::Continue(());
        }

        if let StmtKind::If(condition, then_stmt, else_stmt) = &stmt.kind {
            let (true_paths, false_paths) = self.visit_condition(condition);

            self.paths = true_paths;
            let _ = self.visit_stmt(then_stmt);
            let mut joined = std::mem::take(&mut self.paths);

            self.paths = false_paths;
            if let Some(else_stmt) = else_stmt {
                let _ = self.visit_stmt(else_stmt);
            }
            joined.append(&mut self.paths);
            self.paths = joined;
            self.dedup_paths();
            return ControlFlow::Continue(());
        }

        if let StmtKind::Try(try_) = &stmt.kind {
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
            self.dedup_paths();
            return ControlFlow::Continue(());
        }

        if let StmtKind::Loop(block, source) = &stmt.kind {
            self.visit_loop(block, *source);
            return ControlFlow::Continue(());
        }

        if matches!(stmt.kind, StmtKind::Break | StmtKind::Continue) {
            let paths = std::mem::take(&mut self.paths);
            if let Some(control) = self.loop_controls.last_mut() {
                let destination = if matches!(stmt.kind, StmtKind::Break) {
                    &mut control.breaks
                } else {
                    &mut control.continues
                };
                Self::extend_unique(destination, paths);
            }
            return ControlFlow::Continue(());
        }

        if matches!(stmt.kind, StmtKind::Placeholder) {
            if let Some((modifiers, index, body, body_input)) = self.placeholder {
                self.visit_continuation(modifiers, index, body, body_input);
            }
            return ControlFlow::Continue(());
        }

        if let StmtKind::Return(expr) = stmt.kind {
            if let Some(expr) = expr {
                let _ = self.visit_expr(expr);
            }
            let paths = std::mem::take(&mut self.paths);
            let returns =
                self.return_controls.last_mut().expect("return control stack is not empty");
            Self::extend_unique(returns, paths);
            return ControlFlow::Continue(());
        }

        if matches!(stmt.kind, StmtKind::AssemblyBlock(_)) {
            for path in &mut self.paths {
                for &input in &self.current_inputs {
                    path.mark_input_modified(input);
                }
            }
            return ControlFlow::Continue(());
        }
        let flow = self.walk_stmt(stmt);
        if branch_always_exits(stmt) {
            self.paths.clear();
        }
        flow
    }
}

fn selector_guard(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<(Selector, bool)> {
    let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind else { return None };
    let matches = match op.kind {
        BinOpKind::Eq => true,
        BinOpKind::Ne => false,
        _ => return None,
    };
    if is_msg_sig(lhs) {
        selected_function_selector(gcx, rhs).map(|selector| (selector, matches))
    } else if is_msg_sig(rhs) {
        selected_function_selector(gcx, lhs).map(|selector| (selector, matches))
    } else {
        None
    }
}

fn is_msg_sig(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Member(base, member)
            if member.name == sym::sig && is_builtin_named(base, sym::msg)
    )
}

fn selected_function_selector(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<Selector> {
    let expr = expr.peel_parens();
    let ExprKind::Member(function, member) = &expr.kind else { return None };
    if member.name != sym::selector
        || gcx.builtin_member(expr.id) != Some(Builtin::FunctionSelector)
    {
        return None;
    }
    let ResolvedMember::Res(hir::Res::Item(ItemId::Function(function))) =
        gcx.resolved_member(function.peel_parens().id)?
    else {
        return None;
    };
    Some(gcx.function_selector(function))
}

/// Returns the statically typed implementation contract for a proxy-style delegatecall.
fn delegated_contract<'hir>(
    gcx: Gcx<'hir>,
    full_calldata_inputs: &[CalldataInput],
    expr: &'hir Expr<'hir>,
) -> Option<(ContractId, Option<CalldataInput>)> {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return None };
    let ExprKind::Member(receiver, member) = &callee.peel_parens().kind else { return None };
    let required_input = forwards_full_calldata(args, full_calldata_inputs)?;
    if member.name != kw::Delegatecall
        || gcx.builtin_callee(callee.id) != Some(Builtin::AddressDelegatecall)
        || !gcx.type_of_expr(receiver.peel_parens().id).is_some_and(ty_is_address)
    {
        return None;
    }
    typed_contract_behind_address_cast(gcx, receiver).map(|contract| (contract, required_input))
}

fn typed_contract_behind_address_cast<'hir>(
    gcx: Gcx<'hir>,
    expr: &'hir Expr<'hir>,
) -> Option<ContractId> {
    let expr = expr.peel_parens();
    if let Some(ty) = gcx.type_of_expr(expr.id)
        && let TyKind::Contract(id) = ty.peel_refs().kind
    {
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

fn forwards_full_calldata(
    args: &CallArgs<'_>,
    full_calldata_inputs: &[CalldataInput],
) -> Option<Option<CalldataInput>> {
    let arg = args.exprs().next()?;
    full_calldata_source(arg, full_calldata_inputs)
}

fn full_calldata_source(
    expr: &Expr<'_>,
    full_calldata_inputs: &[CalldataInput],
) -> Option<Option<CalldataInput>> {
    if matches!(
        &expr.peel_parens().kind,
        ExprKind::Member(base, member)
            if member.name == sym::data && is_builtin_named(base, sym::msg)
    ) {
        return Some(None);
    }
    let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return None };
    reses.iter().find_map(|res| {
        let hir::Res::Item(ItemId::Variable(id)) = res else { return None };
        full_calldata_inputs.iter().copied().find(|input| input.variable() == *id).map(Some)
    })
}

fn modifier_input_bindings<'hir>(
    hir: &'hir hir::Hir<'hir>,
    modifier: &'hir hir::Function<'hir>,
    args: &'hir CallArgs<'hir>,
    full_calldata_inputs: &[CalldataInput],
    modifier_index: usize,
) -> Vec<(CalldataInput, Option<CalldataInput>)> {
    modifier
        .parameters
        .iter()
        .copied()
        .filter_map(|param| {
            let arg = arg_for_param(hir, modifier, param, args)?;
            let input = CalldataInput::Modifier { index: modifier_index, param };
            Some((input, full_calldata_source(arg, full_calldata_inputs)?))
        })
        .collect()
}

fn arg_for_param<'hir>(
    hir: &'hir hir::Hir<'hir>,
    function: &'hir hir::Function<'hir>,
    param: hir::VariableId,
    args: &'hir CallArgs<'hir>,
) -> Option<&'hir Expr<'hir>> {
    let param_idx = function.parameters.iter().position(|candidate| *candidate == param)?;
    match args.kind {
        hir::CallArgsKind::Unnamed(exprs) => exprs.get(param_idx),
        hir::CallArgsKind::Named(named) => {
            let param_name = hir.variable(param).name?;
            named.iter().find(|arg| arg.name.name == param_name.name).map(|arg| &arg.value)
        }
    }
}

fn is_builtin_named(expr: &Expr<'_>, name: solar::interface::Symbol) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| matches!(res, hir::Res::Builtin(b) if b.name() == name))
    )
}

fn is_address_cast(callee: &Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Type(hir::Type {
            kind: TypeKind::Elementary(hir::ElementaryType::Address(_)),
            ..
        })
    )
}

fn ty_is_address(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(hir::ElementaryType::Address(_)))
}
