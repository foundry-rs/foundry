use super::ArbitrarySendErc20;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            arg_for_param, branch_always_exits, expr_is_address, function_ids,
            is_address_like_cast, is_address_self, is_address_type, is_elementary, is_msg_sender,
            is_require_or_assert, loop_update, modifier_prefix, receiver_contract_id,
            state_lhs_vars, tuple_elems, underlying_var,
        },
    },
};
use solar::{
    ast::{BinOpKind, StateMutability, UnOpKind, Visibility},
    interface::{Span, Symbol, data_structures::Never},
    sema::{
        Gcx,
        hir::{
            self, CallArgs, CallArgsKind, ContractId, ContractKind, Expr, ExprKind, FunctionId,
            FunctionKind, Hir, ItemId, LoopSource, Modifier, Res, Stmt, StmtKind, TypeKind,
            VariableId, Visit,
        },
    },
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    hash::Hash,
    ops::ControlFlow,
    rc::Rc,
};

declare_forge_lint!(
    ARBITRARY_SEND_ERC20,
    Severity::High,
    "arbitrary-send-erc20",
    "`transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`"
);

declare_forge_lint!(
    ARBITRARY_SEND_ERC20_PERMIT,
    Severity::High,
    "arbitrary-send-erc20-permit",
    "`transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens"
);

/// Recursion budget for `_msgSender()`-style helper chains.
const HELPER_DEPTH: u8 = 3;

impl<'gcx> LateLintPass<'gcx> for ArbitrarySendErc20 {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        func: &'gcx hir::Function<'gcx>,
    ) {
        // Library functions forward `from` from their caller; the call site is flagged instead.
        if matches!(func.state_mutability, StateMutability::Pure | StateMutability::View)
            || func.is_constructor()
            || func.contract.is_some_and(|cid| gcx.hir.contract(cid).kind == ContractKind::Library)
        {
            return;
        }
        let Some(body) = func.body else { return };
        // A modifier prefix that always exits makes the body unreachable.
        if func.modifiers.iter().any(|m| {
            m.id.as_function()
                .and_then(|fid| modifier_prefix(&gcx.hir, fid))
                .is_some_and(|p| p.iter().any(|s| branch_always_exits(s)))
        }) {
            return;
        }
        let mut a = Analyzer::new(gcx, has_solady_safe_transfer_lib(&gcx.hir));
        if let Some(cid) = func.contract {
            a.seed_immutable_facts(cid);
        }
        a.seed_callsite_facts(func);
        for m in func.modifiers {
            a.hoist_modifier_facts(m);
        }
        a.visit_stmts(body.stmts);
        for (span, lint) in a.hits {
            ctx.emit(lint, span);
        }
    }
}

/// Identifier correlating permit and sink token receivers: `token` or `cfg.token`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum TokenKey {
    Var(VariableId),
    Field(VariableId, Symbol),
}

impl TokenKey {
    fn touches(self, v: VariableId) -> bool {
        match self {
            Self::Var(x) | Self::Field(x, _) => x == v,
        }
    }
}

/// An EIP-2612 permit with `spender == address(this)` seen earlier on the current path.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PermitRecord {
    token: TokenKey,
    owner: VariableId,
}

/// Outstanding EIP-3156 repayment licensed by a prior `onFlashLoan` call.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PendingRepayment {
    receiver: VariableId,
    token: VariableId,
    amount: VariableId,
    fee: VariableId,
}

/// An ERC20 `transferFrom`-shaped sink.
struct Sink<'gcx> {
    from: &'gcx Expr<'gcx>,
    to: &'gcx Expr<'gcx>,
    amount: &'gcx Expr<'gcx>,
    token: Option<TokenKey>,
}

/// Facts about an assignment's RHS, captured before any write.
#[derive(Clone, Copy, Default)]
struct Rhs {
    safe: bool,
    is_self: bool,
    alias: Option<VariableId>,
    sum: Option<(VariableId, VariableId)>,
}

