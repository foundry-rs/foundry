use super::ArbitrarySendEth;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            arg_for_param, branch_always_exits, builtins, count_placeholders, expr_is_address,
            expr_ty, function_ids,
            helper_cache::{DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT, HelperAnalysisCache},
            is_address_like_cast, is_address_self, is_builtin, is_contract_cast, is_literal_zero,
            is_msg_sender, is_require_or_assert, referenced_item, stmts_before_placeholder,
            tuple_elems, underlying_var, unique,
        },
    },
};
use solar::{
    ast::{BinOpKind, LitKind, StateMutability, UnOpKind},
    interface::{Span, data_structures::Never, kw, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, CallArgs, CallArgsKind, ContractId, ContractKind, ElementaryType, Expr, ExprKind,
            FunctionId, FunctionKind, Hir, ItemId, LoopSource, Modifier, Res, Stmt, StmtKind,
            TypeKind, Variable, VariableId, Visit,
        },
        ty::TyKind,
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    ARBITRARY_SEND_ETH,
    Severity::High,
    "arbitrary-send-eth",
    "ETH is sent to a user-controlled destination; restrict the destination or the caller"
);

/// Recursion budget for `_msgSender()`-style helper chains.
const HELPER_DEPTH: u8 = 3;

/// Recursion budget for self-alias chains.
const SELF_ALIAS_DEPTH: u8 = 8;

/// Cap on inlined helper calls (covers `ctor → _init → _initInner → _initLeaf`).
const HELPER_CALL_DEPTH: usize = 4;

impl<'hir> LateLintPass<'hir> for ArbitrarySendEth {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if matches!(func.state_mutability, StateMutability::Pure | StateMutability::View)
            || func.is_constructor()
            || func.contract.is_some_and(|cid| hir.contract(cid).kind == ContractKind::Library)
        {
            return;
        }
        let Some(body) = func.body else { return };

        // Modifier arguments are evaluated by the caller before any modifier guard runs.
        let mut args = Analyzer::new(gcx, hir);
        for arg in func.modifiers.iter().flat_map(|m| m.args.exprs()) {
            let _ = args.visit_expr(arg);
        }
        for span in args.hits {
            ctx.emit(&ARBITRARY_SEND_ETH, span);
        }

        let mut a = Analyzer::new(gcx, hir);
        for m in func.modifiers {
            a.hoist_modifier_facts(m);
        }
        a.visit_stmts(body.stmts);
        if !a.hits.is_empty() && !func.modifiers.iter().any(|m| a.guards.modifier_restricts(m)) {
            for span in a.hits {
                ctx.emit(&ARBITRARY_SEND_ETH, span);
            }
        }
    }
}

/// Path-sensitive facts.
#[derive(Clone, Default)]
struct State {
    /// Locals (and function pointers) proven to denote a safe destination on this path.
    safe_vars: HashSet<VariableId>,
    /// True once a caller-restricting guard has fired on this path.
    caller_restricted: bool,
}

impl State {
    fn meet(&self, other: &Self) -> Self {
        Self {
            safe_vars: self.safe_vars.intersection(&other.safe_vars).copied().collect(),
            caller_restricted: self.caller_restricted && other.caller_restricted,
        }
    }
}

struct Analyzer<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    guards: CallerGuards<'hir>,
    state: State,
    /// States at `break`/`continue` of each enclosing loop, innermost last.
    loop_exits: Vec<Vec<State>>,
    /// Every variable written on any path.
    written: HashSet<VariableId>,
    hits: Vec<Span>,
}

