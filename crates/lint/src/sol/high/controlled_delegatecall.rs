use super::ControlledDelegatecall;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{self, LitKind},
    interface::{Span, Symbol, data_structures::Never, kw, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, CallArgs, ElementaryType, ExprKind, FunctionId, FunctionKind, ItemId, LoopSource,
            Res, StmtKind, TypeKind, Visit,
        },
        ty::{Ty, TyKind},
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    CONTROLLED_DELEGATECALL,
    Severity::High,
    "controlled-delegatecall",
    "delegatecall target is not provably trusted"
);

impl<'hir> LateLintPass<'hir> for ControlledDelegatecall {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        let Some(body) = func.body else { return };
        let mut analyzer = Analyzer::new(gcx, hir);
        for modifier in func.modifiers {
            collect_modifier_safety(gcx, hir, modifier, &mut analyzer.safe_vars);
        }
        for stmt in body.stmts {
            let _ = analyzer.visit_stmt(stmt);
            if branch_always_exits(stmt) {
                break;
            }
        }
        for span in analyzer.hits {
            ctx.emit(&CONTROLLED_DELEGATECALL, span);
        }
    }
}

struct Analyzer<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    safe_vars: HashSet<hir::VariableId>,
    hits: Vec<Span>,
}

#[derive(Clone)]
struct FlowState {
    safe_vars: HashSet<hir::VariableId>,
}

impl FlowState {
    fn intersection(a: &Self, b: &Self) -> Self {
        Self { safe_vars: a.safe_vars.intersection(&b.safe_vars).copied().collect() }
    }

    fn intersection_all(mut states: impl Iterator<Item = Self>) -> Self {
        let mut out = states.next().unwrap_or_else(|| Self { safe_vars: HashSet::new() });
        for state in states {
            out = Self::intersection(&out, &state);
        }
        out
    }
}

const HELPER_DEPTH: u8 = 3;

