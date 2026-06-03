use super::ArbitrarySendEth;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{self, LitKind},
    interface::{Span, Symbol, data_structures::Never, kw, sym},
    sema::{
        Gcx, Ty,
        builtins::Builtin,
        hir::{
            self, CallArgs, ContractKind, ElementaryType, ExprKind, FunctionId, FunctionKind,
            ItemId, LoopSource, Res, StmtKind, TypeKind, Visit,
        },
        ty::TyKind,
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

declare_forge_lint!(
    ARBITRARY_SEND_ETH,
    Severity::High,
    "arbitrary-send-eth",
    "ETH is sent to a user-controlled destination; restrict the destination or the caller"
);

impl<'hir> LateLintPass<'hir> for ArbitrarySendEth {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if matches!(func.state_mutability, ast::StateMutability::Pure | ast::StateMutability::View)
            || matches!(func.kind, FunctionKind::Constructor)
            || func.contract.is_some_and(|cid| hir.contract(cid).kind == ContractKind::Library)
        {
            return;
        }
        let Some(body) = func.body else { return };
        let mut entry = Analyzer::new(gcx, hir);
        for m in func.modifiers {
            for arg in m.args.exprs() {
                let _ = entry.visit_expr(arg);
            }
        }
        for span in &entry.hits {
            ctx.emit(&ARBITRARY_SEND_ETH, *span);
        }
        let mut analyzer = Analyzer::new(gcx, hir);
        for m in func.modifiers {
            collect_modifier_safety(gcx, hir, m, &mut analyzer.safe_vars);
        }
        for stmt in body.stmts {
            let _ = analyzer.visit_stmt(stmt);
            if branch_always_exits(stmt) {
                break;
            }
        }
        if analyzer.hits.is_empty() {
            return;
        }
        if func.modifiers.iter().any(|m| modifier_restricts_caller(gcx, hir, m)) {
            return;
        }
        for span in analyzer.hits {
            ctx.emit(&ARBITRARY_SEND_ETH, span);
        }
    }
}

struct Analyzer<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    self_aliases: SelfAliasAnalysis<'hir>,
    /// Locals/non-state vars proven equal to a safe origin on this path.
    safe_vars: HashSet<hir::VariableId>,
    /// Function-pointer locals proven to route to `this` on this path.
    safe_fn_ptrs: HashSet<hir::VariableId>,
    /// True once a caller-restricting guard has fired on this path.
    caller_restricted: bool,
    hits: Vec<Span>,
}

#[derive(Clone)]
struct FlowState {
    safe_vars: HashSet<hir::VariableId>,
    safe_fn_ptrs: HashSet<hir::VariableId>,
    caller_restricted: bool,
}

impl FlowState {
    fn intersection(a: &Self, b: &Self) -> Self {
        Self {
            safe_vars: a.safe_vars.intersection(&b.safe_vars).copied().collect(),
            safe_fn_ptrs: a.safe_fn_ptrs.intersection(&b.safe_fn_ptrs).copied().collect(),
            caller_restricted: a.caller_restricted && b.caller_restricted,
        }
    }

    fn intersection_all(mut states: impl Iterator<Item = Self>) -> Self {
        let mut out = states.next().unwrap_or_else(|| Self {
            safe_vars: HashSet::new(),
            safe_fn_ptrs: HashSet::new(),
            caller_restricted: false,
        });
        for state in states {
            out = Self::intersection(&out, &state);
        }
        out
    }
}

/// Recursion budget for `_msgSender()`-style helper chains.
const HELPER_DEPTH: u8 = 3;

/// Recursion budget for self-alias chains.
const SELF_ALIAS_DEPTH: u8 = 8;