/// Path-sensitive facts.
#[derive(Clone, Default)]
struct State {
    /// Locals and `immutable`/`constant` state proven equal to `msg.sender` or `address(this)`.
    /// Mutable storage may be rewritten between the check and the sink.
    safe_vars: HashSet<VariableId>,
    /// Subset of `safe_vars` proven equal to `address(this)`; recognises permit spenders.
    self_vars: HashSet<VariableId>,
    /// Permits seen on this path, keyed by canonical token / owner.
    permits: HashSet<PermitRecord>,
    /// Pending flash-loan repayments; each `onFlashLoan` call licenses one consumption.
    repayments: HashMap<PendingRepayment, u32>,
    /// `x = y` records `x -> canonical(y)`.
    aliases: HashMap<VariableId, VariableId>,
    /// `x = a + b` records `x -> (a, b)`, matched against flash-repayment sums.
    sum_of: HashMap<VariableId, (VariableId, VariableId)>,
}

impl State {
    fn meet(&self, other: &Self) -> Self {
        Self {
            safe_vars: self.safe_vars.intersection(&other.safe_vars).copied().collect(),
            self_vars: self.self_vars.intersection(&other.self_vars).copied().collect(),
            permits: self.permits.intersection(&other.permits).copied().collect(),
            repayments: self
                .repayments
                .iter()
                .filter_map(|(k, a)| other.repayments.get(k).map(|b| (*k, *a.min(b))))
                .collect(),
            aliases: common_entries(&self.aliases, &other.aliases),
            sum_of: common_entries(&self.sum_of, &other.sum_of),
        }
    }
}

fn common_entries<K: Eq + Hash + Copy, V: PartialEq + Copy>(
    a: &HashMap<K, V>,
    b: &HashMap<K, V>,
) -> HashMap<K, V> {
    a.iter().filter(|(k, v)| b.get(k) == Some(v)).map(|(k, v)| (*k, *v)).collect()
}

struct Analyzer<'gcx> {
    gcx: Gcx<'gcx>,
    /// Gates the `using ... for address` sink form on a Solady-shaped library being present.
    has_solady_lib: bool,
    state: State,
    /// States at `break`/`continue` of each enclosing loop, innermost last.
    loop_exits: Vec<Vec<State>>,
    /// Every variable written on any path.
    written: HashSet<VariableId>,
    hits: Vec<(Span, &'static SolLint)>,
}

impl<'gcx> Analyzer<'gcx> {
    fn new(gcx: Gcx<'gcx>, has_solady_lib: bool) -> Self {
        Self {
            gcx,
            has_solady_lib,
            state: State::default(),
            loop_exits: Vec::new(),
            written: HashSet::new(),
            hits: Vec::new(),
        }
    }

    /// Seeds facts about `immutable`/`constant` state of `cid` from declaration initializers and
    /// the constructor body.
    fn seed_immutable_facts(&mut self, cid: ContractId) {
        for v in self.gcx.hir.contract(cid).variables() {
            let var = self.gcx.hir.variable(v);
            if (var.is_immutable() || var.is_constant())
                && let Some(init) = var.initializer
            {
                if self.is_safe(init) {
                    self.state.safe_vars.insert(v);
                }
                if self.is_self_expr(init) {
                    self.state.self_vars.insert(v);
                }
            }
        }
        if let Some(ctor) = self.gcx.hir.contract(cid).ctor
            && let Some(body) = self.gcx.hir.function(ctor).body
        {
            let mut a = Self::new(self.gcx, self.has_solady_lib);
            a.visit_stmts(body.stmts);
            let is_state = |v: &&VariableId| self.gcx.hir.variable(**v).kind.is_state();
            self.state.safe_vars.extend(a.state.safe_vars.iter().filter(is_state));
            self.state.self_vars.extend(a.state.self_vars.iter().filter(is_state));
        }
    }

    /// Seeds parameters of an internal function or modifier that every invocation site in the
    /// compilation unit passes a safe argument for.
    fn seed_callsite_facts(&mut self, func: &'gcx hir::Function<'gcx>) {
        if !is_internal_only(func) {
            return;
        }
        let index = callsite_index(&self.gcx.hir);
        let Some((fid, _)) =
            self.gcx.hir.functions_enumerated().find(|(_, f)| std::ptr::eq(*f, func))
        else {
            return;
        };
        let Some(Some(facts)) = index.get(&fid) else { return };
        for (&param, &(safe, is_self)) in func.parameters.iter().zip(facts) {
            if safe {
                self.state.safe_vars.insert(param);
            }
            if is_self {
                self.state.self_vars.insert(param);
            }
        }
    }