impl<'hir> Analyzer<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir hir::Hir<'hir>) -> Self {
        Self { gcx, hir, safe_vars: HashSet::new(), hits: Vec::new() }
    }

    fn snapshot(&self) -> FlowState {
        FlowState { safe_vars: self.safe_vars.clone() }
    }

    fn restore(&mut self, state: FlowState) {
        self.safe_vars = state.safe_vars;
    }

    fn is_trusted_target(&self, expr: &'hir hir::Expr<'hir>) -> bool {
        self.is_trusted_target_inner(expr, HELPER_DEPTH)
    }

    fn is_trusted_target_inner(&self, expr: &'hir hir::Expr<'hir>, depth: u8) -> bool {
        match &expr.peel_parens().kind {
            ExprKind::Lit(lit) => match &lit.kind {
                LitKind::Address(_) => true,
                LitKind::Number(n) => n.is_zero(),
                _ => false,
            },
            ExprKind::Ident(reses) => reses.iter().any(|res| match res {
                Res::Builtin(builtin) => builtin.name() == sym::this,
                Res::Item(ItemId::Variable(vid)) => self.is_trusted_var(*vid),
                _ => false,
            }),
            ExprKind::Call(callee, args, _)
                if is_address_like_cast_callee(callee) || is_numeric_cast_callee(callee) =>
            {
                args.exprs().next().is_some_and(|arg| self.is_trusted_target_inner(arg, depth))
            }
            ExprKind::Payable(inner) => self.is_trusted_target_inner(inner, depth),
            ExprKind::Ternary(_, if_true, if_false) => {
                self.is_trusted_target_inner(if_true, depth)
                    && self.is_trusted_target_inner(if_false, depth)
            }
            ExprKind::Assign(_, _, rhs) => self.is_trusted_target_inner(rhs, depth),
            ExprKind::Call(callee, args, _)
                if depth > 0
                    && args.exprs().next().is_none()
                    && callee_no_arg_returns(self.hir, callee, |e| {
                        self.is_trusted_target_inner(e, depth - 1)
                    }) =>
            {
                true
            }
            _ => false,
        }
    }

    fn is_trusted_var(&self, vid: hir::VariableId) -> bool {
        let var = self.hir.variable(vid);
        (var.is_constant() && var_is_address_like(var)) || self.safe_vars.contains(&vid)
    }

    fn handle_assign(
        &mut self,
        lhs: &'hir hir::Expr<'hir>,
        op: Option<hir::BinOp>,
        rhs: &'hir hir::Expr<'hir>,
    ) {
        let lhs = lhs.peel_parens();
        if let ExprKind::Tuple(lhs_elems) = &lhs.kind {
            let rhs_elems = tuple_elems(rhs);
            for (i, lhs_elem) in lhs_elems.iter().enumerate() {
                if let Some(lhs_expr) = lhs_elem {
                    self.assign_one(
                        lhs_expr,
                        op.is_none().then(|| tuple_slot(rhs_elems, i)).flatten(),
                    );
                }
            }
        } else {
            self.assign_one(lhs, op.is_none().then_some(rhs));
        }
    }

    fn assign_one(&mut self, lhs: &'hir hir::Expr<'hir>, rhs: Option<&'hir hir::Expr<'hir>>) {
        let Some(var) = underlying_var(lhs) else { return };
        self.safe_vars.remove(&var);
        let target = self.hir.variable(var);
        if target.kind.is_state() || !var_is_address_like(target) {
            return;
        }
        if rhs.is_some_and(|expr| self.is_trusted_target(expr)) {
            self.safe_vars.insert(var);
        }
    }

    fn delete_one(&mut self, lhs: &'hir hir::Expr<'hir>) {
        let Some(var) = underlying_var(lhs) else { return };
        self.safe_vars.remove(&var);
        let target = self.hir.variable(var);
        if !target.kind.is_state() && var_is_address_like(target) {
            self.safe_vars.insert(var);
        }
    }

    fn handle_decl(&mut self, var: hir::VariableId) {
        let variable = self.hir.variable(var);
        if !var_is_address_like(variable) {
            return;
        }
        if let Some(init) = variable.initializer
            && self.is_trusted_target(init)
        {
            self.safe_vars.insert(var);
        }
    }

    fn is_controlled_delegatecall(&self, expr: &'hir hir::Expr<'hir>) -> bool {
        let ExprKind::Call(callee, _, _) = &expr.peel_parens().kind else {
            return false;
        };
        let ExprKind::Member(receiver, member) = &callee.peel_parens().kind else {
            return false;
        };
        member.name == kw::Delegatecall
            && receiver_is_address(self.gcx, receiver)
            && !self.is_trusted_target(receiver)
    }

    fn add_facts(&mut self, pred: &'hir hir::Expr<'hir>, negate: bool) {
        if expr_has_fact_side_effect(pred) {
            return;
        }
        match &pred.peel_parens().kind {
            ExprKind::Binary(lhs, op, rhs) => {
                let (eq, and_op, or_op) = if negate {
                    (ast::BinOpKind::Ne, ast::BinOpKind::Or, ast::BinOpKind::And)
                } else {
                    (ast::BinOpKind::Eq, ast::BinOpKind::And, ast::BinOpKind::Or)
                };
                if op.kind == and_op {
                    self.add_facts(lhs, negate);
                    self.add_facts(rhs, negate);
                } else if op.kind == or_op {
                    self.add_facts_disjunction(lhs, rhs, negate);
                } else if op.kind == eq {
                    for (safe, candidate) in [(lhs, rhs), (rhs, lhs)] {
                        if self.is_trusted_target(safe)
                            && let Some(var) = underlying_var(candidate)
                            && self.is_trusted_fact_target(var)
                        {
                            self.safe_vars.insert(var);
                        }
                    }
                }
            }
            ExprKind::Unary(op, inner) if matches!(op.kind, ast::UnOpKind::Not) => {
                self.add_facts(inner, !negate);
            }
            _ => {}
        }
    }

    fn add_facts_unchecked(&mut self, pred: &'hir hir::Expr<'hir>, negate: bool) {
        match &pred.peel_parens().kind {
            ExprKind::Binary(lhs, op, rhs) => {
                let (eq, and_op, or_op) = if negate {
                    (ast::BinOpKind::Ne, ast::BinOpKind::Or, ast::BinOpKind::And)
                } else {
                    (ast::BinOpKind::Eq, ast::BinOpKind::And, ast::BinOpKind::Or)
                };
                if op.kind == and_op {
                    self.add_facts_unchecked(lhs, negate);
                    self.add_facts_unchecked(rhs, negate);
                } else if op.kind == or_op {
                    self.add_facts_disjunction(lhs, rhs, negate);
                } else if op.kind == eq {
                    for (safe, candidate) in [(lhs, rhs), (rhs, lhs)] {
                        if self.is_trusted_target(safe)
                            && let Some(var) = underlying_var(candidate)
                            && self.is_trusted_fact_target(var)
                        {
                            self.safe_vars.insert(var);
                        }
                    }
                }
            }
            ExprKind::Unary(op, inner) if matches!(op.kind, ast::UnOpKind::Not) => {
                self.add_facts_unchecked(inner, !negate);
            }
            _ => {}
        }
    }

    fn add_facts_disjunction(
        &mut self,
        lhs: &'hir hir::Expr<'hir>,
        rhs: &'hir hir::Expr<'hir>,
        negate: bool,
    ) {
        let baseline = self.safe_vars.clone();
        self.add_facts_unchecked(lhs, negate);
        let lhs_added: HashSet<_> = self.safe_vars.difference(&baseline).copied().collect();
        self.safe_vars.clone_from(&baseline);
        self.add_facts_unchecked(rhs, negate);
        let rhs_added: HashSet<_> = self.safe_vars.difference(&baseline).copied().collect();
        self.safe_vars = baseline;
        for var in lhs_added.intersection(&rhs_added) {
            self.safe_vars.insert(*var);
        }
    }

    fn is_trusted_fact_target(&self, var: hir::VariableId) -> bool {
        let variable = self.hir.variable(var);
        (!variable.kind.is_state() || variable.is_constant()) && var_is_address_like(variable)
    }

    fn visit_isolated(&mut self, stmts: &'hir [hir::Stmt<'hir>]) {
        let mut exits = vec![self.snapshot()];
        if let Some(fallthrough) = self.visit_stmts_until_loop_exit(stmts, &mut exits) {
            exits.push(fallthrough);
        }
        self.restore(FlowState::intersection_all(exits.into_iter()));
    }

    fn visit_stmts_until_loop_exit(
        &mut self,
        stmts: &'hir [hir::Stmt<'hir>],
        exits: &mut Vec<FlowState>,
    ) -> Option<FlowState> {
        for stmt in stmts {
            self.visit_stmt_until_loop_exit(stmt, exits)?;
        }
        Some(self.snapshot())
    }

    fn visit_stmt_until_loop_exit(
        &mut self,
        stmt: &'hir hir::Stmt<'hir>,
        exits: &mut Vec<FlowState>,
    ) -> Option<()> {
        match &stmt.kind {
            StmtKind::Break | StmtKind::Continue => {
                exits.push(self.snapshot());
                None
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                let state = self.visit_stmts_until_loop_exit(block.stmts, exits)?;
                self.restore(state);
                Some(())
            }
            StmtKind::If(cond, then, else_) => {
                let _ = self.visit_expr(cond);
                let baseline = self.snapshot();
                self.add_facts(cond, false);
                let then_fallthrough = self
                    .visit_stmt_until_loop_exit(then, exits)
                    .and_then(|_| (!branch_always_exits(then)).then(|| self.snapshot()));
                self.restore(baseline);
                self.add_facts(cond, true);
                let else_fallthrough = match else_ {
                    Some(else_stmt) => self
                        .visit_stmt_until_loop_exit(else_stmt, exits)
                        .and_then(|_| (!branch_always_exits(else_stmt)).then(|| self.snapshot())),
                    None => Some(self.snapshot()),
                };
                match (then_fallthrough, else_fallthrough) {
                    (Some(then_state), Some(else_state)) => {
                        self.restore(FlowState::intersection(&then_state, &else_state));
                        Some(())
                    }
                    (Some(state), None) | (None, Some(state)) => {
                        self.restore(state);
                        Some(())
                    }
                    (None, None) => None,
                }
            }
            StmtKind::Loop(..) => {
                let _ = self.visit_stmt(stmt);
                Some(())
            }
            _ => {
                let _ = self.visit_stmt(stmt);
                (!branch_always_exits(stmt)).then_some(())
            }
        }
    }
}