impl<'hir> Analyzer<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir Hir<'hir>) -> Self {
        Self {
            gcx,
            hir,
            guards: CallerGuards::new(gcx, hir),
            state: State::default(),
            loop_exits: Vec::new(),
            written: HashSet::new(),
            hits: Vec::new(),
        }
    }

    /// Hoists `require(param == msg.sender)`-style guards from the prefix of modifier `m` onto
    /// the caller's argument variables.
    fn hoist_modifier_facts(&mut self, m: &'hir Modifier<'hir>) {
        let ItemId::Function(fid) = m.id else { return };
        let Some(prefix) = modifier_prefix(self.hir, fid) else { return };
        let modifier = self.hir.function(fid);
        let mut a = Self::new(self.gcx, self.hir);
        for stmt in prefix {
            a.stmt(stmt);
        }
        for &param in modifier.parameters {
            if a.state.safe_vars.contains(&param)
                && !a.written.contains(&param)
                && let Some(caller) =
                    arg_for_param(self.hir, modifier, param, &m.args).and_then(underlying_var)
                && self.is_safe_target(caller)
            {
                self.state.safe_vars.insert(caller);
            }
        }
    }

    /// True when `expr` denotes a destination that is fixed at deploy time or is the caller
    /// itself: `msg.sender`, `tx.origin`, `address(this)`, address or zero literals,
    /// `immutable`/`constant` state, tracked locals and `this.f` function pointers.
    fn is_safe(&self, expr: &'hir Expr<'hir>) -> bool {
        self.is_safe_inner(expr, HELPER_DEPTH)
    }

    fn is_safe_inner(&self, expr: &'hir Expr<'hir>, depth: u8) -> bool {
        let expr = peel_casts(expr);
        match &expr.kind {
            ExprKind::Member(base, ident) => {
                is_msg_sender(expr)
                    || (ident.name == kw::Origin && is_builtin(base, sym::tx))
                    || is_address_self(base)
            }
            ExprKind::Lit(_) => is_trusted_literal(expr),
            ExprKind::Ident(reses) => {
                is_builtin(expr, sym::this)
                    || reses.iter().filter_map(Res::as_variable).any(|v| self.is_safe_var(v))
            }
            ExprKind::Ternary(_, t, f) => {
                self.is_safe_inner(t, depth) && self.is_safe_inner(f, depth)
            }
            ExprKind::Call(callee, args, _) => {
                depth > 0
                    && args.exprs().next().is_none()
                    && callee_no_arg_returns(self.hir, callee, |e| self.is_safe_inner(e, depth - 1))
            }
            _ => false,
        }
    }

    fn is_safe_var(&self, v: VariableId) -> bool {
        let var = self.hir.variable(v);
        self.state.safe_vars.contains(&v)
            || (var.kind.is_state() && (var.is_immutable() || var.is_constant()))
    }

    /// Only locals and `immutable`/`constant` state can carry a safe-fact: mutable storage may be
    /// rewritten between the check and the sink.
    fn is_safe_target(&self, v: VariableId) -> bool {
        let var = self.hir.variable(v);
        !var.kind.is_state() || var.is_immutable() || var.is_constant()
    }

    /// `target = rhs`; `rhs == None` is an unknown value.
    fn assign_var(&mut self, target: VariableId, rhs: Option<&'hir Expr<'hir>>) {
        self.written.insert(target);
        self.state.safe_vars.remove(&target);
        if !self.hir.variable(target).kind.is_state() && rhs.is_some_and(|r| self.is_safe(r)) {
            self.state.safe_vars.insert(target);
        }
    }

    /// Handles single and tuple LHS; tuple slots align with a tuple-literal RHS.
    fn assign_lhs(&mut self, lhs: &'hir Expr<'hir>, rhs: Option<&'hir Expr<'hir>>) {
        if let Some(elems) = tuple_elems(lhs) {
            let rhs = rhs.and_then(tuple_elems);
            for (i, lhs) in elems.iter().enumerate() {
                if let Some(lhs) = lhs {
                    self.assign_lhs(lhs, rhs.and_then(|r| r.get(i).copied().flatten()));
                }
            }
        } else if let Some(v) = underlying_var(lhs) {
            self.assign_var(v, rhs);
        }
    }

    /// Records variables proven equal to a safe destination by `pred` (`!pred` when `negate`).
    fn add_facts(&mut self, pred: &'hir Expr<'hir>, negate: bool) {
        match &pred.peel_parens().kind {
            ExprKind::Binary(lhs, op, rhs) => {
                let (eq, and, or) = if negate {
                    (BinOpKind::Ne, BinOpKind::Or, BinOpKind::And)
                } else {
                    (BinOpKind::Eq, BinOpKind::And, BinOpKind::Or)
                };
                if op.kind == and {
                    self.add_facts(lhs, negate);
                    self.add_facts(rhs, negate);
                } else if op.kind == or {
                    // Only facts established by both disjuncts hold.
                    let before = self.state.clone();
                    self.add_facts(lhs, negate);
                    let after_lhs = std::mem::replace(&mut self.state, before);
                    self.add_facts(rhs, negate);
                    self.state = after_lhs.meet(&self.state);
                } else if op.kind == eq {
                    for (a, b) in [(lhs, rhs), (rhs, lhs)] {
                        if self.is_safe(a)
                            && let Some(v) = underlying_var(b)
                            && self.is_safe_target(v)
                        {
                            self.state.safe_vars.insert(v);
                        }
                    }
                }
            }
            ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
                self.add_facts(inner, !negate);
            }
            _ => {}
        }
    }

    /// Applies a guard known to hold (`holds`) or fail on the current path.
    fn note_guard(&mut self, cond: &'hir Expr<'hir>, holds: bool) {
        self.add_facts(cond, !holds);
        if self.guards.cond_restricts(cond, holds) {
            self.state.caller_restricted = true;
        }
    }

    /// Visits `stmts` up to the first that cannot fall through; returns whether the end is
    /// reachable.
    fn visit_stmts(&mut self, stmts: &'hir [Stmt<'hir>]) -> bool {
        stmts.iter().all(|s| self.stmt(s))
    }

    /// Visits `stmt`, returning whether control can fall through it.
    fn stmt(&mut self, stmt: &'hir Stmt<'hir>) -> bool {
        match &stmt.kind {
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => return self.visit_stmts(b.stmts),
            StmtKind::Break | StmtKind::Continue => {
                let state = self.state.clone();
                if let Some(exits) = self.loop_exits.last_mut() {
                    exits.push(state);
                }
                return false;
            }
            StmtKind::If(cond, then, else_) => {
                let _ = self.visit_expr(cond);
                let before = self.state.clone();
                self.note_guard(cond, true);
                let then_falls = self.stmt(then);
                let after_then = std::mem::replace(&mut self.state, before);
                self.note_guard(cond, false);
                let else_falls = else_.is_none_or(|e| self.stmt(e));
                match (then_falls, else_falls) {
                    (true, true) => self.state = after_then.meet(&self.state),
                    (true, false) => self.state = after_then,
                    _ => {}
                }
                return then_falls || else_falls;
            }
            StmtKind::Loop(block, source) => {
                // Only facts holding on every exit survive. `for`/`while` bodies may not run at
                // all; `do-while` bodies run at least once.
                let baseline = (!matches!(source, LoopSource::DoWhile)).then(|| self.state.clone());
                self.loop_exits.push(baseline.into_iter().collect());
                if self.visit_stmts(block.stmts) {
                    let state = self.state.clone();
                    self.loop_exits.last_mut().expect("pushed above").push(state);
                }
                let exits = self.loop_exits.pop().expect("pushed above");
                let falls = !exits.is_empty();
                if let Some(joined) = exits.into_iter().reduce(|a, b| a.meet(&b)) {
                    self.state = joined;
                }
                return falls;
            }
            StmtKind::Try(t) => {
                let _ = self.visit_expr(&t.expr);
                let outer = self.state.clone();
                let mut joined = None::<State>;
                for clause in t.clauses {
                    self.state = outer.clone();
                    if self.visit_stmts(clause.block.stmts) {
                        joined = Some(
                            joined.map_or_else(|| self.state.clone(), |j| j.meet(&self.state)),
                        );
                    }
                }
                let falls = joined.is_some();
                self.state = joined.unwrap_or(outer);
                return falls;
            }
            StmtKind::DeclSingle(vid) => {
                if let Some(init) = self.hir.variable(*vid).initializer {
                    self.assign_var(*vid, Some(init));
                }
            }
            StmtKind::DeclMulti(vars, init) => {
                for (vid, rhs) in vars.iter().zip(tuple_elems(init).into_iter().flatten()) {
                    if let (Some(vid), Some(rhs)) = (vid, rhs) {
                        self.assign_var(*vid, Some(rhs));
                    }
                }
            }
            _ => {}
        }
        let _ = self.walk_stmt(stmt);
        !branch_always_exits(stmt)
    }
}