    /// Hoists `require(param == msg.sender | address(this))` guards from the prefix of modifier
    /// `m` onto the caller's argument variables.
    fn hoist_modifier_facts(&mut self, m: &'gcx Modifier<'gcx>) {
        let Some(fid) = m.id.as_function() else { return };
        let Some(prefix) = modifier_prefix(&self.gcx.hir, fid) else { return };
        let modifier = self.gcx.hir.function(fid);
        let mut a = Self::new(self.gcx, self.has_solady_lib);
        for stmt in prefix {
            a.stmt(stmt);
        }
        for &param in modifier.parameters {
            // A fact about a rewritten parameter says nothing about the caller's variable.
            if !a.written.contains(&param)
                && let Some(caller) =
                    arg_for_param(&self.gcx.hir, modifier, param, &m.args).and_then(underlying_var)
                && self.is_safe_target(caller)
            {
                if a.state.safe_vars.contains(&param) {
                    self.state.safe_vars.insert(caller);
                }
                if a.state.self_vars.contains(&param) {
                    self.state.self_vars.insert(caller);
                }
            }
        }
    }

    /// `msg.sender`, `address(this)` or a tracked-safe variable.
    fn is_safe(&self, expr: &Expr<'_>) -> bool {
        origin_matches(&self.gcx.hir, expr, HELPER_DEPTH, &self.state.safe_vars, |e| {
            is_msg_sender(e) || is_address_self(e)
        })
    }

    /// `address(this)` or a tracked self alias.
    fn is_self_expr(&self, expr: &Expr<'_>) -> bool {
        origin_matches(&self.gcx.hir, expr, HELPER_DEPTH, &self.state.self_vars, is_address_self)
    }

    fn is_safe_target(&self, v: VariableId) -> bool {
        let var = self.gcx.hir.variable(v);
        !var.kind.is_state() || var.is_immutable() || var.is_constant()
    }

    /// Follows the alias chain to its root; bounded to guard against cycles.
    fn canonical(&self, v: VariableId) -> VariableId {
        let mut cur = v;
        for _ in 0..8 {
            match self.state.aliases.get(&cur) {
                Some(next) if *next != cur => cur = *next,
                _ => break,
            }
        }
        cur
    }

    fn canonical_key(&self, key: TokenKey) -> TokenKey {
        match key {
            TokenKey::Var(v) => TokenKey::Var(self.canonical(v)),
            TokenKey::Field(v, name) => TokenKey::Field(self.canonical(v), name),
        }
    }

    /// Drops every fact about `v`.
    fn invalidate(&mut self, v: VariableId) {
        let s = &mut self.state;
        s.safe_vars.remove(&v);
        s.self_vars.remove(&v);
        s.aliases.retain(|k, dst| *k != v && *dst != v);
        s.sum_of.retain(|k, (a, b)| *k != v && *a != v && *b != v);
        s.permits.retain(|p| !p.token.touches(v) && p.owner != v);
        s.repayments.retain(|r, _| ![r.receiver, r.token, r.amount, r.fee].contains(&v));
    }

    fn eval_rhs(&self, rhs: Option<&Expr<'_>>) -> Rhs {
        let Some(rhs) = rhs else { return Rhs::default() };
        Rhs {
            safe: self.is_safe(rhs),
            is_self: self.is_self_expr(rhs),
            alias: underlying_var(rhs).map(|v| self.canonical(v)),
            sum: sum_operands(rhs),
        }
    }

    fn assign_var(&mut self, target: VariableId, rhs: Rhs) {
        self.written.insert(target);
        self.invalidate(target);
        if !self.is_safe_target(target) {
            return;
        }
        if rhs.safe {
            self.state.safe_vars.insert(target);
        }
        if rhs.is_self {
            self.state.self_vars.insert(target);
        }
        if let Some(alias) = rhs.alias
            && alias != target
        {
            self.state.aliases.insert(target, alias);
        }
        if let Some(sum) = rhs.sum {
            self.state.sum_of.insert(target, sum);
        }
    }