impl<'hir> Visit<'hir> for Analyzer<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match &stmt.kind {
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                for stmt in block.stmts {
                    let _ = self.visit_stmt(stmt);
                    if branch_always_exits(stmt) {
                        break;
                    }
                }
                return ControlFlow::Continue(());
            }
            StmtKind::If(cond, then, else_) => {
                let _ = self.visit_expr(cond);
                let baseline = self.snapshot();
                self.add_facts(cond, false);
                let _ = self.visit_stmt(then);
                let then_exits = branch_always_exits(then);
                let after_then = self.snapshot();
                self.restore(baseline);
                self.add_facts(cond, true);
                let else_exits = match else_ {
                    Some(else_stmt) => {
                        let _ = self.visit_stmt(else_stmt);
                        branch_always_exits(else_stmt)
                    }
                    None => false,
                };
                let after_else = self.snapshot();
                let joined = match (then_exits, else_exits) {
                    (true, false) => after_else,
                    (false, true) => after_then,
                    _ => FlowState::intersection(&after_then, &after_else),
                };
                self.restore(joined);
                return ControlFlow::Continue(());
            }
            StmtKind::Loop(block, source) => {
                if matches!(source, LoopSource::DoWhile)
                    && !do_while_user_stmts(block.stmts).iter().any(stmt_has_break_or_continue)
                {
                    for stmt in do_while_user_stmts(block.stmts) {
                        let _ = self.visit_stmt(stmt);
                        if branch_always_exits(stmt) {
                            break;
                        }
                    }
                    if let Some(cond) = do_while_lowered_condition(block.stmts) {
                        let _ = self.visit_expr(cond);
                    }
                } else {
                    self.visit_isolated(block.stmts);
                }
                return ControlFlow::Continue(());
            }
            StmtKind::Try(stmt_try) => {
                let _ = self.visit_expr(&stmt_try.expr);
                let outer = self.snapshot();
                let mut clause_exits = Vec::new();
                for clause in stmt_try.clauses {
                    self.restore(outer.clone());
                    let mut exited = false;
                    for stmt in clause.block.stmts {
                        let _ = self.visit_stmt(stmt);
                        if branch_always_exits(stmt) {
                            exited = true;
                            break;
                        }
                    }
                    if !exited {
                        clause_exits.push(self.snapshot());
                    }
                }
                self.restore(
                    clause_exits
                        .into_iter()
                        .reduce(|a, b| FlowState::intersection(&a, &b))
                        .unwrap_or(outer),
                );
                return ControlFlow::Continue(());
            }
            StmtKind::Err(_) => {
                self.safe_vars.clear();
            }
            StmtKind::DeclSingle(var) => self.handle_decl(*var),
            StmtKind::DeclMulti(vars, init) => {
                if let ExprKind::Tuple(exprs) = &init.peel_parens().kind {
                    for (var, expr) in vars.iter().zip(exprs.iter()) {
                        let (Some(var), Some(expr)) = (var, expr) else { continue };
                        self.assign_one_var(*var, Some(expr));
                    }
                } else {
                    for var in vars.iter().flatten() {
                        self.assign_one_var(*var, None);
                    }
                }
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Binary(lhs, op, rhs) = &expr.kind
            && matches!(op.kind, ast::BinOpKind::And | ast::BinOpKind::Or)
        {
            let _ = self.visit_expr(lhs);
            let negate = matches!(op.kind, ast::BinOpKind::Or);
            let skipped_rhs = self.snapshot();
            self.add_facts(lhs, negate);
            let result = self.visit_expr(rhs);
            let ran_rhs = self.snapshot();
            self.restore(FlowState::intersection(&skipped_rhs, &ran_rhs));
            return result;
        }
        if let ExprKind::Ternary(cond, then_expr, else_expr) = &expr.kind {
            let _ = self.visit_expr(cond);
            let pre_arm = self.snapshot();
            self.add_facts(cond, false);
            let _ = self.visit_expr(then_expr);
            let post_then = self.snapshot();
            self.restore(pre_arm);
            self.add_facts(cond, true);
            let _ = self.visit_expr(else_expr);
            let post_else = self.snapshot();
            self.restore(FlowState::intersection(&post_then, &post_else));
            return ControlFlow::Continue(());
        }
        if self.is_controlled_delegatecall(expr) {
            self.hits.push(expr.span);
        }
        match &expr.kind {
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                let result = self.walk_expr(expr);
                if let Some(cond) = args.exprs().next() {
                    let mut args = args.exprs();
                    let _ = args.next();
                    if !args.any(expr_has_fact_side_effect) {
                        self.add_facts(cond, false);
                    }
                }
                return result;
            }
            ExprKind::Assign(lhs, op, rhs) => {
                let result = self.walk_expr(expr);
                self.handle_assign(lhs, *op, rhs);
                return result;
            }
            ExprKind::Delete(target) => self.delete_one(target.peel_parens()),
            _ => {}
        }
        self.walk_expr(expr)
    }
}