impl<'hir> Analyzer<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir hir::Hir<'hir>) -> Self {
        Self {
            gcx,
            hir,
            self_aliases: SelfAliasAnalysis::new(gcx, hir),
            safe_vars: HashSet::new(),
            safe_fn_ptrs: HashSet::new(),
            caller_restricted: false,
            hits: Vec::new(),
        }
    }

    fn snapshot(&self) -> FlowState {
        FlowState {
            safe_vars: self.safe_vars.clone(),
            safe_fn_ptrs: self.safe_fn_ptrs.clone(),
            caller_restricted: self.caller_restricted,
        }
    }

    fn restore(&mut self, state: FlowState) {
        self.safe_vars = state.safe_vars;
        self.safe_fn_ptrs = state.safe_fn_ptrs;
        self.caller_restricted = state.caller_restricted;
    }

    fn is_safe(&self, expr: &'hir hir::Expr<'hir>) -> bool {
        self.is_safe_inner(expr, HELPER_DEPTH)
    }

    fn is_safe_inner(&self, expr: &'hir hir::Expr<'hir>, depth: u8) -> bool {
        match &expr.peel_parens().kind {
            ExprKind::Member(base, ident) if ident.name == sym::sender => {
                is_builtin(base, sym::msg)
            }
            ExprKind::Member(base, ident) if ident.name == kw::Origin => is_builtin(base, sym::tx),
            ExprKind::Ident(_) if is_builtin(expr, sym::this) => true,
            // Address literals are safe; only `0` is accepted among numeric literals.
            ExprKind::Lit(lit) => match &lit.kind {
                LitKind::Address(_) => true,
                LitKind::Number(n) => n.is_zero(),
                _ => false,
            },
            ExprKind::Ident(reses) => reses.iter().any(|r| match r {
                Res::Item(ItemId::Variable(vid)) => self.is_safe_var(*vid),
                _ => false,
            }),
            // Peel address and numeric casts so `payable(address(uint160(0)))` is safe.
            ExprKind::Call(callee, args, _)
                if is_address_like_cast_callee(callee) || is_numeric_cast_callee(callee) =>
            {
                args.exprs().next().is_some_and(|e| self.is_safe_inner(e, depth))
            }
            ExprKind::Payable(inner) => self.is_safe_inner(inner, depth),
            ExprKind::Ternary(_, t, f) => {
                self.is_safe_inner(t, depth) && self.is_safe_inner(f, depth)
            }
            ExprKind::Call(callee, args, _)
                if depth > 0
                    && args.exprs().next().is_none()
                    && callee_no_arg_returns(self.hir, callee, |e| {
                        self.is_safe_inner(e, depth - 1)
                    }) =>
            {
                true
            }
            _ => false,
        }
    }

    /// True when `vid` is currently in `safe_vars`, or is an `immutable`/`constant`
    /// address-typed state variable.
    fn is_safe_var(&self, vid: hir::VariableId) -> bool {
        if self.safe_vars.contains(&vid) {
            return true;
        }
        let var = self.hir.variable(vid);
        var.kind.is_state() && (var.is_immutable() || var.is_constant()) && var_is_address_like(var)
    }

    /// `target = rhs`: update `safe_vars` for non-state targets.
    fn assign(&mut self, target: hir::VariableId, rhs: &'hir hir::Expr<'hir>) {
        if self.is_safe(rhs) {
            self.safe_vars.insert(target);
        } else {
            self.safe_vars.remove(&target);
        }
    }

    /// Handles single-var and tuple LHS; tuple slots align with a tuple-literal RHS.
    fn handle_assign(&mut self, lhs: &hir::Expr<'_>, rhs: &'hir hir::Expr<'hir>) {
        let lhs = lhs.peel_parens();
        if let ExprKind::Tuple(lhs_elems) = &lhs.kind {
            let rhs_elems = tuple_elems(rhs);
            for (i, lhs_elem) in lhs_elems.iter().enumerate() {
                if let Some(lhs_expr) = lhs_elem {
                    self.assign_one(lhs_expr, tuple_slot(rhs_elems, i));
                }
            }
        } else {
            self.assign_one(lhs, Some(rhs));
        }
    }

    /// `rhs == None` (unknown slot) drops the target's safe-fact.
    fn assign_one(&mut self, lhs: &hir::Expr<'_>, rhs: Option<&'hir hir::Expr<'hir>>) {
        let Some(target) = underlying_var(lhs) else { return };
        self.safe_vars.remove(&target);
        self.safe_fn_ptrs.remove(&target);
        if self.hir.variable(target).kind.is_state() {
            return;
        }
        if matches!(self.hir.variable(target).ty.kind, TypeKind::Function(_)) {
            if rhs.is_some_and(|r| self.is_fn_ptr_safe_rhs(r)) {
                self.safe_fn_ptrs.insert(target);
            }
            return;
        }
        if rhs.is_some_and(|r| self.is_safe(r)) {
            self.safe_vars.insert(target);
        }
    }

    /// True when `expr` is a function-pointer value whose destination is `this`.
    fn is_fn_ptr_safe_rhs(&self, expr: &hir::Expr<'_>) -> bool {
        match &expr.peel_parens().kind {
            ExprKind::Member(base, _) => is_address_self(base),
            ExprKind::Ident(reses) => reses.iter().any(|r| {
                matches!(r, Res::Item(ItemId::Variable(vid)) if self.safe_fn_ptrs.contains(vid))
            }),
            ExprKind::Ternary(_, t, f) => self.is_fn_ptr_safe_rhs(t) && self.is_fn_ptr_safe_rhs(f),
            _ => false,
        }
    }

    /// True when `expr` is a fn-pointer call whose destination is provably `this`.
    fn fn_ptr_call_routes_to_self(&self, expr: &'hir hir::Expr<'hir>) -> bool {
        let ExprKind::Call(callee, _, _) = &expr.kind else { return false };
        let callee_inner = callee.peel_parens();
        let is_fn_ptr = match &callee_inner.kind {
            ExprKind::Ident(reses) => reses.iter().any(|r| {
                matches!(r, Res::Item(ItemId::Variable(vid))
                    if matches!(self.hir.variable(*vid).ty.kind, TypeKind::Function(_)))
            }),
            _ => expr_is_function(self.gcx, callee_inner),
        };
        is_fn_ptr && self.is_fn_ptr_safe_rhs(callee_inner)
    }

    /// Records vars proven equal to a safe origin from `pred`. `negate = true` flips polarity.
    fn add_facts(&mut self, pred: &'hir hir::Expr<'hir>, negate: bool) {
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
                    for (a, b) in [(lhs, rhs), (rhs, lhs)] {
                        if self.is_safe(a)
                            && let Some(v) = underlying_var(b)
                            && self.is_safe_target(v)
                        {
                            self.safe_vars.insert(v);
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

    /// `lhs ∨ rhs`: a safety fact is added only if it holds under both arms.
    fn add_facts_disjunction(
        &mut self,
        lhs: &'hir hir::Expr<'hir>,
        rhs: &'hir hir::Expr<'hir>,
        negate: bool,
    ) {
        let baseline = self.safe_vars.clone();
        self.add_facts(lhs, negate);
        let lhs_added: HashSet<_> = self.safe_vars.difference(&baseline).copied().collect();
        self.safe_vars.clone_from(&baseline);
        self.add_facts(rhs, negate);
        let rhs_added: HashSet<_> = self.safe_vars.difference(&baseline).copied().collect();
        self.safe_vars = baseline;
        for v in lhs_added.intersection(&rhs_added) {
            self.safe_vars.insert(*v);
        }
    }

    /// A variable can carry a safe-fact iff it's a local/param or an `immutable`/`constant`
    fn is_safe_target(&self, v: hir::VariableId) -> bool {
        let var = self.hir.variable(v);
        !var.kind.is_state() || var.is_immutable() || var.is_constant()
    }

    /// Visits a body that may execute zero times or out-of-line (loops, try clauses).
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

impl<'hir> hir::Visit<'hir> for Analyzer<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match &stmt.kind {
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                for s in block.stmts {
                    let _ = self.visit_stmt(s);
                    if branch_always_exits(s) {
                        break;
                    }
                }
                return ControlFlow::Continue(());
            }
            StmtKind::If(cond, then, else_) => {
                let _ = self.visit_expr(cond);
                let baseline = self.snapshot();
                self.add_facts(cond, false);
                if cond_restricts_caller(self.hir, cond, true, &[], &mut self.self_aliases) {
                    self.caller_restricted = true;
                }
                let _ = self.visit_stmt(then);
                let then_exits = branch_always_exits(then);
                let after_then = self.snapshot();
                self.restore(baseline);
                self.add_facts(cond, true);
                if cond_restricts_caller(self.hir, cond, false, &[], &mut self.self_aliases) {
                    self.caller_restricted = true;
                }
                let else_exits = match else_ {
                    Some(e) => {
                        let _ = self.visit_stmt(e);
                        branch_always_exits(e)
                    }
                    None => false,
                };
                let after_else = self.snapshot();
                // When both branches exit, the joined state is never read (the caller
                // breaks on `branch_always_exits`), so intersection is a safe default.
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
                    for s in block.stmts {
                        let _ = self.visit_stmt(s);
                    }
                } else {
                    self.visit_isolated(block.stmts);
                }
                return ControlFlow::Continue(());
            }
            StmtKind::Try(t) => {
                let _ = self.visit_expr(&t.expr);
                let outer = self.snapshot();
                let mut clause_exits = Vec::new();
                for clause in t.clauses {
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
            StmtKind::DeclSingle(vid) => {
                let var = self.hir.variable(*vid);
                if var_is_address_like(var)
                    && let Some(init) = var.initializer
                {
                    self.assign(*vid, init);
                } else if matches!(var.ty.kind, TypeKind::Function(_)) {
                    if var.initializer.is_some_and(|init| self.is_fn_ptr_safe_rhs(init)) {
                        self.safe_fn_ptrs.insert(*vid);
                    } else {
                        self.safe_fn_ptrs.remove(vid);
                    }
                }
            }
            StmtKind::DeclMulti(vars, init) => {
                if let ExprKind::Tuple(rhs) = &init.peel_parens().kind {
                    for (lhs, rhs) in vars.iter().zip(rhs.iter()) {
                        let (Some(vid), Some(expr)) = (lhs, rhs) else { continue };
                        let var = self.hir.variable(*vid);
                        if var_is_address_like(var) {
                            self.assign(*vid, expr);
                        } else if matches!(var.ty.kind, TypeKind::Function(_)) {
                            if self.is_fn_ptr_safe_rhs(expr) {
                                self.safe_fn_ptrs.insert(*vid);
                            } else {
                                self.safe_fn_ptrs.remove(vid);
                            }
                        }
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
        if let ExprKind::Ternary(cond, then_e, else_e) = &expr.kind {
            let _ = self.visit_expr(cond);
            let pre_arm = self.snapshot();
            self.add_facts(cond, false);
            let _ = self.visit_expr(then_e);
            let post_then = self.snapshot();
            self.restore(pre_arm);
            self.add_facts(cond, true);
            let _ = self.visit_expr(else_e);
            let post_else = self.snapshot();
            self.restore(FlowState::intersection(&post_then, &post_else));
            return ControlFlow::Continue(());
        }
        match &expr.kind {
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                let result = self.walk_expr(expr);
                if let Some(cond) = args.exprs().next() {
                    self.add_facts(cond, false);
                    if cond_restricts_caller(self.hir, cond, true, &[], &mut self.self_aliases) {
                        self.caller_restricted = true;
                    }
                }
                return result;
            }
            ExprKind::Call(..) => {
                if !self.caller_restricted
                    && let Some(dest) = match_sink(self.gcx, self.hir, expr)
                    && !self.is_safe(dest)
                    && !self.fn_ptr_call_routes_to_self(expr)
                {
                    self.hits.push(expr.span);
                }
            }
            ExprKind::Assign(lhs, _, rhs) => self.handle_assign(lhs, rhs),
            ExprKind::Delete(target) => self.assign_one(target.peel_parens(), None),
            _ => {}
        }
        self.walk_expr(expr)
    }
}

/// Returns the destination expression when `expr` is an ETH-sending sink.
fn match_sink<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> Option<&'hir hir::Expr<'hir>> {
    let ExprKind::Call(callee, args, opts) = &expr.kind else { return None };
    if let ExprKind::Ident(reses) = &callee.peel_parens().kind
        && reses.iter().any(|r| matches!(r, Res::Builtin(Builtin::Selfdestruct)))
    {
        let dest = args.exprs().next()?;
        if is_address_self(dest) {
            return None;
        }
        return Some(dest);
    }

    if let Some(opts) = opts
        && opts.args.iter().any(|arg| arg.name.name == sym::value && !is_literal_zero(&arg.value))
    {
        let callee_inner = callee.peel_parens();
        match &callee_inner.kind {
            ExprKind::Member(recv, _) if !is_address_self(recv) => return Some(recv),
            ExprKind::Ident(reses)
                if reses.iter().any(|r| {
                    matches!(
                        r,
                        Res::Item(ItemId::Variable(vid))
                            if matches!(hir.variable(*vid).ty.kind, TypeKind::Function(_))
                    )
                }) =>
            {
                return Some(callee);
            }
            _ if expr_is_function(gcx, callee_inner) => {
                return Some(callee);
            }
            _ => {}
        }
    }

    let ExprKind::Member(recv, member) = &callee.peel_parens().kind else { return None };
    if matches!(member.name, sym::transfer | sym::send)
        && args.len() == 1
        && receiver_is_address(gcx, recv)
        && !is_address_self(recv)
    {
        let amt = args.exprs().next()?;
        if !is_literal_zero(amt) {
            return Some(recv);
        }
    }
    match_eth_library_call(gcx, hir, recv, member.name, args)
}

/// Recognises common OZ/Solady ETH-sending helpers and returns the destination expression.
fn match_eth_library_call<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    recv: &'hir hir::Expr<'hir>,
    member: Symbol,
    args: &'hir CallArgs<'hir>,
) -> Option<&'hir hir::Expr<'hir>> {
    let n = args.len();
    let using = receiver_is_address(gcx, recv);
    let recv_is_lib = matches!(&recv.peel_parens().kind, ExprKind::Ident(reses)
    if reses.iter().any(|r| matches!(
        r,
        Res::Item(ItemId::Contract(cid))
            if hir.contract(*cid).kind == ContractKind::Library
    )));
    if !using && !recv_is_lib {
        return None;
    }
    let name = member.as_str();
    let valid = match name {
        "sendValue" | "safeTransferETH" | "safeMoveETH" => (using && n == 1) || (!using && n == 2),
        "forceSafeTransferETH" => (using && matches!(n, 1 | 2)) || (!using && matches!(n, 2 | 3)),
        "trySafeTransferETH" => (using && n == 2) || (!using && n == 3),
        "functionCallWithValue" => (using && matches!(n, 2 | 3)) || (!using && matches!(n, 3 | 4)),
        "safeTransferAllETH" => (using && n == 0) || (!using && n == 1),
        "forceSafeTransferAllETH" => {
            (using && matches!(n, 0 | 1)) || (!using && matches!(n, 1 | 2))
        }
        "trySafeTransferAllETH" => (using && n == 1) || (!using && n == 2),
        _ => false,
    };

    if !valid {
        return None;
    }

    let dest = if using { recv } else { arg(args, 0, &["to", "target", "recipient"])? };
    let amount = match name {
        "safeTransferAllETH" | "forceSafeTransferAllETH" | "trySafeTransferAllETH" => None,
        "functionCallWithValue" => {
            Some(arg(args, if using { 1 } else { 2 }, &["value", "amount"])?)
        }
        _ => Some(arg(args, if using { 0 } else { 1 }, &["amount", "value"])?),
    };
    if amount.is_some_and(is_literal_zero) || is_address_self(dest) {
        return None;
    }
    Some(dest)
}

/// Looks up call-site arg `pos` (positional) or any name in `names` (named-arg form).
fn arg<'hir>(
    args: &'hir CallArgs<'hir>,
    pos: usize,
    names: &[&str],
) -> Option<&'hir hir::Expr<'hir>> {
    match args.kind {
        hir::CallArgsKind::Unnamed(exprs) => exprs.get(pos),
        hir::CallArgsKind::Named(named) => {
            named.iter().find(|a| names.iter().any(|n| a.name.as_str() == *n)).map(|a| &a.value)
        }
    }
}

/// True when a modifier reverts unless `msg.sender` equals a trusted principal.
fn modifier_restricts_caller<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    invocation: &hir::Modifier<'_>,
) -> bool {
    let ItemId::Function(fid) = invocation.id else { return false };
    let mut self_aliases = SelfAliasAnalysis::new(gcx, hir);
    modifier_function_restricts_caller(hir, fid, &mut Vec::new(), &mut self_aliases)
}

/// Resolves the `FunctionId` invoked by a modifier or base-constructor invocation.
fn invoked_function(hir: &hir::Hir<'_>, invocation: &hir::Modifier<'_>) -> Option<FunctionId> {
    match invocation.id {
        ItemId::Function(fid) => Some(fid),
        ItemId::Contract(cid) => hir.contract(cid).ctor,
        _ => None,
    }
}

fn modifier_function_restricts_caller<'hir>(
    hir: &'hir hir::Hir<'hir>,
    fid: FunctionId,
    stack: &mut Vec<FunctionId>,
    self_aliases: &mut SelfAliasAnalysis<'hir>,
) -> bool {
    if stack.contains(&fid) {
        return false;
    }
    let Some((modifier, prefix)) = modifier_prefix(hir, fid) else { return false };
    stack.push(fid);
    let restricts = prefix
        .iter()
        .any(|s| stmt_restricts_caller(hir, s, modifier.parameters, stack, self_aliases));
    stack.pop();
    restricts
}