impl<'hir> Visit<'hir> for Analyzer<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Never> {
        self.stmt(stmt);
        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Never> {
        match &expr.kind {
            // `rhs` may not execute: its facts and writes survive only if they also hold without
            // it, while `lhs` facts flow into `rhs`.
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let _ = self.visit_expr(lhs);
                let skipped = self.state.clone();
                self.add_facts(lhs, op.kind == BinOpKind::Or);
                let _ = self.visit_expr(rhs);
                self.state = skipped.meet(&self.state);
            }
            ExprKind::Ternary(cond, t, f) => {
                let _ = self.visit_expr(cond);
                let before = self.state.clone();
                self.add_facts(cond, false);
                let _ = self.visit_expr(t);
                let after_t = std::mem::replace(&mut self.state, before);
                self.add_facts(cond, true);
                let _ = self.visit_expr(f);
                self.state = after_t.meet(&self.state);
            }
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                // Sinks inside the predicate run before the guard takes effect.
                let _ = self.walk_expr(expr);
                if let Some(cond) = args.exprs().next() {
                    self.note_guard(cond, true);
                }
            }
            ExprKind::Call(..) => {
                if !self.state.caller_restricted
                    && let Some(dest) = match_sink(self.gcx, self.hir, expr)
                    && !self.is_safe(dest)
                {
                    self.hits.push(expr.span);
                }
                let _ = self.walk_expr(expr);
            }
            ExprKind::Assign(lhs, _, rhs) => {
                self.assign_lhs(lhs, Some(rhs));
                let _ = self.walk_expr(expr);
            }
            ExprKind::Delete(target) => {
                self.assign_lhs(target, None);
                let _ = self.walk_expr(expr);
            }
            _ => {
                let _ = self.walk_expr(expr);
            }
        }
        ControlFlow::Continue(())
    }
}

/// Destination of an ETH-sending call: `selfdestruct(x)`, `x.{call,send,transfer}`,
/// `f{value: v}()`, `IFoo(x).f{value: v}()` and common OpenZeppelin/Solady helpers. Sends to
/// `address(this)` or of a literal-zero amount are not sinks.
fn match_sink<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
) -> Option<&'hir Expr<'hir>> {
    let ExprKind::Call(callee, args, opts) = &expr.kind else { return None };
    let callee = callee.peel_parens();
    if builtins(callee).any(|b| b == Builtin::Selfdestruct) {
        return args.exprs().next().filter(|dest| !is_address_self(dest));
    }
    if opts.is_some_and(|o| {
        o.args.iter().any(|a| a.name.name == sym::value && !is_literal_zero(&a.value))
    }) {
        return match &callee.kind {
            ExprKind::Member(recv, _) => (!is_address_self(recv)).then_some(recv),
            _ => expr_is_function(gcx, callee).then_some(callee),
        };
    }
    let ExprKind::Member(recv, member) = &callee.kind else { return None };
    if matches!(member.name, sym::transfer | sym::send)
        && args.len() == 1
        && expr_is_address(gcx, recv)
    {
        return (!is_address_self(recv) && !is_literal_zero(args.exprs().next()?)).then_some(recv);
    }
    match_eth_library_call(gcx, hir, recv, member.name.as_str(), args)
}