impl<'hir> Analyzer<'hir> {
    fn assign_one_var(&mut self, var: hir::VariableId, rhs: Option<&'hir hir::Expr<'hir>>) {
        self.safe_vars.remove(&var);
        let variable = self.hir.variable(var);
        if variable.kind.is_state() || !var_is_address_like(variable) {
            return;
        }
        if rhs.is_some_and(|expr| self.is_trusted_target(expr)) {
            self.safe_vars.insert(var);
        }
    }
}

fn underlying_var(expr: &hir::Expr<'_>) -> Option<hir::VariableId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|res| match res {
            Res::Item(ItemId::Variable(vid)) => Some(*vid),
            _ => None,
        }),
        ExprKind::Call(callee, args, _)
            if is_address_like_cast_callee(callee) || is_numeric_cast_callee(callee) =>
        {
            args.exprs().next().and_then(underlying_var)
        }
        ExprKind::Payable(inner) => underlying_var(inner),
        _ => None,
    }
}

fn tuple_elems<'hir>(expr: &'hir hir::Expr<'hir>) -> Option<&'hir [Option<&'hir hir::Expr<'hir>>]> {
    match &expr.peel_parens().kind {
        ExprKind::Tuple(elems) => Some(*elems),
        _ => None,
    }
}

fn tuple_slot<'hir>(
    elems: Option<&'hir [Option<&'hir hir::Expr<'hir>>]>,
    idx: usize,
) -> Option<&'hir hir::Expr<'hir>> {
    elems.and_then(|elems| elems.get(idx).copied().flatten())
}