/// Returns the modifier function and the statements preceding its unique `_;` placeholder,
/// or `None` when `fid` is not an eligible single-placeholder modifier.
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

fn stmt_restricts_caller<'hir>(
    hir: &'hir hir::Hir<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
    params: &[hir::VariableId],
    stack: &mut Vec<FunctionId>,
    self_aliases: &mut SelfAliasAnalysis<'hir>,
) -> bool {
    match &stmt.kind {
        StmtKind::Expr(e) => expr_restricts_caller(hir, e, params, stack, self_aliases),
        StmtKind::If(cond, then, else_) => {
            let then_exits = branch_always_exits(then);
            let else_exits = else_.as_ref().is_some_and(|e| branch_always_exits(e));
            let by_if_revert = match (then_exits, else_exits) {
                (true, false) => cond_restricts_caller(hir, cond, false, params, self_aliases),
                (false, true) => cond_restricts_caller(hir, cond, true, params, self_aliases),
                _ => false,
            };
            if by_if_revert {
                return true;
            }
            let then_restricts = stmt_restricts_caller(hir, then, params, stack, self_aliases);
            let else_restricts = else_
                .as_ref()
                .is_some_and(|e| stmt_restricts_caller(hir, e, params, stack, self_aliases));
            match (then_exits, else_exits) {
                (true, true) => true,
                (true, false) => else_.is_some() && else_restricts,
                (false, true) => then_restricts,
                (false, false) => then_restricts && else_.is_some() && else_restricts,
            }
        }
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => {
            b.stmts.iter().any(|s| stmt_restricts_caller(hir, s, params, stack, self_aliases))
        }
        _ => false,
    }
}

fn expr_restricts_caller<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    params: &[hir::VariableId],
    stack: &mut Vec<FunctionId>,
    self_aliases: &mut SelfAliasAnalysis<'hir>,
) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
    if is_require_or_assert(callee) {
        return args
            .exprs()
            .next()
            .is_some_and(|c| cond_restricts_caller(hir, c, true, params, self_aliases));
    }
    let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return false };
    reses.iter().any(|r| match r {
        Res::Item(ItemId::Function(fid)) => {
            if stack.contains(fid) {
                return false;
            }
            let f = hir.function(*fid);
            let Some(body) = f.body else { return false };
            // Trailing bare `return;` is a normal exit and cannot bypass an earlier guard.
            let mut stmts = body.stmts;
            while let Some((last, init)) = stmts.split_last() {
                if matches!(last.kind, StmtKind::Return(None)) {
                    stmts = init;
                } else {
                    break;
                }
            }
            if stmts.iter().any(stmt_contains_return) {
                return false;
            }
            stack.push(*fid);
            let r = stmts
                .iter()
                .any(|s| stmt_restricts_caller(hir, s, f.parameters, stack, self_aliases));
            stack.pop();
            r
        }
        _ => false,
    })
}