/// Destination of an OpenZeppelin `Address` / Solady `SafeTransferLib` ETH helper, called either
/// statically (`Lib.f(to, ...)`) or via `using ... for address` (`to.f(...)`).
fn match_eth_library_call<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    recv: &'hir Expr<'hir>,
    name: &str,
    args: &'hir CallArgs<'hir>,
) -> Option<&'hir Expr<'hir>> {
    // Amount position and accepted arities, in the static form.
    let (amount, arities): (Option<usize>, &[usize]) = match name {
        "sendValue" | "safeTransferETH" | "safeMoveETH" => (Some(1), &[2]),
        "forceSafeTransferETH" => (Some(1), &[2, 3]),
        "trySafeTransferETH" => (Some(1), &[3]),
        "functionCallWithValue" => (Some(2), &[3, 4]),
        "safeTransferAllETH" => (None, &[1]),
        "forceSafeTransferAllETH" => (None, &[1, 2]),
        "trySafeTransferAllETH" => (None, &[2]),
        _ => return None,
    };
    let using = expr_is_address(gcx, recv);
    let is_lib = matches!(referenced_item(recv), Some(ItemId::Contract(cid))
        if hir.contract(cid).kind == ContractKind::Library);
    if (!using && !is_lib) || !arities.contains(&(args.len() + usize::from(using))) {
        return None;
    }
    let dest = if using { recv } else { arg(args, 0, &["to", "target", "recipient"])? };
    if let Some(i) = amount
        && is_literal_zero(arg(args, i - usize::from(using), &["amount", "value"])?)
    {
        return None;
    }
    (!is_address_self(dest)).then_some(dest)
}

/// Call-site argument at position `pos`, or bound to any of `names` in the named form.
fn arg<'hir>(args: &'hir CallArgs<'hir>, pos: usize, names: &[&str]) -> Option<&'hir Expr<'hir>> {
    match args.kind {
        CallArgsKind::Unnamed(exprs) => exprs.get(pos),
        CallArgsKind::Named(named) => {
            named.iter().find(|a| names.contains(&a.name.as_str())).map(|a| &a.value)
        }
    }
}

/// Statements before the unique `_;` of modifier `fid`, when it is reached unconditionally.
fn modifier_prefix<'hir>(hir: &'hir Hir<'hir>, fid: FunctionId) -> Option<Vec<&'hir Stmt<'hir>>> {
    let modifier = hir.function(fid);
    let body = modifier.body.filter(|_| matches!(modifier.kind, FunctionKind::Modifier))?;
    if count_placeholders(body.stmts) != 1 {
        return None;
    }
    let mut prefix = Vec::new();
    stmts_before_placeholder(body.stmts, &mut prefix)?;
    Some(prefix)
}

/// Recognises guards that restrict `msg.sender` to a deploy-time-fixed principal, backed by a
/// memoised analysis of which state variables may alias `address(this)`.
struct CallerGuards<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    alias_cache: HelperAnalysisCache<(VariableId, u8), bool>,
    /// Functions currently being inlined, to stop recursion.
    stack: Vec<FunctionId>,
}