const fn var_is_address_like(var: &hir::Variable<'_>) -> bool {
    matches!(
        var.ty.kind,
        TypeKind::Elementary(ElementaryType::Address(_)) | TypeKind::Custom(ItemId::Contract(_))
    )
}

fn receiver_is_address<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    gcx.type_of_expr(expr.peel_parens().id).is_some_and(ty_is_address)
}

fn is_address_like_cast_callee(callee: &hir::Expr<'_>) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Type(hir::Type {
            kind: TypeKind::Elementary(ElementaryType::Address(_)),
            ..
        }) => true,
        ExprKind::Ident(reses) => {
            !reses.is_empty()
                && reses.iter().all(|res| matches!(res, Res::Item(ItemId::Contract(_))))
        }
        _ => false,
    }
}

fn is_numeric_cast_callee(callee: &hir::Expr<'_>) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) => {
            matches!(ty, ElementaryType::Int(_) | ElementaryType::UInt(_) | ElementaryType::Bytes)
        }
        _ => false,
    }
}

fn ty_is_address(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::Address(_)))
}

fn callee_no_arg_returns<'hir>(
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
    mut pred: impl FnMut(&'hir hir::Expr<'hir>) -> bool,
) -> bool {
    let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return false };
    let fids: Vec<_> = reses
        .iter()
        .filter_map(|res| match res {
            Res::Item(ItemId::Function(fid)) => Some(*fid),
            _ => None,
        })
        .collect();
    let [fid] = fids.as_slice() else { return false };
    function_is_statically_trusted(hir.function(*fid))
        && function_no_arg_returns(hir, *fid, &mut pred)
}