/// True when any reachable statement is `return`. Used to disqualify caller-restricting
/// helpers that might return without reverting.
fn stmt_contains_return(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) => true,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) | StmtKind::Loop(b, _) => {
            b.stmts.iter().any(stmt_contains_return)
        }
        StmtKind::If(_, t, e) => {
            stmt_contains_return(t) || e.as_ref().is_some_and(|s| stmt_contains_return(s))
        }
        StmtKind::Try(t) => {
            t.clauses.iter().any(|c| c.block.stmts.iter().any(stmt_contains_return))
        }
        _ => false,
    }
}

/// True when `cond` entails `msg.sender == trusted` along every accepting path.
fn cond_restricts_caller<'hir>(
    hir: &'hir hir::Hir<'hir>,
    cond: &'hir hir::Expr<'hir>,
    polarity: bool,
    params: &[hir::VariableId],
    self_aliases: &mut SelfAliasAnalysis<'hir>,
) -> bool {
    match &cond.peel_parens().kind {
        ExprKind::Binary(lhs, op, rhs) => {
            let (eq, any_op, all_op) = if polarity {
                (ast::BinOpKind::Eq, ast::BinOpKind::And, ast::BinOpKind::Or)
            } else {
                (ast::BinOpKind::Ne, ast::BinOpKind::Or, ast::BinOpKind::And)
            };
            if op.kind == any_op {
                cond_restricts_caller(hir, lhs, polarity, params, self_aliases)
                    || cond_restricts_caller(hir, rhs, polarity, params, self_aliases)
            } else if op.kind == all_op {
                cond_restricts_caller(hir, lhs, polarity, params, self_aliases)
                    && cond_restricts_caller(hir, rhs, polarity, params, self_aliases)
            } else if op.kind == eq {
                let mut pair = |a: &'hir hir::Expr<'hir>, b: &'hir hir::Expr<'hir>| {
                    is_msg_sender_like(hir, a, HELPER_DEPTH)
                        && is_trusted_principal_inner(hir, b, params, HELPER_DEPTH, self_aliases)
                };
                pair(lhs, rhs) || pair(rhs, lhs)
            } else {
                false
            }
        }
        ExprKind::Unary(op, inner) if matches!(op.kind, ast::UnOpKind::Not) => {
            cond_restricts_caller(hir, inner, !polarity, params, self_aliases)
        }
        _ => false,
    }
}

/// `msg.sender` modulo parens / casts / `payable(...)` / no-arg helpers.
fn is_msg_sender_like<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    depth: u8,
) -> bool {
    is_caller_like(hir, expr, depth, sym::msg, sym::sender)
}

/// `tx.origin` modulo parens / casts / `payable(...)` / no-arg helpers.
fn is_tx_origin_like<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    depth: u8,
) -> bool {
    is_caller_like(hir, expr, depth, sym::tx, kw::Origin)
}

/// True when `callee` is a zero-arg function whose body is `return <pred-matching>;`.
fn callee_no_arg_returns<'hir>(
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
    mut pred: impl FnMut(&'hir hir::Expr<'hir>) -> bool,
) -> bool {
    let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return false };
    reses.iter().any(|r| {
        matches!(r, Res::Item(ItemId::Function(fid)) if function_no_arg_returns(hir, *fid, &mut pred))
    })
}

/// True when `fid` is a zero-parameter function whose body is `return expr;`,
/// or `namedRet = expr;` (with an optional trailing bare `return;`).
fn function_no_arg_returns<'hir>(
    hir: &'hir hir::Hir<'hir>,
    fid: FunctionId,
    pred: &mut impl FnMut(&'hir hir::Expr<'hir>) -> bool,
) -> bool {
    let f = hir.function(fid);
    let Some(body) = f.body else { return false };
    if !f.parameters.is_empty() {
        return false;
    }
    // A trailing bare `return;` is a no-op; ignore it before matching the body shape.
    let stmts: &[_] = match body.stmts.split_last() {
        Some((last, rest)) if matches!(last.kind, StmtKind::Return(None)) => rest,
        _ => body.stmts,
    };
    if stmts.len() != 1 {
        return false;
    }
    match &stmts[0].kind {
        StmtKind::Return(Some(e)) => pred(e),
        // Named-return form: the sole named return is assigned the result.
        StmtKind::Expr(e) => match &e.peel_parens().kind {
            ExprKind::Assign(lhs, None, rhs) => {
                f.returns.len() == 1
                    && underlying_var(lhs).is_some_and(|v| v == f.returns[0])
                    && pred(rhs)
            }
            _ => false,
        },
        _ => false,
    }
}

/// Shared shape for `msg.sender` / `tx.origin` recognition.
fn is_caller_like<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    depth: u8,
    ns: Symbol,
    member: Symbol,
) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Member(base, ident) if ident.name == member => is_builtin(base, ns),
        ExprKind::Payable(inner) => is_caller_like(hir, inner, depth, ns, member),
        ExprKind::Call(callee, args, _) if is_address_like_cast_callee(callee) => {
            args.exprs().next().is_some_and(|e| is_caller_like(hir, e, depth, ns, member))
        }
        ExprKind::Call(callee, args, _) if depth > 0 && args.exprs().next().is_none() => {
            callee_no_arg_returns(hir, callee, |e| is_caller_like(hir, e, depth - 1, ns, member))
        }
        _ => false,
    }
}

/// Conservatively recognises deploy-time-fixed caller principals.
fn is_trusted_principal_inner<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    params: &[hir::VariableId],
    depth: u8,
    self_aliases: &mut SelfAliasAnalysis<'hir>,
) -> bool {
    if expr_touches_param(expr, params)
        || is_msg_sender_like(hir, expr, HELPER_DEPTH)
        || is_tx_origin_like(hir, expr, HELPER_DEPTH)
        || is_address_self(expr)
    {
        return false;
    }
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Lit(lit) => match &lit.kind {
            LitKind::Address(_) => true,
            LitKind::Number(n) => n.is_zero(),
            _ => false,
        },
        ExprKind::Call(callee, args, _) if is_address_like_cast_callee(callee) => {
            args.exprs().next().is_some_and(|inner| match &inner.peel_parens().kind {
                // Address literals trust; only the `0` numeric literal trusts.
                ExprKind::Lit(lit) => match &lit.kind {
                    LitKind::Address(_) => true,
                    LitKind::Number(n) => n.is_zero(),
                    _ => false,
                },
                _ => is_trusted_principal_inner(hir, inner, params, depth, self_aliases),
            })
        }
        ExprKind::Payable(inner) => {
            is_trusted_principal_inner(hir, inner, params, depth, self_aliases)
        }
        ExprKind::Ident(reses) => reses.iter().any(|r| match r {
            Res::Item(ItemId::Variable(vid)) => {
                let var = hir.variable(*vid);
                var.kind.is_state() && !self_aliases.state_var_aliases_self(*vid, SELF_ALIAS_DEPTH)
            }
            _ => false,
        }),
        ExprKind::Member(base, _) => {
            is_trusted_principal_inner(hir, base, params, depth, self_aliases)
        }
        ExprKind::Index(base, idx) => {
            is_trusted_principal_inner(hir, base, params, depth, self_aliases)
                && idx.is_none_or(|i| index_is_static(hir, i, params))
        }
        ExprKind::Call(callee, args, _) => {
            depth > 0
                && args.exprs().next().is_none()
                && callee_no_arg_returns(hir, callee, |e| {
                    is_trusted_principal_inner(hir, e, &[], depth - 1, self_aliases)
                })
        }
        _ => false,
    }
}