impl<'hir> CallerGuards<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir Hir<'hir>) -> Self {
        Self {
            gcx,
            hir,
            alias_cache: HelperAnalysisCache::new(DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT),
            stack: Vec::new(),
        }
    }

    /// True when modifier `m` reverts unless `msg.sender` is a trusted principal.
    fn modifier_restricts(&mut self, m: &Modifier<'_>) -> bool {
        let ItemId::Function(fid) = m.id else { return false };
        modifier_prefix(self.hir, fid)
            .is_some_and(|prefix| prefix.into_iter().any(|s| self.stmt_restricts(s)))
    }

    fn stmt_restricts(&mut self, stmt: &'hir Stmt<'hir>) -> bool {
        match &stmt.kind {
            StmtKind::Expr(e) => self.expr_restricts(e),
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => {
                b.stmts.iter().any(|s| self.stmt_restricts(s))
            }
            StmtKind::If(cond, then, else_) => {
                let then_exits = branch_always_exits(then);
                let else_exits = else_.is_some_and(|e| branch_always_exits(e));
                // `if (!guard) revert;` restricts by itself; otherwise every non-exiting branch
                // must restrict.
                (then_exits != else_exits && self.cond_restricts(cond, else_exits))
                    || ((then_exits || self.stmt_restricts(then))
                        && (else_exits || else_.is_some_and(|e| self.stmt_restricts(e))))
            }
            _ => false,
        }
    }

    /// `require(guard)` / `assert(guard)`, or a call to an internal helper whose body restricts
    /// the caller and cannot `return` early.
    fn expr_restricts(&mut self, expr: &'hir Expr<'hir>) -> bool {
        let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
        if is_require_or_assert(callee) {
            return args.exprs().next().is_some_and(|c| self.cond_restricts(c, true));
        }
        function_ids(callee).any(|fid| {
            if self.stack.contains(&fid) {
                return false;
            }
            let Some(body) = self.hir.function(fid).body else { return false };
            // A trailing bare `return;` is a normal exit and cannot bypass an earlier guard.
            let mut stmts = body.stmts;
            while let [rest @ .., last] = stmts
                && matches!(last.kind, StmtKind::Return(None))
            {
                stmts = rest;
            }
            if stmts.iter().any(stmt_contains_return) {
                return false;
            }
            self.stack.push(fid);
            let restricts = stmts.iter().any(|s| self.stmt_restricts(s));
            self.stack.pop();
            restricts
        })
    }

    /// True when `cond` (holding iff `holds`) entails `msg.sender == <trusted>` on every path.
    fn cond_restricts(&mut self, cond: &'hir Expr<'hir>, holds: bool) -> bool {
        match &cond.peel_parens().kind {
            ExprKind::Binary(lhs, op, rhs) => {
                let (eq, any, all) = if holds {
                    (BinOpKind::Eq, BinOpKind::And, BinOpKind::Or)
                } else {
                    (BinOpKind::Ne, BinOpKind::Or, BinOpKind::And)
                };
                if op.kind == any {
                    self.cond_restricts(lhs, holds) || self.cond_restricts(rhs, holds)
                } else if op.kind == all {
                    self.cond_restricts(lhs, holds) && self.cond_restricts(rhs, holds)
                } else if op.kind == eq {
                    [(lhs, rhs), (rhs, lhs)].into_iter().any(|(a, b)| {
                        is_msg_sender_like(self.hir, a, HELPER_DEPTH)
                            && self.is_trusted_principal(b, HELPER_DEPTH)
                    })
                } else {
                    false
                }
            }
            ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
                self.cond_restricts(inner, !holds)
            }
            _ => false,
        }
    }

    /// Conservatively recognises deploy-time-fixed caller principals: address/zero literals and
    /// state (or statically indexed state) that cannot alias `address(this)`, possibly behind a
    /// no-arg getter. Parameters, locals, `msg.sender`, `tx.origin` and `this` are rejected.
    fn is_trusted_principal(&mut self, expr: &'hir Expr<'hir>, depth: u8) -> bool {
        let expr = peel_casts(expr);
        match &expr.kind {
            ExprKind::Lit(_) => is_trusted_literal(expr),
            ExprKind::Ident(reses) => reses.iter().filter_map(Res::as_variable).any(|v| {
                self.hir.variable(v).kind.is_state()
                    && !self.state_var_aliases_self(v, SELF_ALIAS_DEPTH)
            }),
            ExprKind::Member(base, _) => self.is_trusted_principal(base, depth),
            ExprKind::Index(base, idx) => {
                self.is_trusted_principal(base, depth)
                    && idx.is_none_or(|i| index_is_static(self.hir, i))
            }
            ExprKind::Call(callee, args, _) => {
                depth > 0
                    && args.exprs().next().is_none()
                    && callee_no_arg_returns(self.hir, callee, |e| {
                        self.is_trusted_principal(e, depth - 1)
                    })
            }
            _ => false,
        }
    }

    /// True when state variable `v` may hold `address(this)`: through its initializer or an
    /// assignment in any function of its contract or a derived contract.
    fn state_var_aliases_self(&mut self, v: VariableId, depth: u8) -> bool {
        let var = self.hir.variable(v);
        if depth == 0 || !var.kind.is_state() {
            return false;
        }
        let key = (v, depth);
        if let Some(cached) = self.alias_cache.get(&key) {
            return *cached;
        }
        if self.alias_cache.is_in_progress(&key) {
            return false;
        }
        self.alias_cache.start(key);
        let hir = self.hir;
        let aliases = var
            .initializer
            .is_some_and(|init| self.rhs_carries_self(var, init, depth - 1, &HashSet::new()))
            || var.contract.is_some_and(|cid| {
                hir.contracts_enumerated().any(|(c, contract)| {
                    (c == cid || contract.linearized_bases.contains(&cid))
                        && self.contract_assigns_self(c, v, depth - 1)
                })
            });
        self.alias_cache.finish(key, aliases);
        aliases
    }

    /// Whether assigning `rhs` to `target` may plant `address(this)` in it: an address-typed
    /// target must receive `address(this)` itself, an aggregate may embed it anywhere.
    fn rhs_carries_self(
        &mut self,
        target: &Variable<'_>,
        rhs: &'hir Expr<'hir>,
        depth: u8,
        locals: &HashSet<VariableId>,
    ) -> bool {
        if var_is_address_like(target) {
            self.expr_resolves_to_self(rhs, depth)
                || lhs_root_var(rhs).is_some_and(|v| locals.contains(&v))
        } else {
            self.expr_may_contain_self(rhs, depth, locals)
        }
    }

    /// True when `expr` may embed `address(this)` (or a local carrying it) anywhere.
    fn expr_may_contain_self(
        &mut self,
        expr: &'hir Expr<'hir>,
        depth: u8,
        locals: &HashSet<VariableId>,
    ) -> bool {
        if self.expr_resolves_to_self(expr, depth)
            || lhs_root_var(expr).is_some_and(|v| locals.contains(&v))
        {
            return true;
        }
        if depth == 0 {
            return false;
        }
        let children: Vec<&'hir Expr<'hir>> = match &peel_casts(expr).kind {
            ExprKind::Call(_, args, _) => args.exprs().collect(),
            ExprKind::Ternary(_, t, f) => vec![t, f],
            ExprKind::Tuple(elems) => elems.iter().copied().flatten().collect(),
            ExprKind::Array(elems) => elems.iter().collect(),
            _ => Vec::new(),
        };
        children.into_iter().any(|e| self.expr_may_contain_self(e, depth - 1, locals))
    }

    /// True when `expr` may evaluate to `address(this)`.
    fn expr_resolves_to_self(&mut self, expr: &'hir Expr<'hir>, depth: u8) -> bool {
        let expr = peel_casts(expr);
        if is_address_self(expr) {
            return true;
        }
        if depth == 0 {
            return false;
        }
        match &expr.kind {
            ExprKind::Ident(_) | ExprKind::Member(..) | ExprKind::Index(..) => {
                lhs_root_var(expr).is_some_and(|v| self.state_var_aliases_self(v, depth))
            }
            ExprKind::Call(callee, args, _) if args.exprs().next().is_none() => {
                let hir = self.hir;
                callee_fids(hir, callee).into_iter().any(|fid| {
                    function_no_arg_returns(hir, fid, &mut |e| {
                        self.expr_resolves_to_self(e, depth - 1)
                    })
                })
            }
            ExprKind::Call(callee, args, _) => identity_helper_arg(self.hir, callee, args)
                .is_some_and(|a| self.expr_resolves_to_self(a, depth - 1)),
            ExprKind::Ternary(_, t, f) => {
                self.expr_resolves_to_self(t, depth - 1) || self.expr_resolves_to_self(f, depth - 1)
            }
            ExprKind::Assign(_, _, rhs) => self.expr_resolves_to_self(rhs, depth - 1),
            _ => false,
        }
    }

    /// Scans every function of `cid` for an assignment that may plant `address(this)` in `v`.
    fn contract_assigns_self(&mut self, cid: ContractId, v: VariableId, depth: u8) -> bool {
        let hir = self.hir;
        hir.contract(cid).all_functions().any(|fid| {
            let mut scan = SelfAssignScan {
                guards: &mut *self,
                target: v,
                depth,
                found: false,
                stack: Vec::new(),
                locals: HashSet::new(),
            };
            scan.scan_function(fid, None);
            scan.found
        })
    }
}