const fn function_is_statically_trusted(func: &hir::Function<'_>) -> bool {
    !func.virtual_ && !func.override_
}

fn collect_modifier_safety<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    invocation: &'hir hir::Modifier<'hir>,
    out_safe: &mut HashSet<hir::VariableId>,
) {
    let ItemId::Function(fid) = invocation.id else { return };
    let Some((modifier, prefix)) = modifier_prefix(hir, fid) else { return };
    let arg_map: Vec<(hir::VariableId, hir::VariableId)> = modifier
        .parameters
        .iter()
        .filter_map(|&modifier_param| {
            let arg = arg_for_param(hir, modifier, modifier_param, &invocation.args)?;
            Some((modifier_param, underlying_var(arg)?))
        })
        .collect();
    if arg_map.is_empty() {
        return;
    }

    let mut assigned_params = HashSet::new();
    let mut collector = AssignedParamCollector { hir, out: &mut assigned_params };
    for stmt in &prefix {
        let _ = collector.visit_stmt(stmt);
    }

    let mut analyzer = Analyzer::new(gcx, hir);
    for stmt in &prefix {
        let _ = analyzer.visit_stmt(stmt);
        if branch_always_exits(stmt) {
            break;
        }
    }

    for (modifier_param, caller_var) in arg_map {
        if !assigned_params.contains(&modifier_param)
            && analyzer.safe_vars.contains(&modifier_param)
            && analyzer.is_trusted_fact_target(caller_var)
        {
            out_safe.insert(caller_var);
        }
    }
}

fn modifier_prefix<'hir>(
    hir: &'hir hir::Hir<'hir>,
    fid: FunctionId,
) -> Option<(&'hir hir::Function<'hir>, Vec<&'hir hir::Stmt<'hir>>)> {
    let modifier = hir.function(fid);
    if !matches!(modifier.kind, FunctionKind::Modifier) {
        return None;
    }
    let body = modifier.body?;
    if count_placeholders(body.stmts) != 1 {
        return None;
    }
    let mut prefix = Vec::new();
    collect_stmts_before_placeholder(body.stmts, &mut prefix)?;
    Some((modifier, prefix))
}

fn collect_stmts_before_placeholder<'hir>(
    stmts: &'hir [hir::Stmt<'hir>],
    out: &mut Vec<&'hir hir::Stmt<'hir>>,
) -> Option<()> {
    for (i, stmt) in stmts.iter().enumerate() {
        match &stmt.kind {
            StmtKind::Placeholder => {
                out.extend(stmts[..i].iter());
                return Some(());
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block)
                if count_placeholders(block.stmts) >= 1 =>
            {
                out.extend(stmts[..i].iter());
                return collect_stmts_before_placeholder(block.stmts, out);
            }
            _ => {
                if count_placeholders_in_stmt(stmt) > 0 {
                    return None;
                }
            }
        }
    }
    None
}

fn arg_for_param<'hir>(
    hir: &'hir hir::Hir<'hir>,
    function: &hir::Function<'hir>,
    param: hir::VariableId,
    args: &'hir CallArgs<'hir>,
) -> Option<&'hir hir::Expr<'hir>> {
    let param_idx = function.parameters.iter().position(|p| *p == param)?;
    match args.kind {
        hir::CallArgsKind::Unnamed(exprs) => exprs.get(param_idx),
        hir::CallArgsKind::Named(named) => {
            let param_name = hir.variable(param).name?;
            named.iter().find(|arg| arg.name.name == param_name.name).map(|arg| &arg.value)
        }
    }
}