/// Memoized state-var self-alias analysis used by caller-restriction checks.
struct SelfAliasAnalysis<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    cache: HashMap<(hir::VariableId, u8), bool>,
    active: HashSet<(hir::VariableId, u8)>,
}

impl<'hir> SelfAliasAnalysis<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir hir::Hir<'hir>) -> Self {
        Self { gcx, hir, cache: HashMap::new(), active: HashSet::new() }
    }

    /// True when `vid` is a state variable that may alias `address(this)`.
    fn state_var_aliases_self(&mut self, vid: hir::VariableId, depth: u8) -> bool {
        if depth == 0 {
            return false;
        }
        let var = self.hir.variable(vid);
        if !var.kind.is_state() {
            return false;
        }

        let key = (vid, depth);
        if let Some(result) = self.cache.get(&key) {
            return *result;
        }
        if !self.active.insert(key) {
            return false;
        }

        let result = self.state_var_aliases_self_uncached(vid, depth);
        self.active.remove(&key);
        self.cache.insert(key, result);
        result
    }

    fn state_var_aliases_self_uncached(&mut self, vid: hir::VariableId, depth: u8) -> bool {
        let var = self.hir.variable(vid);
        if let Some(init) = var.initializer {
            let initializer_aliases = if var_is_address_like(var) {
                self.expr_resolves_to_self(init, depth - 1)
            } else {
                self.expr_may_contain_self_in(init, depth - 1, &HashSet::new())
            };
            if initializer_aliases {
                return true;
            }
        }
        let Some(cid) = var.contract else { return false };
        if self.contract_function_assigns_to_self(cid, vid, depth - 1) {
            return true;
        }
        let derived_contracts: Vec<_> = self
            .hir
            .contracts_enumerated()
            .filter_map(|(other_cid, other)| {
                (other_cid != cid && other.linearized_bases.contains(&cid)).then_some(other_cid)
            })
            .collect();
        derived_contracts
            .into_iter()
            .any(|other_cid| self.contract_function_assigns_to_self(other_cid, vid, depth - 1))
    }

    /// Conservative free-standing "this expression *may* embed `address(this)` somewhere".
    fn expr_may_contain_self_in(
        &mut self,
        expr: &'hir hir::Expr<'hir>,
        depth: u8,
        local_aliases: &HashSet<hir::VariableId>,
    ) -> bool {
        if self.expr_resolves_to_self(expr, depth) {
            return true;
        }
        if let Some(vid) = lhs_root_var(expr)
            && local_aliases.contains(&vid)
        {
            return true;
        }
        if depth == 0 {
            return false;
        }
        match &expr.peel_parens().kind {
            ExprKind::Payable(inner) => {
                self.expr_may_contain_self_in(inner, depth - 1, local_aliases)
            }
            ExprKind::Call(callee, args, _)
                if is_address_like_cast_callee(callee) || is_numeric_cast_callee(callee) =>
            {
                args.exprs()
                    .next()
                    .is_some_and(|e| self.expr_may_contain_self_in(e, depth - 1, local_aliases))
            }
            ExprKind::Call(_, args, _) => {
                args.exprs().any(|e| self.expr_may_contain_self_in(e, depth - 1, local_aliases))
            }
            ExprKind::Ternary(_, t, f) => {
                self.expr_may_contain_self_in(t, depth - 1, local_aliases)
                    || self.expr_may_contain_self_in(f, depth - 1, local_aliases)
            }
            ExprKind::Tuple(elems) => elems
                .iter()
                .flatten()
                .any(|e| self.expr_may_contain_self_in(e, depth - 1, local_aliases)),
            ExprKind::Array(elems) => {
                elems.iter().any(|e| self.expr_may_contain_self_in(e, depth - 1, local_aliases))
            }
            _ => false,
        }
    }

    /// True when `expr` may evaluate to `address(this)`.
    fn expr_resolves_to_self(&mut self, expr: &'hir hir::Expr<'hir>, depth: u8) -> bool {
        if is_address_self(expr) {
            return true;
        }
        if depth == 0 {
            return false;
        }
        match &expr.peel_parens().kind {
            ExprKind::Payable(inner) => self.expr_resolves_to_self(inner, depth - 1),
            ExprKind::Call(callee, args, _)
                if is_address_like_cast_callee(callee) || is_numeric_cast_callee(callee) =>
            {
                args.exprs().next().is_some_and(|e| self.expr_resolves_to_self(e, depth - 1))
            }
            ExprKind::Ident(reses) => reses.iter().any(|r| match r {
                Res::Item(ItemId::Variable(other_vid)) => {
                    self.state_var_aliases_self(*other_vid, depth)
                }
                _ => false,
            }),
            ExprKind::Member(_, _) | ExprKind::Index(_, _) => lhs_root_var(expr)
                .map(|vid| self.state_var_aliases_self(vid, depth))
                .unwrap_or(false),
            ExprKind::Call(callee, args, _) => {
                if args.exprs().count() == 0 {
                    self.callee_returns_self(callee, depth - 1)
                } else if let Some(arg) = identity_helper_arg(self.hir, callee, args) {
                    self.expr_resolves_to_self(arg, depth - 1)
                } else {
                    false
                }
            }
            ExprKind::Ternary(_, t, f) => {
                self.expr_resolves_to_self(t, depth - 1) || self.expr_resolves_to_self(f, depth - 1)
            }
            ExprKind::Assign(_, _, rhs) => self.expr_resolves_to_self(rhs, depth - 1),
            _ => false,
        }
    }

    /// True when `callee` is a zero-arg helper whose body is `return <self-resolving>;`.
    fn callee_returns_self(&mut self, callee: &'hir hir::Expr<'hir>, depth: u8) -> bool {
        if callee_no_arg_returns(self.hir, callee, |e| self.expr_resolves_to_self(e, depth)) {
            return true;
        }
        let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return false };
        let ExprKind::Ident(reses) = &base.peel_parens().kind else { return false };
        let Some(cid) = reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Contract(cid))
                if self.hir.contract(*cid).kind == ContractKind::Library =>
            {
                Some(*cid)
            }
            _ => None,
        }) else {
            return false;
        };
        let contract_ids: Vec<_> = if self.hir.contract(cid).linearization_failed() {
            vec![cid]
        } else {
            self.hir.contract(cid).linearized_bases.to_vec()
        };
        for bid in contract_ids {
            let fids: Vec<_> = self.hir.contract(bid).all_functions().collect();
            for fid in fids {
                if self.hir.function(fid).name.is_none_or(|n| n.name != member.name) {
                    continue;
                }
                if function_no_arg_returns(self.hir, fid, &mut |e| {
                    self.expr_resolves_to_self(e, depth)
                }) {
                    return true;
                }
            }
        }
        false
    }

    /// Scans every function of `cid` for an assignment that aliases `vid` to `address(this)`.
    fn contract_function_assigns_to_self(
        &mut self,
        cid: hir::ContractId,
        vid: hir::VariableId,
        depth: u8,
    ) -> bool {
        let fids: Vec<_> = self.hir.contract(cid).all_functions().collect();
        for fid in fids {
            let f = self.hir.function(fid);
            let Some(body) = f.body else { continue };
            let mut found = false;
            let mut scan = SelfAssignScan {
                hir: self.hir,
                aliases: self,
                target: vid,
                depth,
                found: &mut found,
                helper_stack: Vec::new(),
                local_self_aliases: HashSet::new(),
            };
            for inv in f.modifiers {
                if *scan.found {
                    break;
                }
                if let Some(invoked_fid) = invoked_function(scan.hir, inv) {
                    scan.scan_invoked(invoked_fid, &inv.args);
                }
            }
            if *scan.found {
                return true;
            }
            for stmt in body.stmts {
                if *scan.found {
                    break;
                }
                let _ = scan.visit_stmt(stmt);
            }
            if found {
                return true;
            }
        }
        false
    }
}