    /// Handles single and tuple LHS; `rhs == None` is an unknown value (`delete`).
    fn assign_lhs(&mut self, lhs: &Expr<'_>, rhs: Option<&Expr<'_>>) {
        // Writing `cfg.token` drops permits keyed on that field.
        if let ExprKind::Member(base, ident) = &lhs.peel_parens().kind
            && let Some(base) = underlying_var(base)
        {
            let key = TokenKey::Field(self.canonical(base), ident.name);
            self.state.permits.retain(|p| p.token != key);
        }
        if let Some(elems) = tuple_elems(lhs) {
            let rhs = rhs.and_then(tuple_elems);
            // Evaluate every slot before writing any, so `(x, y) = (y, x)` stays consistent.
            let slots: Vec<_> = elems
                .iter()
                .enumerate()
                .map(|(i, l)| (*l, self.eval_rhs(rhs.and_then(|r| r.get(i).copied().flatten()))))
                .collect();
            for (lhs, rhs) in slots {
                if let Some(v) = lhs.and_then(underlying_var) {
                    self.assign_var(v, rhs);
                }
            }
        } else if let Some(v) = underlying_var(lhs) {
            let rhs = self.eval_rhs(rhs);
            self.assign_var(v, rhs);
        }
    }

    /// Records variables proven safe by `pred` (`!pred` when `negate`).
    fn add_facts(&mut self, pred: &Expr<'_>, negate: bool) {
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
                        if let Some(v) = underlying_var(b)
                            && self.is_safe_target(v)
                        {
                            if self.is_safe(a) {
                                self.state.safe_vars.insert(v);
                            }
                            if self.is_self_expr(a) {
                                self.state.self_vars.insert(v);
                            }
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

    /// EIP-2612 `token.permit(owner, <self>, ...)` or the OpenZeppelin-style wrapper
    /// `Lib.safePermit(token, owner, <self>, ...)`.
    fn match_permit_call(&self, expr: &Expr<'gcx>) -> Option<PermitRecord> {
        let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
        let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
        let (token, owner, spender) = match ident.name.as_str() {
            "permit" => {
                let a = canonical_args(
                    args,
                    &[&["owner"], &["spender"], &["value"], &["deadline"], &["v"], &["r"], &["s"]],
                )?;
                (*recv, a[0], a[1])
            }
            "safePermit"
                if receiver_contract_id(self.gcx, recv).is_some_and(|cid| {
                    self.gcx.hir.contract(cid).kind == ContractKind::Library
                }) =>
            {
                let a = canonical_args(
                    args,
                    &[
                        &["token"],
                        &["owner"],
                        &["spender"],
                        &["value"],
                        &["deadline"],
                        &["v"],
                        &["r"],
                        &["s"],
                    ],
                )?;
                (a[0], a[1], a[2])
            }
            _ => return None,
        };
        if !self.is_self_expr(spender) {
            return None;
        }
        Some(PermitRecord {
            token: self.canonical_key(token_key(token)?),
            owner: self.canonical(underlying_var(owner)?),
        })
    }

    fn permit_covers(&self, sink: &Sink<'_>) -> bool {
        let (Some(token), Some(owner)) = (sink.token, underlying_var(sink.from)) else {
            return false;
        };
        self.state.permits.contains(&PermitRecord {
            token: self.canonical_key(token),
            owner: self.canonical(owner),
        })
    }

    /// `expr` is `amount + fee` (either order), or a local bound to that sum.
    fn amount_matches(&self, expr: &Expr<'_>, amount: VariableId, fee: VariableId) -> bool {
        let sum = sum_operands(expr)
            .or_else(|| underlying_var(expr).and_then(|v| self.state.sum_of.get(&v).copied()));
        matches!(sum, Some(pair) if pair == (amount, fee) || pair == (fee, amount))
    }

    /// Consumes one pending repayment matched by a sink pulling `amount + fee` from the flash-loan
    /// receiver back to `address(this)`.
    fn consume_repayment(&mut self, sink: &Sink<'_>) -> bool {
        let (Some(from), Some(TokenKey::Var(token))) = (underlying_var(sink.from), sink.token)
        else {
            return false;
        };
        if !self.is_self_expr(sink.to) {
            return false;
        }
        let Some(rep) = self.state.repayments.keys().copied().find(|r| {
            r.receiver == from
                && r.token == token
                && self.amount_matches(sink.amount, r.amount, r.fee)
        }) else {
            return false;
        };
        match self.state.repayments.get_mut(&rep) {
            Some(count) if *count > 1 => *count -= 1,
            _ => {
                self.state.repayments.remove(&rep);
            }
        }
        true
    }

    /// Visits `stmts` up to the first that cannot fall through; returns whether the end is
    /// reachable.
    fn visit_stmts(&mut self, stmts: &'gcx [Stmt<'gcx>]) -> bool {
        stmts.iter().all(|s| self.stmt(s))
    }

    /// Visits `stmt`, returning whether control can fall through it.
    fn stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> bool {
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
                self.add_facts(cond, false);
                let then_falls = self.stmt(then);
                let after_then = std::mem::replace(&mut self.state, before);
                self.add_facts(cond, true);
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
                if self.visit_stmts(block.stmts)
                    && loop_update(*source).is_none_or(|update| self.stmt(update))
                {
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
                // Only the success clause sees the effects of the tried call.
                let before = self.state.clone();
                let _ = self.visit_expr(&t.expr);
                let after_call = self.state.clone();
                let mut joined = None::<State>;
                for (i, clause) in t.clauses.iter().enumerate() {
                    self.state = if i == 0 { after_call.clone() } else { before.clone() };
                    if self.visit_stmts(clause.block.stmts) {
                        joined = Some(
                            joined.map_or_else(|| self.state.clone(), |j| j.meet(&self.state)),
                        );
                    }
                }
                let falls = joined.is_some();
                self.state = joined.unwrap_or(after_call);
                return falls;
            }
            StmtKind::DeclSingle(vid) => {
                if let Some(init) = self.gcx.hir.variable(*vid).initializer {
                    let rhs = self.eval_rhs(Some(init));
                    self.assign_var(*vid, rhs);
                }
            }
            StmtKind::DeclMulti(vars, init) => {
                for (vid, rhs) in vars.iter().zip(tuple_elems(init).into_iter().flatten()) {
                    if let (Some(vid), Some(rhs)) = (vid, rhs) {
                        let rhs = self.eval_rhs(Some(rhs));
                        self.assign_var(*vid, rhs);
                    }
                }
            }
            _ => {}
        }
        let _ = self.walk_stmt(stmt);
        !branch_always_exits(stmt)
    }
}