/// Scans one function, its modifiers / base constructors and inlined internal helpers for an
/// assignment that may plant `address(this)` into `target`.
struct SelfAssignScan<'a, 'hir> {
    guards: &'a mut CallerGuards<'hir>,
    target: VariableId,
    depth: u8,
    found: bool,
    stack: Vec<FunctionId>,
    /// Locals that may (path-insensitively) carry `address(this)`.
    locals: HashSet<VariableId>,
}

impl<'hir> SelfAssignScan<'_, 'hir> {
    fn may_contain_self(&mut self, expr: &'hir Expr<'hir>) -> bool {
        self.guards.expr_may_contain_self(expr, self.depth, &self.locals)
    }

    fn note_local(&mut self, v: VariableId, rhs: &'hir Expr<'hir>) {
        if !self.guards.hir.variable(v).kind.is_state() && self.may_contain_self(rhs) {
            self.locals.insert(v);
        }
    }

    fn assign(&mut self, lhs: &'hir Expr<'hir>, rhs: &'hir Expr<'hir>) {
        if let Some(elems) = tuple_elems(lhs) {
            let rhs = tuple_elems(rhs);
            for (i, lhs) in elems.iter().enumerate() {
                if let Some(lhs) = lhs
                    && let Some(rhs) = rhs.and_then(|r| r.get(i).copied().flatten())
                {
                    self.assign(lhs, rhs);
                }
            }
        } else if let Some(v) = lhs_root_var(lhs) {
            if v == self.target {
                let var = self.guards.hir.variable(v);
                self.found |= self.guards.rhs_carries_self(var, rhs, self.depth, &self.locals);
            } else {
                self.note_local(v, rhs);
            }
        }
    }

    /// Scans `fid`, seeding its parameters from `args` when given.
    fn scan_function(&mut self, fid: FunctionId, args: Option<&'hir CallArgs<'hir>>) {
        if self.found || self.stack.len() >= HELPER_CALL_DEPTH || self.stack.contains(&fid) {
            return;
        }
        let hir = self.guards.hir;
        let f = hir.function(fid);
        let Some(body) = f.body else { return };
        let saved = self.locals.clone();
        for &param in f.parameters {
            if let Some(arg) = args.and_then(|args| arg_for_param(hir, f, param, args))
                && self.may_contain_self(arg)
            {
                self.locals.insert(param);
            }
        }
        self.stack.push(fid);
        for m in f.modifiers {
            if let Some(invoked) = invoked_function(hir, m) {
                self.scan_function(invoked, Some(&m.args));
            }
        }
        for stmt in body.stmts {
            let _ = self.visit_stmt(stmt);
        }
        self.stack.pop();
        self.locals = saved;
    }
}