/// Parameter returned verbatim by a single-statement function body.
fn function_returns_param(hir: &hir::Hir<'_>, fid: FunctionId) -> Option<hir::VariableId> {
    let f = hir.function(fid);
    let body = f.body?;
    if body.stmts.len() != 1 || f.returns.len() != 1 {
        return None;
    }
    let StmtKind::Return(Some(ret)) = &body.stmts[0].kind else { return None };
    fn unwrap<'a>(e: &'a hir::Expr<'a>) -> &'a hir::Expr<'a> {
        let e = e.peel_parens();
        match &e.kind {
            ExprKind::Payable(inner) => unwrap(inner),
            ExprKind::Call(callee, args, _)
                if is_address_like_cast_callee(callee) || is_numeric_cast_callee(callee) =>
            {
                args.exprs().next().map(unwrap).unwrap_or(e)
            }
            _ => e,
        }
    }
    let inner = unwrap(ret);
    let ExprKind::Ident(reses) = &inner.kind else { return None };
    for r in *reses {
        let Res::Item(ItemId::Variable(vid)) = r else { continue };
        if f.parameters.iter().any(|p| p == vid) {
            return Some(*vid);
        }
    }
    None
}

/// Resolves a bare `id(addr)` or library-static `Lib.id(addr)` identity-helper call.
fn identity_helper_arg<'hir>(
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
    args: &'hir hir::CallArgs<'hir>,
) -> Option<&'hir hir::Expr<'hir>> {
    let callee = callee.peel_parens();
    let call_arity = args.exprs().count();
    let try_fid = |fid: FunctionId| -> Option<&'hir hir::Expr<'hir>> {
        let f = hir.function(fid);
        if f.parameters.len() != call_arity {
            return None;
        }
        let param = function_returns_param(hir, fid)?;
        arg_for_param(hir, f, param, args)
    };
    match &callee.kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Function(fid)) => try_fid(*fid),
            _ => None,
        }),
        ExprKind::Member(base, member) => {
            let ExprKind::Ident(reses) = &base.peel_parens().kind else { return None };
            let cid = reses.iter().find_map(|r| match r {
                Res::Item(ItemId::Contract(cid)) => Some(*cid),
                _ => None,
            })?;
            if hir.contract(cid).kind != ContractKind::Library {
                return None;
            }
            find_in_bases_or_self(hir, cid, |bid| {
                hir.contract(bid).all_functions().find_map(|fid| {
                    hir.function(fid)
                        .name
                        .is_some_and(|n| n.name == member.name)
                        .then(|| try_fid(fid))
                        .flatten()
                })
            })
        }
        _ => None,
    }
}

/// Call-site argument expression bound to `param`, supporting positional and named args.
fn arg_for_param<'hir>(
    hir: &'hir hir::Hir<'hir>,
    f: &hir::Function<'hir>,
    param: hir::VariableId,
    args: &'hir hir::CallArgs<'hir>,
) -> Option<&'hir hir::Expr<'hir>> {
    let param_idx = f.parameters.iter().position(|p| *p == param)?;
    match args.kind {
        hir::CallArgsKind::Unnamed(exprs) => exprs.get(param_idx),
        hir::CallArgsKind::Named(named) => {
            let pname = hir.variable(param).name?;
            named.iter().find(|a| a.name.name == pname.name).map(|a| &a.value)
        }
    }
}

/// `uint<N>(x)` / `int<N>(x)` cast callee, for unwrapping integer-round-trip launderings.
fn is_numeric_cast_callee(callee: &hir::Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Type(hir::Type {
            kind: TypeKind::Elementary(ElementaryType::UInt(_) | ElementaryType::Int(_)),
            ..
        })
    )
}

/// Cap helper-call recursion (covers `ctor → _init → _initInner → _initLeaf`).
const HELPER_CALL_DEPTH: u8 = 4;

/// Per-function scan state for [`SelfAliasAnalysis::contract_function_assigns_to_self`].
struct SelfAssignScan<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    aliases: &'a mut SelfAliasAnalysis<'hir>,
    target: hir::VariableId,
    depth: u8,
    found: &'a mut bool,
    helper_stack: Vec<FunctionId>,
    /// Locals path-insensitively known to *may* carry `address(this)`.
    local_self_aliases: HashSet<hir::VariableId>,
}

impl<'hir> SelfAssignScan<'_, 'hir> {
    fn expr_may_contain_self(&mut self, expr: &'hir hir::Expr<'hir>) -> bool {
        self.aliases.expr_may_contain_self_in(expr, self.depth, &self.local_self_aliases)
    }

    /// True when `lhs` (possibly inside a tuple) aliases the target.
    fn lhs_aliases_target(
        &mut self,
        lhs: &'hir hir::Expr<'hir>,
        rhs: &'hir hir::Expr<'hir>,
    ) -> bool {
        let lhs = lhs.peel_parens();
        let rhs = rhs.peel_parens();
        if let ExprKind::Tuple(lhs_elems) = &lhs.kind {
            let rhs_elems = tuple_elems(rhs);
            return lhs_elems.iter().enumerate().any(|(i, lhs_elem)| {
                lhs_elem.is_some_and(|le| {
                    tuple_slot(rhs_elems, i).is_some_and(|r| self.lhs_aliases_target(le, r))
                })
            });
        }
        if lhs_root_var(lhs) != Some(self.target) {
            return false;
        }
        let target = self.hir.variable(self.target);
        if var_is_address_like(target) {
            self.aliases.expr_resolves_to_self(rhs, self.depth)
                || lhs_root_var(rhs).is_some_and(|vid| self.local_self_aliases.contains(&vid))
        } else {
            self.expr_may_contain_self(rhs)
        }
    }

    /// Records non-state locals proven (path-insensitively) to carry `address(this)`.
    fn record_local_self_alias(&mut self, lhs: &hir::Expr<'_>, rhs: &'hir hir::Expr<'hir>) {
        let lhs = lhs.peel_parens();
        let rhs = rhs.peel_parens();
        if let ExprKind::Tuple(lhs_elems) = &lhs.kind {
            let rhs_elems = tuple_elems(rhs);
            for (i, lhs_elem) in lhs_elems.iter().enumerate() {
                if let Some(le) = lhs_elem
                    && let Some(re) = tuple_slot(rhs_elems, i)
                {
                    self.record_local_self_alias(le, re);
                }
            }
            return;
        }
        if let Some(vid) = lhs_root_var(lhs)
            && !self.hir.variable(vid).kind.is_state()
            && self.expr_may_contain_self(rhs)
        {
            self.local_self_aliases.insert(vid);
        }
    }

    /// Single internal-helper `FunctionId` for a bare-ident call; rejects overloads.
    fn helper_callee(&self, callee: &hir::Expr<'_>) -> Option<FunctionId> {
        let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return None };
        let mut fid_iter = reses.iter().filter_map(|r| match r {
            Res::Item(ItemId::Function(fid)) => Some(*fid),
            _ => None,
        });
        let fid = fid_iter.next()?;
        fid_iter.next().is_none().then_some(fid)
    }

    /// Marks each helper parameter as a self-carrier when its call-site arg may carry self.
    fn seed_helper_param_aliases(
        &mut self,
        f: &hir::Function<'hir>,
        call_args: &'hir hir::CallArgs<'hir>,
    ) {
        for &param in f.parameters {
            if let Some(arg) = arg_for_param(self.hir, f, param, call_args)
                && self.expr_may_contain_self(arg)
            {
                self.local_self_aliases.insert(param);
            }
        }
    }

    /// Walks an invoked function (modifier or base constructor) and its own modifier chain.
    fn scan_invoked(&mut self, invoked_fid: FunctionId, inv_args: &'hir hir::CallArgs<'hir>) {
        if (self.helper_stack.len() as u8) >= HELPER_CALL_DEPTH
            || self.helper_stack.contains(&invoked_fid)
        {
            return;
        }
        let invoked = self.hir.function(invoked_fid);
        let Some(inv_body) = invoked.body else { return };
        let saved = self.local_self_aliases.clone();
        self.seed_helper_param_aliases(invoked, inv_args);
        self.helper_stack.push(invoked_fid);
        for inner in invoked.modifiers {
            if *self.found {
                break;
            }
            if let Some(inner_fid) = invoked_function(self.hir, inner) {
                self.scan_invoked(inner_fid, &inner.args);
            }
        }
        for stmt in inv_body.stmts {
            if *self.found {
                break;
            }
            let _ = self.visit_stmt(stmt);
        }
        self.helper_stack.pop();
        self.local_self_aliases = saved;
    }
}