impl<'gcx> Visit<'gcx> for Analyzer<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Never> {
        self.stmt(stmt);
        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Never> {
        match &expr.kind {
            // `rhs` may not execute: its facts and writes survive only if they also hold without
            // it, while `lhs` facts flow into `rhs`. Sinks in `rhs` are still reported.
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let _ = self.visit_expr(lhs);
                let skipped = self.state.clone();
                self.add_facts(lhs, op.kind == BinOpKind::Or);
                let _ = self.visit_expr(rhs);
                self.state = skipped.meet(&self.state);
            }
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                // Sinks inside the predicate run before the guard takes effect.
                let _ = self.walk_expr(expr);
                if let Some(cond) = args.exprs().next() {
                    self.add_facts(cond, false);
                }
            }
            ExprKind::Call(callee, ..) => {
                if let Some(rep) = match_flash_loan_call(self.gcx, expr) {
                    *self.state.repayments.entry(rep).or_insert(0) += 1;
                } else if let Some(permit) = self.match_permit_call(expr) {
                    self.state.permits.insert(permit);
                } else if let Some(sink) = match_sink(self.gcx, self.has_solady_lib, expr)
                    && !self.is_safe(sink.from)
                    && !self.consume_repayment(&sink)
                {
                    // A prior permit does not make the sink safe: a non-permit token with a
                    // fallback (e.g. WETH) silently accepts the permit.
                    let lint = if self.permit_covers(&sink) {
                        &ARBITRARY_SEND_ERC20_PERMIT
                    } else {
                        &ARBITRARY_SEND_ERC20
                    };
                    self.hits.push((expr.span, lint));
                }
                // Arguments are evaluated before the callee runs: walk them first, then drop facts
                // about state the callee writes.
                let _ = self.walk_expr(expr);
                if let Some(fid) = function_ids(callee).next() {
                    for v in state_writes(&self.gcx.hir, fid) {
                        self.invalidate(v);
                    }
                }
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

/// True when `expr` is `base(..)` or a variable in `vars`, through parens, `payable(..)`, casts,
/// ternaries whose both arms qualify and no-arg helpers whose body returns such an expression.
fn origin_matches(
    hir: &Hir<'_>,
    expr: &Expr<'_>,
    depth: u8,
    vars: &HashSet<VariableId>,
    base: fn(&Expr<'_>) -> bool,
) -> bool {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Payable(inner) => return origin_matches(hir, inner, depth, vars, base),
        ExprKind::Call(callee, args, _) if is_address_like_cast(callee) => {
            return args.exprs().next().is_some_and(|e| origin_matches(hir, e, depth, vars, base));
        }
        _ => {}
    }
    base(expr)
        || match &expr.kind {
            ExprKind::Ident(reses) => {
                reses.iter().filter_map(Res::as_variable).any(|v| vars.contains(&v))
            }
            ExprKind::Ternary(_, t, f) => {
                origin_matches(hir, t, depth, vars, base)
                    && origin_matches(hir, f, depth, vars, base)
            }
            ExprKind::Call(callee, args, _) if depth > 0 && args.exprs().next().is_none() => {
                function_ids(callee).any(|fid| {
                    let f = hir.function(fid);
                    f.parameters.is_empty()
                        && matches!(f.body.map(|b| b.stmts), Some([stmt])
                            if matches!(&stmt.kind, StmtKind::Return(Some(e))
                                if origin_matches(hir, e, depth - 1, vars, base)))
                })
            }
            _ => false,
        }
}

/// `a + b` with both operands variables.
fn sum_operands(expr: &Expr<'_>) -> Option<(VariableId, VariableId)> {
    match &expr.peel_parens().kind {
        ExprKind::Binary(lhs, op, rhs) if op.kind == BinOpKind::Add => {
            underlying_var(lhs).zip(underlying_var(rhs))
        }
        _ => None,
    }
}

/// `token` or `cfg.token` receiver key, through casts and `payable(..)`.
fn token_key(expr: &Expr<'_>) -> Option<TokenKey> {
    if let Some(v) = underlying_var(expr) {
        return Some(TokenKey::Var(v));
    }
    match &expr.peel_parens().kind {
        ExprKind::Member(base, ident) => Some(TokenKey::Field(underlying_var(base)?, ident.name)),
        _ => None,
    }
}

/// Positional or named call arguments in declaration order; `slots[i]` lists the parameter names
/// accepted for position `i`. `None` when the arity differs or a slot is unmatched.
fn canonical_args<'gcx>(
    args: &'gcx CallArgs<'gcx>,
    slots: &[&[&str]],
) -> Option<Vec<&'gcx Expr<'gcx>>> {
    if args.len() != slots.len() {
        return None;
    }
    match args.kind {
        CallArgsKind::Unnamed(exprs) => Some(exprs.iter().collect()),
        CallArgsKind::Named(named) => slots
            .iter()
            .map(|names| named.iter().find(|a| names.contains(&a.name.as_str())).map(|a| &a.value))
            .collect(),
    }
}

/// EIP-3156 `receiver.onFlashLoan(initiator, token, amount, fee, data)` on a receiver type
/// declaring the exact signature. Literal arguments yield `None`.
fn match_flash_loan_call<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> Option<PendingRepayment> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
    if ident.name.as_str() != "onFlashLoan" {
        return None;
    }
    let a = canonical_args(args, &[&["initiator"], &["token"], &["amount"], &["fee"], &["data"]])?;
    let cid = receiver_contract_id(gcx, recv)?;
    if !contract_has_function(
        &gcx.hir,
        cid,
        "onFlashLoan",
        &["address", "address", "uint256", "uint256", "bytes"],
        &["bytes32"],
    ) {
        return None;
    }
    Some(PendingRepayment {
        receiver: underlying_var(recv)?,
        token: underlying_var(a[1])?,
        amount: underlying_var(a[2])?,
        fee: underlying_var(a[3])?,
    })
}