impl<'hir> Visit<'hir> for SelfAssignScan<'_, 'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.guards.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Never> {
        if self.found {
            return ControlFlow::Continue(());
        }
        match &stmt.kind {
            StmtKind::DeclSingle(vid) => {
                if let Some(init) = self.guards.hir.variable(*vid).initializer {
                    self.note_local(*vid, init);
                }
            }
            StmtKind::DeclMulti(vars, init) => {
                for (vid, rhs) in vars.iter().zip(tuple_elems(init).into_iter().flatten()) {
                    if let (Some(vid), Some(rhs)) = (vid, rhs) {
                        self.note_local(*vid, rhs);
                    }
                }
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Never> {
        if self.found {
            return ControlFlow::Continue(());
        }
        match &expr.peel_parens().kind {
            ExprKind::Assign(lhs, _, rhs) => self.assign(lhs, rhs),
            ExprKind::Call(callee, args, _) => match &callee.peel_parens().kind {
                // `target.push(<self>)` on an array / bytes state variable.
                ExprKind::Member(recv, member) => {
                    if member.name.as_str() == "push"
                        && lhs_root_var(recv) == Some(self.target)
                        && expr_is_array_or_bytes(self.guards.gcx, recv)
                        && args.exprs().any(|a| self.may_contain_self(a))
                    {
                        self.found = true;
                    }
                }
                _ => {
                    if let Some(fid) = unique(function_ids(callee)) {
                        self.scan_function(fid, Some(args));
                    }
                }
            },
            _ => {}
        }
        self.walk_expr(expr)
    }
}

/// The function invoked by a modifier or base-constructor invocation.
fn invoked_function(hir: &Hir<'_>, m: &Modifier<'_>) -> Option<FunctionId> {
    match m.id {
        ItemId::Function(fid) => Some(fid),
        ItemId::Contract(cid) => hir.contract(cid).ctor,
        _ => None,
    }
}

/// Functions a callee may name: every overload of a bare identifier, or a library-static `Lib.f`.
fn callee_fids(hir: &Hir<'_>, callee: &Expr<'_>) -> Vec<FunctionId> {
    match &callee.peel_parens().kind {
        ExprKind::Member(base, member) => match referenced_item(base) {
            Some(ItemId::Contract(cid)) if hir.contract(cid).kind == ContractKind::Library => hir
                .contract(cid)
                .functions()
                .filter(|f| hir.function(*f).name.is_some_and(|n| n.name == member.name))
                .collect(),
            _ => Vec::new(),
        },
        _ => function_ids(callee).collect(),
    }
}

/// True when `callee` names a zero-parameter function whose body returns an expression matching
/// `pred`.
fn callee_no_arg_returns<'hir>(
    hir: &'hir Hir<'hir>,
    callee: &'hir Expr<'hir>,
    mut pred: impl FnMut(&'hir Expr<'hir>) -> bool,
) -> bool {
    callee_fids(hir, callee).into_iter().any(|fid| function_no_arg_returns(hir, fid, &mut pred))
}

/// True when `fid` takes no parameters and its body is `return e;` or `namedRet = e;` (optionally
/// followed by a bare `return;`) with `pred(e)`.
fn function_no_arg_returns<'hir>(
    hir: &'hir Hir<'hir>,
    fid: FunctionId,
    pred: &mut impl FnMut(&'hir Expr<'hir>) -> bool,
) -> bool {
    let f = hir.function(fid);
    let Some(body) = f.body else { return false };
    let stmts = match body.stmts {
        [rest @ .., last] if matches!(last.kind, StmtKind::Return(None)) => rest,
        stmts => stmts,
    };
    let [stmt] = stmts else { return false };
    f.parameters.is_empty()
        && match &stmt.kind {
            StmtKind::Return(Some(e)) => pred(e),
            StmtKind::Expr(e) => matches!(&e.peel_parens().kind, ExprKind::Assign(lhs, None, rhs)
                if f.returns.len() == 1 && underlying_var(lhs) == Some(f.returns[0]) && pred(rhs)),
            _ => false,
        }
}

/// Argument returned verbatim (modulo casts) by an identity helper call `id(x)` / `Lib.id(x)`.
fn identity_helper_arg<'hir>(
    hir: &'hir Hir<'hir>,
    callee: &'hir Expr<'hir>,
    args: &'hir CallArgs<'hir>,
) -> Option<&'hir Expr<'hir>> {
    callee_fids(hir, callee).into_iter().find_map(|fid| {
        let f = hir.function(fid);
        let [stmt] = f.body?.stmts else { return None };
        let StmtKind::Return(Some(ret)) = &stmt.kind else { return None };
        let param = underlying_var(peel_casts(ret))?;
        (f.parameters.len() == args.len() && f.returns.len() == 1 && f.parameters.contains(&param))
            .then(|| arg_for_param(hir, f, param, args))
            .flatten()
    })
}