impl<'hir> hir::Visit<'hir> for SelfAssignScan<'_, 'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        if *self.found {
            return ControlFlow::Continue(());
        }
        match &stmt.kind {
            StmtKind::DeclSingle(vid) => {
                let var = self.hir.variable(*vid);
                if !var.kind.is_state()
                    && let Some(init) = var.initializer
                    && self.expr_may_contain_self(init)
                {
                    self.local_self_aliases.insert(*vid);
                }
            }
            StmtKind::DeclMulti(vars, init) => {
                if let ExprKind::Tuple(rhs) = &init.peel_parens().kind {
                    for (lhs, rhs) in vars.iter().zip(rhs.iter()) {
                        if let (Some(vid), Some(expr)) = (lhs, rhs)
                            && !self.hir.variable(*vid).kind.is_state()
                            && self.expr_may_contain_self(expr)
                        {
                            self.local_self_aliases.insert(*vid);
                        }
                    }
                }
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if *self.found {
            return ControlFlow::Continue(());
        }
        if let ExprKind::Assign(lhs, _, rhs) = &expr.peel_parens().kind {
            self.record_local_self_alias(lhs, rhs);
            if self.lhs_aliases_target(lhs, rhs) {
                *self.found = true;
                return ControlFlow::Continue(());
            }
        }
        if let ExprKind::Call(callee, call_args, _) = &expr.peel_parens().kind
            && let ExprKind::Member(recv, member) = &callee.peel_parens().kind
            && member.name.as_str() == "push"
            && lhs_root_var(recv) == Some(self.target)
            && expr_is_array_or_bytes(self.aliases.gcx, recv)
            && call_args.exprs().any(|a| self.expr_may_contain_self(a))
        {
            *self.found = true;
            return ControlFlow::Continue(());
        }
        if let ExprKind::Call(callee, call_args, _) = &expr.peel_parens().kind
            && (self.helper_stack.len() as u8) < HELPER_CALL_DEPTH
            && let Some(fid) = self.helper_callee(callee)
            && !self.helper_stack.contains(&fid)
        {
            let f = self.hir.function(fid);
            if let Some(body) = f.body {
                let saved = self.local_self_aliases.clone();
                self.seed_helper_param_aliases(f, call_args);
                self.helper_stack.push(fid);
                for stmt in body.stmts {
                    if *self.found {
                        break;
                    }
                    let _ = self.visit_stmt(stmt);
                }
                self.helper_stack.pop();
                self.local_self_aliases = saved;
            }
        }
        self.walk_expr(expr)
    }
}

/// Returns the slot expressions of a tuple literal (after peeling parens), or `None` when
/// `expr` is not a tuple. Slots themselves may be `None` (gaps in a tuple LHS).
fn tuple_elems<'hir>(expr: &'hir hir::Expr<'hir>) -> Option<&'hir [Option<&'hir hir::Expr<'hir>>]> {
    match &expr.peel_parens().kind {
        ExprKind::Tuple(elems) => Some(*elems),
        _ => None,
    }
}

/// Looks up a single slot from the result of [`tuple_elems`].
fn tuple_slot<'hir>(
    elems: Option<&'hir [Option<&'hir hir::Expr<'hir>>]>,
    i: usize,
) -> Option<&'hir hir::Expr<'hir>> {
    elems.and_then(|e| e.get(i).copied()).flatten()
}

/// Applies `f` to each contract in `cid`'s linearization, or just `cid` itself when
/// linearization failed, returning the first `Some` result.
fn find_in_bases_or_self<T>(
    hir: &hir::Hir<'_>,
    cid: hir::ContractId,
    mut f: impl FnMut(hir::ContractId) -> Option<T>,
) -> Option<T> {
    let contract = hir.contract(cid);
    if contract.linearization_failed() {
        f(cid)
    } else {
        contract.linearized_bases.iter().find_map(|&bid| f(bid))
    }
}

/// Variable at the root of an LHS expression.
fn lhs_root_var(lhs: &hir::Expr<'_>) -> Option<hir::VariableId> {
    match &lhs.peel_parens().kind {
        ExprKind::Ident(_) => underlying_var(lhs),
        ExprKind::Member(base, _) => lhs_root_var(base),
        ExprKind::Index(base, _) => lhs_root_var(base),
        ExprKind::Call(callee, args, _) if is_address_like_cast_callee(callee) => {
            args.exprs().next().and_then(lhs_root_var)
        }
        ExprKind::Payable(inner) => lhs_root_var(inner),
        _ => None,
    }
}

/// True when every sub-expression of `expr` is independent of the call's parameters.
fn index_is_static<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    params: &[hir::VariableId],
) -> bool {
    fn walk<'hir>(
        hir: &'hir hir::Hir<'hir>,
        e: &'hir hir::Expr<'hir>,
        params: &[hir::VariableId],
    ) -> bool {
        if expr_touches_param(e, params)
            || is_msg_sender_like(hir, e, HELPER_DEPTH)
            || is_tx_origin_like(hir, e, HELPER_DEPTH)
        {
            return false;
        }
        match &e.peel_parens().kind {
            ExprKind::Lit(_) => true,
            ExprKind::Ident(reses) => reses.iter().all(|r| match r {
                Res::Item(ItemId::Variable(vid)) => hir.variable(*vid).kind.is_state(),
                Res::Builtin(_) => false,
                _ => true,
            }),
            ExprKind::Payable(i) | ExprKind::Unary(_, i) => walk(hir, i, params),
            ExprKind::Binary(l, _, r) => walk(hir, l, params) && walk(hir, r, params),
            ExprKind::Member(base, _) => walk(hir, base, params),
            ExprKind::Index(base, idx) => {
                walk(hir, base, params) && idx.is_none_or(|i| walk(hir, i, params))
            }
            ExprKind::Ternary(c, t, f) => {
                walk(hir, c, params) && walk(hir, t, params) && walk(hir, f, params)
            }
            ExprKind::Call(callee, args, _) => {
                let callee_ok = match &callee.peel_parens().kind {
                    ExprKind::Type(_) => true,
                    ExprKind::Ident(reses) => {
                        reses.iter().any(|r| matches!(r, Res::Item(ItemId::Contract(_))))
                    }
                    _ => false,
                };
                callee_ok && args.exprs().all(|a| walk(hir, a, params))
            }
            _ => false,
        }
    }
    walk(hir, expr, params)
}

/// True when any sub-expression references one of the supplied `VariableId`s.
fn expr_touches_param(expr: &hir::Expr<'_>, params: &[hir::VariableId]) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses
            .iter()
            .any(|r| matches!(r, Res::Item(ItemId::Variable(vid)) if params.contains(vid))),
        ExprKind::Binary(l, _, r) | ExprKind::Assign(l, _, r) => {
            expr_touches_param(l, params) || expr_touches_param(r, params)
        }
        ExprKind::Unary(_, i)
        | ExprKind::Payable(i)
        | ExprKind::Delete(i)
        | ExprKind::Member(i, _) => expr_touches_param(i, params),
        ExprKind::Index(b, idx) => {
            expr_touches_param(b, params) || idx.is_some_and(|i| expr_touches_param(i, params))
        }
        ExprKind::Ternary(c, t, f) => {
            expr_touches_param(c, params)
                || expr_touches_param(t, params)
                || expr_touches_param(f, params)
        }
        ExprKind::Call(callee, args, _) => {
            expr_touches_param(callee, params)
                || args.exprs().any(|a| expr_touches_param(a, params))
        }
        _ => false,
    }
}

