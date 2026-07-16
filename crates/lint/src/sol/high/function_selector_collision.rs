use super::FunctionSelectorCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::primitives::branch_always_exits},
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
use std::{collections::HashSet, ops::ControlFlow};

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
            fallback_input: fallback.parameters.first().copied(),
            paths: vec![PathState::initial()],
            loop_controls: Vec::new(),
            targets: Vec::new(),
        };
        for stmt in body.stmts {
            let _ = collector.visit_stmt(stmt);
        }

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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PathState {
    selector_filter: SelectorFilter,
    fallback_input_unmodified: bool,
}

impl PathState {
    fn initial() -> Self {
        Self { selector_filter: SelectorFilter::default(), fallback_input_unmodified: true }
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
    fallback_input: Option<hir::VariableId>,
    paths: Vec<PathState>,
    loop_controls: Vec<LoopControl>,
    targets: Vec<DelegateTarget>,
}

impl<'hir> DelegateTargetCollector<'hir> {
    fn record_target(&mut self, contract: ContractId, requires_unmodified_input: bool) {
        let mut filters = Vec::new();
        for path in &self.paths {
            if (!requires_unmodified_input || path.fallback_input_unmodified)
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
        let has_unmodified = paths.iter().any(|path| path.fallback_input_unmodified);
        let has_modified = paths.iter().any(|path| !path.fallback_input_unmodified);
        paths.clear();
        if has_unmodified {
            paths.push(PathState::initial());
        }
        if has_modified {
            paths.push(PathState {
                selector_filter: SelectorFilter::default(),
                fallback_input_unmodified: false,
            });
        }
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
            if let Some((target, requires_unmodified_input)) =
                delegated_contract(self.gcx, self.fallback_input, expr)
            {
                self.record_target(target, requires_unmodified_input);
            }
            return ControlFlow::Continue(());
        }

        let mutates_input = self.fallback_input.is_some_and(|input| {
            matches!(
                &expr.peel_parens().kind,
                ExprKind::Assign(lhs, _, _) if lvalue_contains_var(lhs, input)
            )
        });
        let flow = self.walk_expr(expr);
        if mutates_input {
            for path in &mut self.paths {
                path.fallback_input_unmodified = false;
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

        if matches!(stmt.kind, StmtKind::AssemblyBlock(_)) {
            for path in &mut self.paths {
                path.fallback_input_unmodified = false;
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
    fallback_input: Option<hir::VariableId>,
    expr: &'hir Expr<'hir>,
) -> Option<(ContractId, bool)> {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return None };
    let ExprKind::Member(receiver, member) = &callee.peel_parens().kind else { return None };
    let requires_unmodified_input = forwards_full_calldata(args, fallback_input)?;
    if member.name != kw::Delegatecall
        || gcx.builtin_callee(callee.id) != Some(Builtin::AddressDelegatecall)
        || !gcx.type_of_expr(receiver.peel_parens().id).is_some_and(ty_is_address)
    {
        return None;
    }
    typed_contract_behind_address_cast(gcx, receiver)
        .map(|contract| (contract, requires_unmodified_input))
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
    fallback_input: Option<hir::VariableId>,
) -> Option<bool> {
    let arg = args.exprs().next()?;
    if matches!(
        &arg.peel_parens().kind,
        ExprKind::Member(base, member)
            if member.name == sym::data && is_builtin_named(base, sym::msg)
    ) {
        return Some(false);
    }
    let fallback_input = fallback_input?;
    matches!(
        &arg.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(
                |res| matches!(res, hir::Res::Item(ItemId::Variable(id)) if *id == fallback_input),
            )
    )
    .then_some(true)
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