/// Variable at the root of an lvalue, through member / index accesses and address casts.
fn lhs_root_var(lhs: &Expr<'_>) -> Option<VariableId> {
    match &lhs.peel_parens().kind {
        ExprKind::Member(base, _) | ExprKind::Index(base, _) | ExprKind::Payable(base) => {
            lhs_root_var(base)
        }
        ExprKind::Call(callee, args, _) if is_address_like_cast(callee) => {
            args.exprs().next().and_then(lhs_root_var)
        }
        _ => underlying_var(lhs),
    }
}

/// True when an index expression only depends on literals and state: no locals, parameters,
/// builtins (`msg.sender`) or non-cast calls.
fn index_is_static(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    expr.visit(&mut |e| match &e.kind {
        ExprKind::Lit(_)
        | ExprKind::Type(_)
        | ExprKind::Payable(_)
        | ExprKind::Unary(..)
        | ExprKind::Binary(..)
        | ExprKind::Member(..)
        | ExprKind::Index(..)
        | ExprKind::Ternary(..)
        | ExprKind::Tuple([Some(_)]) => ControlFlow::Continue(()),
        ExprKind::Ident(reses)
            if reses.iter().all(|r| match r {
                Res::Item(ItemId::Variable(v)) => hir.variable(*v).kind.is_state(),
                Res::Builtin(_) => false,
                _ => true,
            }) =>
        {
            ControlFlow::Continue(())
        }
        ExprKind::Call(callee, ..)
            if matches!(callee.peel_parens().kind, ExprKind::Type(_))
                || is_contract_cast(callee) =>
        {
            ControlFlow::Continue(())
        }
        _ => ControlFlow::Break(()),
    })
    .is_continue()
}

/// True when any statement of a helper body is a `return` (bare or valued).
fn stmt_contains_return(stmt: &Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) => true,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) | StmtKind::Loop(b, _) => {
            b.stmts.iter().any(stmt_contains_return)
        }
        StmtKind::If(_, t, e) => {
            stmt_contains_return(t) || e.is_some_and(|e| stmt_contains_return(e))
        }
        StmtKind::Try(t) => {
            t.clauses.iter().any(|c| c.block.stmts.iter().any(stmt_contains_return))
        }
        _ => false,
    }
}

/// `msg.sender` modulo parens, casts, `payable(..)` and no-arg helpers such as `_msgSender()`.
fn is_msg_sender_like<'hir>(hir: &'hir Hir<'hir>, expr: &'hir Expr<'hir>, depth: u8) -> bool {
    let expr = peel_casts(expr);
    is_msg_sender(expr)
        || matches!(&expr.kind, ExprKind::Call(callee, args, _)
            if depth > 0
                && args.exprs().next().is_none()
                && callee_no_arg_returns(hir, callee, |e| is_msg_sender_like(hir, e, depth - 1)))
}

/// Looks through parens, `payable(..)`, address-like casts and integer casts.
fn peel_casts<'a>(expr: &'a Expr<'a>) -> &'a Expr<'a> {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Payable(inner) => peel_casts(inner),
        ExprKind::Call(callee, args, _)
            if is_address_like_cast(callee) || is_numeric_cast(callee) =>
        {
            args.exprs().next().map_or(expr, peel_casts)
        }
        _ => expr,
    }
}

/// `uint<N>(..)` / `int<N>(..)` cast head.
fn is_numeric_cast(callee: &Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Type(hir::Type {
            kind: TypeKind::Elementary(ElementaryType::UInt(_) | ElementaryType::Int(_)),
            ..
        })
    )
}

/// An address literal or the integer literal `0`.
fn is_trusted_literal(expr: &Expr<'_>) -> bool {
    matches!(&expr.kind, ExprKind::Lit(lit) if matches!(lit.kind, LitKind::Address(_)))
        || is_literal_zero(expr)
}

/// `address` / `address payable` or a contract / interface type.
const fn var_is_address_like(var: &Variable<'_>) -> bool {
    matches!(
        var.ty.kind,
        TypeKind::Elementary(ElementaryType::Address(_)) | TypeKind::Custom(ItemId::Contract(_))
    )
}

fn expr_is_function<'hir>(gcx: Gcx<'hir>, expr: &'hir Expr<'hir>) -> bool {
    expr_ty(gcx, expr).is_some_and(|ty| matches!(ty.peel_refs().kind, TyKind::Fn(_)))
}

fn expr_is_array_or_bytes<'hir>(gcx: Gcx<'hir>, expr: &'hir Expr<'hir>) -> bool {
    expr_ty(gcx, expr).is_some_and(|ty| {
        matches!(
            ty.peel_refs().kind,
            TyKind::Array(..) | TyKind::DynArray(_) | TyKind::Elementary(ElementaryType::Bytes)
        )
    })
}