struct AssignedParamCollector<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    out: &'a mut HashSet<hir::VariableId>,
}

impl AssignedParamCollector<'_, '_> {
    fn add_lhs(&mut self, lhs: &hir::Expr<'_>) {
        match &lhs.peel_parens().kind {
            ExprKind::Tuple(elems) => {
                for expr in elems.iter().flatten() {
                    self.add_lhs(expr);
                }
            }
            _ => {
                if let Some(var) = underlying_var(lhs) {
                    self.out.insert(var);
                }
            }
        }
    }
}

impl<'hir> Visit<'hir> for AssignedParamCollector<'_, 'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.peel_parens().kind {
            ExprKind::Assign(lhs, _, _) => self.add_lhs(lhs),
            ExprKind::Delete(target) => self.add_lhs(target),
            _ => {}
        }
        self.walk_expr(expr)
    }
}

fn function_no_arg_returns<'hir>(
    hir: &'hir hir::Hir<'hir>,
    fid: FunctionId,
    pred: &mut impl FnMut(&'hir hir::Expr<'hir>) -> bool,
) -> bool {
    let func = hir.function(fid);
    let Some(body) = func.body else { return false };
    if !func.parameters.is_empty() {
        return false;
    }
    let stmts = match body.stmts.split_last() {
        Some((last, rest)) if matches!(last.kind, StmtKind::Return(None)) => rest,
        _ => body.stmts,
    };
    if stmts.len() != 1 {
        return false;
    }
    match &stmts[0].kind {
        StmtKind::Return(Some(expr)) => pred(expr),
        StmtKind::Expr(expr) => match &expr.peel_parens().kind {
            ExprKind::Assign(lhs, None, rhs) => {
                func.returns.len() == 1
                    && underlying_var(lhs).is_some_and(|var| var == func.returns[0])
                    && pred(rhs)
            }
            _ => false,
        },
        _ => false,
    }
}

fn is_require_or_assert(callee: &hir::Expr<'_>) -> bool {
    let ExprKind::Ident(reses) = &callee.kind else { return false };
    reses.iter().any(
        |res| matches!(res, Res::Builtin(builtin) if builtin.name() == sym::require || builtin.name() == sym::assert),
    )
}

fn branch_always_exits(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Revert(_) => true,
        StmtKind::Expr(expr) => is_exit_call(expr),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.iter().any(branch_always_exits)
        }
        StmtKind::If(_, then_stmt, Some(else_stmt)) => {
            branch_always_exits(then_stmt) && branch_always_exits(else_stmt)
        }
        StmtKind::Try(stmt_try) => {
            !stmt_try.clauses.is_empty()
                && stmt_try
                    .clauses
                    .iter()
                    .all(|clause| clause.block.stmts.iter().any(branch_always_exits))
        }
        _ => false,
    }
}

fn is_exit_call(expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return false };
    if is_builtin(callee, kw::Revert) {
        return true;
    }
    if let ExprKind::Ident(reses) = &callee.peel_parens().kind
        && reses.iter().any(|res| matches!(res, Res::Builtin(Builtin::Selfdestruct)))
    {
        return true;
    }
    if is_require_or_assert(callee)
        && let hir::CallArgsKind::Unnamed(unnamed) = args.kind
        && let Some(first) = unnamed.first()
        && matches!(
            &first.peel_parens().kind,
            ExprKind::Lit(lit) if matches!(lit.kind, ast::LitKind::Bool(false))
        )
    {
        return true;
    }
    false
}