/// `recv.transferFrom(from, to, amt)` / `recv.safeTransferFrom(from, to, amt)` on a contract
/// declaring ERC20's `transferFrom(address,address,uint256) returns (bool)` (ERC721's same-named
/// overload is excluded), `addr.safeTransferFrom(..)` via `using SafeTransferLib for address`,
/// or the library form `Lib.safeTransferFrom(token, from, to, amt)`.
fn match_sink<'gcx>(
    gcx: Gcx<'gcx>,
    has_solady_lib: bool,
    expr: &'gcx Expr<'gcx>,
) -> Option<Sink<'gcx>> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
    let name = ident.name.as_str();
    if matches!(name, "transferFrom" | "safeTransferFrom")
        && let Some(a) = canonical_args(args, &[&["from"], &["to"], &["value", "amount"]])
    {
        let erc20 =
            receiver_contract_id(gcx, recv).is_some_and(|cid| has_transfer_from(&gcx.hir, cid));
        // The HIR does not expose `using` bindings, so the `address` receiver form is accepted
        // only when a Solady-shaped library is compiled in.
        if erc20 || (name == "safeTransferFrom" && has_solady_lib && expr_is_address(gcx, recv)) {
            return Some(Sink { from: a[0], to: a[1], amount: a[2], token: token_key(recv) });
        }
    }
    if name == "safeTransferFrom"
        && let Some(a) =
            canonical_args(args, &[&["token"], &["from"], &["to"], &["value", "amount"]])
        && let Some(cid) = receiver_contract_id(gcx, recv)
        && gcx.hir.contract(cid).kind == ContractKind::Library
        && library_has_safe_transfer_from(&gcx.hir, cid)
    {
        return Some(Sink { from: a[1], to: a[2], amount: a[3], token: token_key(a[0]) });
    }
    None
}