/// Hoists `require(modParam == msg.sender)`-style guards from the modifier prefix.
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
        .filter_map(|&mp| {
            let arg = arg_for_param(hir, modifier, mp, &invocation.args)?;
            Some((mp, underlying_var(arg)?))
        })
        .collect();
    if arg_map.is_empty() {
        return;
    }
    let mut assigned_params: HashSet<hir::VariableId> = HashSet::new();
    let mut collector = AssignedParamCollector { hir, out: &mut assigned_params };
    for stmt in &prefix {
        let _ = collector.visit_stmt(stmt);
    }
    let mut a = Analyzer::new(gcx, hir);
    for stmt in &prefix {
        let _ = a.visit_stmt(stmt);
    }
    for (mp, caller) in arg_map {
        if !assigned_params.contains(&mp) && a.safe_vars.contains(&mp) && a.is_safe_target(caller) {
            out_safe.insert(caller);
        }
    }
}

/// Statements preceding the unique `_;` in a modifier body, in execution order.
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
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b)
                if count_placeholders(b.stmts) >= 1 =>
            {
                out.extend(stmts[..i].iter());
                return collect_stmts_before_placeholder(b.stmts, out);
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

/// Collects every `VariableId` that appears as the target of an assignment or `delete`.
struct AssignedParamCollector<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    out: &'a mut HashSet<hir::VariableId>,
}

impl AssignedParamCollector<'_, '_> {
    fn add_lhs(&mut self, lhs: &hir::Expr<'_>) {
        match &lhs.peel_parens().kind {
            ExprKind::Tuple(elems) => {
                for e in elems.iter().flatten() {
                    self.add_lhs(e);
                }
            }
            _ => {
                if let Some(vid) = underlying_var(lhs) {
                    self.out.insert(vid);
                }
            }
        }
    }
}

impl<'hir> hir::Visit<'hir> for AssignedParamCollector<'_, 'hir> {
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

/// Strips the trailing `if (...) break;` that lowers `do { ... } while (cond);`.
fn do_while_user_stmts<'a, 'hir>(stmts: &'a [hir::Stmt<'hir>]) -> &'a [hir::Stmt<'hir>] {
    if let Some((last, rest)) = stmts.split_last()
        && let StmtKind::If(_, t, e) = &last.kind
        && (is_break_stmt(t) || e.as_ref().is_some_and(|e| is_break_stmt(e)))
    {
        return rest;
    }
    stmts
}

fn is_break_stmt(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Break => true,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => {
            b.stmts.len() == 1 && is_break_stmt(&b.stmts[0])
        }
        _ => false,
    }
}

fn stmt_has_break_or_continue(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Break | StmtKind::Continue => true,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => {
            b.stmts.iter().any(stmt_has_break_or_continue)
        }
        StmtKind::If(_, t, e) => {
            stmt_has_break_or_continue(t)
                || e.as_ref().is_some_and(|s| stmt_has_break_or_continue(s))
        }
        StmtKind::Try(t) => {
            t.clauses.iter().any(|c| c.block.stmts.iter().any(stmt_has_break_or_continue))
        }
        StmtKind::Loop(..) => false,
        _ => false,
    }
}

fn count_placeholders(stmts: &[hir::Stmt<'_>]) -> usize {
    stmts.iter().map(count_placeholders_in_stmt).sum()
}

fn count_placeholders_in_stmt(stmt: &hir::Stmt<'_>) -> usize {
    match &stmt.kind {
        StmtKind::Placeholder => 1,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) | StmtKind::Loop(b, _) => {
            count_placeholders(b.stmts)
        }
        StmtKind::If(_, t, e) => {
            count_placeholders_in_stmt(t) + e.as_ref().map_or(0, |s| count_placeholders_in_stmt(s))
        }
        StmtKind::Try(t) => t.clauses.iter().map(|c| count_placeholders(c.block.stmts)).sum(),
        _ => 0,
    }
}

/// Resolves a `VariableId` for bare idents and address-like wrappers.
fn underlying_var(expr: &hir::Expr<'_>) -> Option<hir::VariableId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Variable(vid)) => Some(*vid),
            _ => None,
        }),
        ExprKind::Call(callee, args, _) if is_address_like_cast_callee(callee) => {
            args.exprs().next().and_then(underlying_var)
        }
        ExprKind::Payable(inner) => underlying_var(inner),
        _ => None,
    }
}

/// `address` / `address payable` or a contract/interface type.
const fn var_is_address_like(var: &hir::Variable<'_>) -> bool {
    matches!(
        var.ty.kind,
        TypeKind::Elementary(ElementaryType::Address(_)) | TypeKind::Custom(ItemId::Contract(_))
    )
}

/// True when `expr`'s static type is `address` / `address payable`.
fn receiver_is_address<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    expr_ty(gcx, expr).is_some_and(ty_is_address)
}

/// Callee of a single-argument cast that yields an address-shaped value.
fn is_address_like_cast_callee(callee: &hir::Expr<'_>) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Type(hir::Type {
            kind: TypeKind::Elementary(ElementaryType::Address(_)),
            ..
        }) => true,
        ExprKind::Ident(reses) => reses.iter().any(|r| matches!(r, Res::Item(ItemId::Contract(_)))),
        _ => false,
    }
}

fn expr_ty<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> Option<Ty<'hir>> {
    gcx.type_of_expr(expr.peel_parens().id)
}

fn expr_is_function<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    expr_ty(gcx, expr).is_some_and(|ty| matches!(ty.peel_refs().kind, TyKind::Fn(_)))
}

fn expr_is_array_or_bytes<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    expr_ty(gcx, expr).is_some_and(|ty| {
        matches!(
            ty.peel_refs().kind,
            TyKind::Array(..) | TyKind::DynArray(_) | TyKind::Elementary(ElementaryType::Bytes)
        )
    })
}

fn ty_is_address(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::Address(_)))
}

fn is_require_or_assert(callee: &hir::Expr<'_>) -> bool {
    let ExprKind::Ident(reses) = &callee.kind else { return false };
    reses.iter().any(
        |r| matches!(r, Res::Builtin(b) if b.name() == sym::require || b.name() == sym::assert),
    )
}

/// `address(this)`, `payable(this)`, `IFoo(this)`, `IFoo(address(this))`, or bare `this`.
fn is_address_self(expr: &hir::Expr<'_>) -> bool {
    let expr = expr.peel_parens();
    if is_builtin(expr, sym::this) {
        return true;
    }
    if let ExprKind::Payable(inner) = &expr.kind {
        return is_address_self(inner);
    }
    matches!(&expr.kind, ExprKind::Call(callee, args, _) if is_address_like_cast_callee(callee)
        && args.exprs().next().is_some_and(is_address_self))
}

fn is_builtin(expr: &hir::Expr<'_>, name: Symbol) -> bool {
    let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return false };
    reses.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == name))
}

fn is_literal_zero(expr: &hir::Expr<'_>) -> bool {
    if let ExprKind::Lit(lit) = &expr.peel_parens().kind
        && let LitKind::Number(n) = &lit.kind
    {
        return n.is_zero();
    }
    false
}

/// `return`, custom-error `revert`, `revert(...)`, or `assert(false)` / `require(false, ...)`.
fn branch_always_exits(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Revert(_) => true,
        StmtKind::Expr(expr) => is_exit_call(expr),
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => b.stmts.iter().any(branch_always_exits),
        StmtKind::If(_, t, Some(e)) => branch_always_exits(t) && branch_always_exits(e),
        StmtKind::Try(t) => {
            !t.clauses.is_empty()
                && t.clauses.iter().all(|c| c.block.stmts.iter().any(branch_always_exits))
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
        && reses.iter().any(|r| matches!(r, Res::Builtin(Builtin::Selfdestruct)))
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