fn expr_has_fact_side_effect(expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Assign(..) | ExprKind::Delete(_) => true,
        ExprKind::Unary(op, inner) => {
            matches!(
                op.kind,
                ast::UnOpKind::PreInc
                    | ast::UnOpKind::PreDec
                    | ast::UnOpKind::PostInc
                    | ast::UnOpKind::PostDec
            ) || expr_has_fact_side_effect(inner)
        }
        ExprKind::Array(exprs) => exprs.iter().any(expr_has_fact_side_effect),
        ExprKind::Binary(lhs, _, rhs) => {
            expr_has_fact_side_effect(lhs) || expr_has_fact_side_effect(rhs)
        }
        ExprKind::Call(callee, args, options) => {
            expr_has_fact_side_effect(callee)
                || args.exprs().any(expr_has_fact_side_effect)
                || options.is_some_and(|options| {
                    options.args.iter().any(|arg| expr_has_fact_side_effect(&arg.value))
                })
        }
        ExprKind::Index(base, index) => {
            expr_has_fact_side_effect(base) || index.is_some_and(expr_has_fact_side_effect)
        }
        ExprKind::Slice(base, start, end) => {
            expr_has_fact_side_effect(base)
                || start.is_some_and(expr_has_fact_side_effect)
                || end.is_some_and(expr_has_fact_side_effect)
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) | ExprKind::YulMember(base, _) => {
            expr_has_fact_side_effect(base)
        }
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            expr_has_fact_side_effect(cond)
                || expr_has_fact_side_effect(then_expr)
                || expr_has_fact_side_effect(else_expr)
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().any(|expr| expr_has_fact_side_effect(expr))
        }
        ExprKind::Lit(_)
        | ExprKind::Ident(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => false,
    }
}

/// Strips the trailing `if (...) break;` that lowers `do { ... } while (cond);`.
fn do_while_user_stmts<'a, 'hir>(stmts: &'a [hir::Stmt<'hir>]) -> &'a [hir::Stmt<'hir>] {
    if let Some((last, rest)) = stmts.split_last()
        && let StmtKind::If(_, then_stmt, else_stmt) = &last.kind
        && (is_break_stmt(then_stmt) || else_stmt.as_ref().is_some_and(|stmt| is_break_stmt(stmt)))
    {
        return rest;
    }
    stmts
}

fn do_while_lowered_condition<'hir>(
    stmts: &'hir [hir::Stmt<'hir>],
) -> Option<&'hir hir::Expr<'hir>> {
    let last = stmts.last()?;
    let StmtKind::If(cond, then_stmt, else_stmt) = &last.kind else { return None };
    (is_break_stmt(then_stmt) || else_stmt.as_ref().is_some_and(|stmt| is_break_stmt(stmt)))
        .then_some(*cond)
}

fn is_break_stmt(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Break => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.len() == 1 && is_break_stmt(&block.stmts[0])
        }
        _ => false,
    }
}

fn stmt_has_break_or_continue(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Break | StmtKind::Continue => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            block.stmts.iter().any(stmt_has_break_or_continue)
        }
        StmtKind::If(_, then_stmt, else_stmt) => {
            stmt_has_break_or_continue(then_stmt)
                || else_stmt.as_ref().is_some_and(|stmt| stmt_has_break_or_continue(stmt))
        }
        StmtKind::Try(stmt_try) => stmt_try
            .clauses
            .iter()
            .any(|clause| clause.block.stmts.iter().any(stmt_has_break_or_continue)),
        _ => false,
    }
}

fn count_placeholders(stmts: &[hir::Stmt<'_>]) -> usize {
    stmts.iter().map(count_placeholders_in_stmt).sum()
}

fn count_placeholders_in_stmt(stmt: &hir::Stmt<'_>) -> usize {
    match &stmt.kind {
        StmtKind::Placeholder => 1,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            count_placeholders(block.stmts)
        }
        StmtKind::If(_, then_stmt, else_stmt) => {
            count_placeholders_in_stmt(then_stmt)
                + else_stmt.as_ref().map_or(0, |stmt| count_placeholders_in_stmt(stmt))
        }
        StmtKind::Try(stmt_try) => {
            stmt_try.clauses.iter().map(|clause| count_placeholders(clause.block.stmts)).sum()
        }
        _ => 0,
    }
}

fn is_builtin(expr: &hir::Expr<'_>, name: Symbol) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| matches!(res, Res::Builtin(builtin) if builtin.name() == name))
    )
}