/// State variables written by `fid` or by the internal functions it calls (one level deep).
fn state_writes<'gcx>(hir: &'gcx Hir<'gcx>, fid: FunctionId) -> HashSet<VariableId> {
    let mut w = StateWrites { hir, out: HashSet::new(), callees: Vec::new() };
    w.scan(fid);
    for callee in std::mem::take(&mut w.callees) {
        w.scan(callee);
    }
    w.out
}

struct StateWrites<'gcx> {
    hir: &'gcx Hir<'gcx>,
    out: HashSet<VariableId>,
    callees: Vec<FunctionId>,
}

impl StateWrites<'_> {
    fn scan(&mut self, fid: FunctionId) {
        if let Some(body) = self.hir.function(fid).body {
            for stmt in body.stmts {
                let _ = self.visit_stmt(stmt);
            }
        }
    }
}

impl<'gcx> Visit<'gcx> for StateWrites<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Never> {
        match &expr.kind {
            ExprKind::Assign(lhs, ..) | ExprKind::Delete(lhs) => {
                self.out.extend(state_lhs_vars(self.hir, lhs));
            }
            ExprKind::Call(callee, ..) => self.callees.extend(function_ids(callee).next()),
            _ => {}
        }
        self.walk_expr(expr)
    }
}

/// Internal functions and modifiers are only reachable from the compilation unit, so their
/// parameters can be proven safe from the invocation sites seen there.
const fn is_internal_only(f: &hir::Function<'_>) -> bool {
    !f.parameters.is_empty()
        && (matches!(f.kind, FunctionKind::Modifier)
            || (f.kind.is_function()
                && matches!(f.visibility, Visibility::Private | Visibility::Internal)))
}

/// Per internal function, whether every call site passes a statically safe / self argument for
/// each parameter; `None` when some call site could not be matched to the parameters.
type CallsiteFacts = HashMap<FunctionId, Option<Vec<(bool, bool)>>>;

thread_local! {
    static CALLSITE_INDEX: RefCell<Option<(usize, Rc<CallsiteFacts>)>> = const { RefCell::new(None) };
}

/// The call-site index of `hir`, built once per compilation unit.
fn callsite_index<'gcx>(hir: &'gcx Hir<'gcx>) -> Rc<CallsiteFacts> {
    let key = std::ptr::from_ref(hir) as usize;
    CALLSITE_INDEX.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some((cached_key, index)) = &*slot
            && *cached_key == key
        {
            return index.clone();
        }
        let mut c = CallsiteCollector { hir, out: HashMap::new() };
        for (_, func) in hir.functions_enumerated() {
            for m in func.modifiers {
                if let ItemId::Function(fid) = m.id {
                    c.record(fid, &m.args);
                }
            }
            for stmt in func.body.map_or(&[][..], |b| b.stmts) {
                let _ = c.visit_stmt(stmt);
            }
        }
        let index = Rc::new(c.out);
        *slot = Some((key, index.clone()));
        index
    })
}

struct CallsiteCollector<'gcx> {
    hir: &'gcx Hir<'gcx>,
    out: CallsiteFacts,
}

impl<'gcx> CallsiteCollector<'gcx> {
    fn record(&mut self, fid: FunctionId, args: &'gcx CallArgs<'gcx>) {
        let f = self.hir.function(fid);
        if !is_internal_only(f) {
            return;
        }
        let call_args = f.parameters.iter().map(|&p| arg_for_param(self.hir, f, p, args)).collect();
        let entry =
            self.out.entry(fid).or_insert_with(|| Some(vec![(true, true); f.parameters.len()]));
        let (Some(facts), Some(call_args)) = (entry.as_mut(), call_args) else {
            *entry = None;
            return;
        };
        let none = HashSet::new();
        for ((safe, is_self), arg) in facts.iter_mut().zip::<Vec<_>>(call_args) {
            *safe &= origin_matches(self.hir, arg, HELPER_DEPTH, &none, |e| {
                is_msg_sender(e) || is_address_self(e)
            });
            *is_self &= origin_matches(self.hir, arg, HELPER_DEPTH, &none, is_address_self);
        }
    }
}

impl<'gcx> Visit<'gcx> for CallsiteCollector<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Never> {
        if let ExprKind::Call(callee, args, _) = &expr.kind
            && let Some(fid) = function_ids(callee).next()
        {
            self.record(fid, args);
        }
        self.walk_expr(expr)
    }
}

/// Whether the sources declare a Solady-shaped `SafeTransferLib` library.
fn has_solady_safe_transfer_lib(hir: &Hir<'_>) -> bool {
    hir.contracts_enumerated().any(|(cid, c)| {
        c.kind == ContractKind::Library
            && c.name.as_str() == "SafeTransferLib"
            && library_has_safe_transfer_from(hir, cid)
    })
}

/// ERC20's `transferFrom(address,address,uint256) returns (bool)`.
fn has_transfer_from(hir: &Hir<'_>, cid: ContractId) -> bool {
    contract_has_function(hir, cid, "transferFrom", &["address", "address", "uint256"], &["bool"])
}

fn contract_has_function(
    hir: &Hir<'_>,
    cid: ContractId,
    name: &str,
    params: &[&str],
    returns: &[&str],
) -> bool {
    hir.contract(cid).functions().any(|fid| {
        let f = hir.function(fid);
        f.name.is_some_and(|n| n.name.as_str() == name)
            && f.parameters.len() == params.len()
            && f.returns.len() == returns.len()
            && f.parameters.iter().zip(params).all(|(id, abi)| is_elementary(hir, *id, abi))
            && f.returns.iter().zip(returns).all(|(id, abi)| is_elementary(hir, *id, abi))
    })
}

/// 4-arg `safeTransferFrom(token, address, address, uint256)` where `token` is `address` (Solady)
/// or an ERC20 contract type (OpenZeppelin `SafeERC20`); ERC721/1155 helpers are excluded since
/// their `transferFrom` has no return value.
fn library_has_safe_transfer_from(hir: &Hir<'_>, cid: ContractId) -> bool {
    hir.contract(cid).functions().any(|fid| {
        let f = hir.function(fid);
        let [token, from, to, amount] = f.parameters else { return false };
        let token_ok = match hir.variable(*token).ty.kind {
            TypeKind::Custom(ItemId::Contract(token_cid)) => has_transfer_from(hir, token_cid),
            _ => is_address_type(hir, *token),
        };
        f.name.is_some_and(|n| n.name.as_str() == "safeTransferFrom")
            && token_ok
            && is_address_type(hir, *from)
            && is_address_type(hir, *to)
            && is_elementary(hir, *amount, "uint256")
    })
}
