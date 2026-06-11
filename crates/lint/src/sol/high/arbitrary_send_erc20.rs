use super::ArbitrarySendErc20;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    interface::{Span, data_structures::Never, kw, sym},
    sema::{
        Gcx,
        hir::{
            self, ContractKind, ElementaryType, ExprKind, ItemId, LoopSource, Res, StmtKind,
            TypeKind, Visit,
        },
        ty::{Ty, TyKind},
    },
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
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

impl<'hir> LateLintPass<'hir> for ArbitrarySendErc20 {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if !func.kind.is_function()
            || matches!(
                func.state_mutability,
                ast::StateMutability::Pure | ast::StateMutability::View
            )
        {
            return;
        }
        // Library functions typically forward `from` from their caller; flag at the call
        // site (in user contracts) instead, where the trust boundary actually lives.
        if func.contract.is_some_and(|cid| hir.contract(cid).kind == ContractKind::Library) {
            return;
        }
        let Some(body) = func.body else { return };

        let has_solady_lib = has_solady_safe_transfer_lib(hir);
        // Skip when a modifier's prefix definitely exits before `_;`.
        if func.modifiers.iter().any(|m| modifier_prefix_always_exits(hir, m)) {
            return;
        }
        let mut a = Analyzer::new(gcx, hir, has_solady_lib);
        // Seed `self_vars` / `safe_vars` with immutable/constant state vars proven
        // equal to `address(this)` / `msg.sender` by their declaration initializer
        // or the contract's constructor.
        if let Some(cid) = func.contract {
            seed_immutable_facts(gcx, hir, has_solady_lib, cid, &mut a);
        }
        seed_internal_callsite_facts(gcx, hir, has_solady_lib, func, &mut a);
        for m in func.modifiers {
            collect_modifier_safety(
                gcx,
                hir,
                has_solady_lib,
                m,
                &mut a.safe_vars,
                &mut a.self_vars,
            );
        }
        for stmt in body.stmts {
            let _ = a.visit_stmt(stmt);
            // Skip dead code after a definite exit.
            if branch_always_exits(stmt) {
                break;
            }
        }
        for (span, lint) in a.hits {
            ctx.emit(lint, span);
        }
    }
}

/// `(token, owner)` of an EIP-2612 permit recorded earlier on the current path with
/// `spender == address(this)`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PermitRecord {
    token: TokenKey,
    owner: hir::VariableId,
}

/// Identifier used to correlate permit / sink token receivers. `Field` lets
/// `cfg.token.permit(...)` and `cfg.token.transferFrom(...)` match (FN-5).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum TokenKey {
    Var(hir::VariableId),
    Field(hir::VariableId, solar::interface::Symbol),
}

impl TokenKey {
    fn touches(&self, v: hir::VariableId) -> bool {
        match self {
            Self::Var(x) | Self::Field(x, _) => *x == v,
        }
    }
}

/// RHS facts captured before any write so tuple-swap assignments stay consistent.
#[derive(Clone, Copy, Default)]
struct AssignRhs {
    is_safe: bool,
    is_self: bool,
    alias: Option<hir::VariableId>,
    sum: Option<(hir::VariableId, hir::VariableId)>,
}

/// Outstanding EIP-3156 repayment licensed by a prior `onFlashLoan` call.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PendingRepayment {
    receiver: hir::VariableId,
    token: hir::VariableId,
    amount: hir::VariableId,
    fee: hir::VariableId,
}

struct Analyzer<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    /// Variables proven safe (equal to `msg.sender` or `address(this)`) on this path.
    /// Function-locals and `immutable`/`constant` state vars only — mutable storage may be
    /// rewritten between the check and the sink.
    safe_vars: HashSet<hir::VariableId>,
    /// Locals proven equal to `address(this)`. Subset of `safe_vars`, used to recognise
    /// the permit `spender` arg via an alias.
    self_vars: HashSet<hir::VariableId>,
    /// Permits seen earlier on this path. Killed on token/owner reassignment.
    permits: HashSet<PermitRecord>,
    /// Pending flash-loan repayments, as a multiset: repeated `onFlashLoan` calls each
    /// license one consumption.
    repayments: HashMap<PendingRepayment, u32>,
    /// `x = y` records `x -> canonical(y)`; killed on writes to either side.
    aliases: HashMap<hir::VariableId, hir::VariableId>,
    /// `x = a + b` records `x -> (a, b)`; matches flash-repayment sums.
    sum_of: HashMap<hir::VariableId, (hir::VariableId, hir::VariableId)>,
    /// Gates the `using ... for address` sink branch on a `SafeTransferLib` being present.
    has_solady_lib: bool,
    hits: Vec<(Span, &'static SolLint)>,
    /// Vars written by an assignment / `delete`. Used to drop unsound modifier-param
    /// hoists when the modifier rewrote the param.
    written: HashSet<hir::VariableId>,
}

#[derive(Clone)]
struct FlowState {
    safe_vars: HashSet<hir::VariableId>,
    self_vars: HashSet<hir::VariableId>,
    permits: HashSet<PermitRecord>,
    repayments: HashMap<PendingRepayment, u32>,
    aliases: HashMap<hir::VariableId, hir::VariableId>,
    sum_of: HashMap<hir::VariableId, (hir::VariableId, hir::VariableId)>,
}

#[derive(Default)]
struct ProjectIndex {
    function_ids_by_ptr: HashMap<usize, hir::FunctionId>,
    internal_callsites: HashMap<hir::FunctionId, ParamCallsiteFacts>,
}

struct ParamCallsiteFacts {
    seen: Vec<bool>,
    all_safe: Vec<bool>,
    all_self: Vec<bool>,
    unknown: bool,
}

impl ParamCallsiteFacts {
    fn new(len: usize) -> Self {
        Self {
            seen: vec![false; len],
            all_safe: vec![true; len],
            all_self: vec![true; len],
            unknown: false,
        }
    }
}

impl FlowState {
    fn empty() -> Self {
        Self {
            safe_vars: HashSet::new(),
            self_vars: HashSet::new(),
            permits: HashSet::new(),
            repayments: HashMap::new(),
            aliases: HashMap::new(),
            sum_of: HashMap::new(),
        }
    }

    fn intersection(a: &Self, b: &Self) -> Self {
        Self {
            safe_vars: a.safe_vars.intersection(&b.safe_vars).copied().collect(),
            self_vars: a.self_vars.intersection(&b.self_vars).copied().collect(),
            permits: a.permits.intersection(&b.permits).copied().collect(),
            // Multiset intersection: min per key.
            repayments: a
                .repayments
                .iter()
                .filter_map(|(k, va)| b.repayments.get(k).map(|vb| (*k, *va.min(vb))))
                .collect(),
            aliases: a
                .aliases
                .iter()
                .filter_map(|(k, v)| (b.aliases.get(k) == Some(v)).then_some((*k, *v)))
                .collect(),
            sum_of: a
                .sum_of
                .iter()
                .filter_map(|(k, v)| (b.sum_of.get(k) == Some(v)).then_some((*k, *v)))
                .collect(),
        }
    }

    fn intersection_all(mut states: impl Iterator<Item = Self>) -> Self {
        let mut out = states.next().unwrap_or_else(Self::empty);
        for state in states {
            out = Self::intersection(&out, &state);
        }
        out
    }
}

/// Recursion budget for `_msgSender()`-style helper chains.
const HELPER_DEPTH: u8 = 3;

impl<'hir> Analyzer<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir hir::Hir<'hir>, has_solady_lib: bool) -> Self {
        Self {
            gcx,
            hir,
            safe_vars: HashSet::new(),
            self_vars: HashSet::new(),
            permits: HashSet::new(),
            repayments: HashMap::new(),
            aliases: HashMap::new(),
            sum_of: HashMap::new(),
            has_solady_lib,
            hits: Vec::new(),
            written: HashSet::new(),
        }
    }

    fn snapshot(&self) -> FlowState {
        FlowState {
            safe_vars: self.safe_vars.clone(),
            self_vars: self.self_vars.clone(),
            permits: self.permits.clone(),
            repayments: self.repayments.clone(),
            aliases: self.aliases.clone(),
            sum_of: self.sum_of.clone(),
        }
    }

    fn restore(&mut self, state: FlowState) {
        self.safe_vars = state.safe_vars;
        self.self_vars = state.self_vars;
        self.permits = state.permits;
        self.repayments = state.repayments;
        self.aliases = state.aliases;
        self.sum_of = state.sum_of;
    }

    /// Follows the alias chain to its root. Bounded to guard against cycles.
    fn canonical(&self, v: hir::VariableId) -> hir::VariableId {
        let mut cur = v;
        for _ in 0..8 {
            match self.aliases.get(&cur).copied() {
                Some(next) if next != cur => cur = next,
                _ => break,
            }
        }
        cur
    }

    fn is_safe(&self, expr: &hir::Expr<'_>) -> bool {
        self.is_safe_inner(expr, HELPER_DEPTH)
    }

    fn is_safe_inner(&self, expr: &hir::Expr<'_>, depth: u8) -> bool {
        match &expr.peel_parens().kind {
            ExprKind::Member(base, ident) if ident.name == sym::sender => {
                is_builtin(base, sym::msg)
            }
            ExprKind::Ident(_) if is_builtin(expr, sym::this) => true,
            ExprKind::Ident(reses) => reses.iter().any(
                |r| matches!(r, Res::Item(ItemId::Variable(vid)) if self.safe_vars.contains(vid)),
            ),
            ExprKind::Call(callee, args, _) if is_address_cast(callee) => {
                args.exprs().next().is_some_and(|e| self.is_safe_inner(e, depth))
            }
            ExprKind::Payable(inner) => self.is_safe_inner(inner, depth),
            ExprKind::Ternary(_, t, f) => {
                self.is_safe_inner(t, depth) && self.is_safe_inner(f, depth)
            }
            // No-arg helper whose body is `return X;` with `X` statically safe (e.g. `_msgSender`).
            ExprKind::Call(callee, args, _)
                if depth > 0
                    && args.exprs().next().is_none()
                    && self.callee_returns_safe(callee, depth - 1) =>
            {
                true
            }
            _ => false,
        }
    }

    fn callee_returns_safe(&self, callee: &hir::Expr<'_>, depth: u8) -> bool {
        let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return false };
        reses.iter().any(|r| match r {
            Res::Item(ItemId::Function(fid)) => {
                let f = self.hir.function(*fid);
                let Some(body) = f.body else { return false };
                f.parameters.is_empty()
                    && body.stmts.len() == 1
                    && matches!(
                        &body.stmts[0].kind,
                        StmtKind::Return(Some(e)) if self.is_safe_inner(e, depth)
                    )
            }
            _ => false,
        })
    }

    /// Variant of `assign_one_flags` for declarations: skips permit/repayment kills
    /// since a freshly declared variable has no prior facts.
    fn assign(&mut self, target: hir::VariableId, rhs: &hir::Expr<'_>) {
        let eval = self.eval_rhs(Some(rhs));
        self.written.insert(target);
        if eval.is_safe {
            self.safe_vars.insert(target);
        } else {
            self.safe_vars.remove(&target);
        }
        if eval.is_self {
            self.self_vars.insert(target);
        } else {
            self.self_vars.remove(&target);
        }
        if let Some(alias) = eval.alias
            && alias != target
        {
            self.aliases.insert(target, alias);
        }
        if let Some(sum) = eval.sum {
            self.sum_of.insert(target, sum);
        }
    }

    /// `true` when `expr` resolves to `address(this)`: bare form, a local alias,
    /// `payable(...)`/cast wraps, a ternary whose both branches are self, or a no-arg
    /// helper returning a statically-self expression.
    fn is_self_expr(&self, expr: &hir::Expr<'_>) -> bool {
        self.is_self_expr_inner(expr, HELPER_DEPTH)
    }

    fn is_self_expr_inner(&self, expr: &hir::Expr<'_>, depth: u8) -> bool {
        let expr = expr.peel_parens();
        if is_address_self(expr) {
            return true;
        }
        match &expr.kind {
            ExprKind::Ident(reses) => reses.iter().any(
                |r| matches!(r, Res::Item(ItemId::Variable(vid)) if self.self_vars.contains(vid)),
            ),
            ExprKind::Payable(inner) => self.is_self_expr_inner(inner, depth),
            ExprKind::Call(callee, args, _) if is_address_cast(callee) => {
                args.exprs().next().is_some_and(|e| self.is_self_expr_inner(e, depth))
            }
            ExprKind::Ternary(_, t, f) => {
                self.is_self_expr_inner(t, depth) && self.is_self_expr_inner(f, depth)
            }
            ExprKind::Call(callee, args, _)
                if depth > 0
                    && args.exprs().next().is_none()
                    && self.callee_returns_self(callee, depth - 1) =>
            {
                true
            }
            _ => false,
        }
    }

    fn callee_returns_self(&self, callee: &hir::Expr<'_>, depth: u8) -> bool {
        let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return false };
        reses.iter().any(|r| match r {
            Res::Item(ItemId::Function(fid)) => {
                let f = self.hir.function(*fid);
                let Some(body) = f.body else { return false };
                f.parameters.is_empty()
                    && body.stmts.len() == 1
                    && matches!(
                        &body.stmts[0].kind,
                        StmtKind::Return(Some(e)) if self.is_self_expr_inner(e, depth)
                    )
            }
            _ => false,
        })
    }

    /// Matches EIP-2612 `token.permit(owner, <self>, value, deadline, v, r, s)`, plus
    /// the library form `Lib.safePermit(token, owner, <self>, value, deadline, v, r, s)`
    /// (OpenZeppelin-style wrapper that delegates to `token.permit`).
    fn match_permit_call(&self, expr: &'hir hir::Expr<'hir>) -> Option<PermitRecord> {
        let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
        let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
        let name = ident.name.as_str();

        if name == "permit"
            && let Some(canonical) = canonical_args(
                args.kind,
                &[&["owner"], &["spender"], &["value"], &["deadline"], &["v"], &["r"], &["s"]],
            )
            && self.is_self_expr(canonical[1])
        {
            return Some(PermitRecord {
                token: self.canonical_key(token_key(recv)?),
                owner: self.canonical(underlying_var(canonical[0])?),
            });
        }

        if name == "safePermit"
            && let Some(canonical) = canonical_args(
                args.kind,
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
            )
            && self.is_self_expr(canonical[2])
            && let Some(cid) = receiver_contract_id(self.gcx, recv)
            && self.hir.contract(cid).kind == ContractKind::Library
        {
            return Some(PermitRecord {
                token: self.canonical_key(token_key(canonical[0])?),
                owner: self.canonical(underlying_var(canonical[1])?),
            });
        }

        None
    }

    /// Drops permits referencing `target` by raw id. Permits are stored at the
    /// canonical chain root, so reassigning an alias var leaves the root's permit
    /// intact — only reassigning the root itself kills it.
    fn kill_permits_for(&mut self, target: hir::VariableId) {
        self.permits.retain(|p| !p.token.touches(target) && p.owner != target);
    }

    /// Canonicalizes a `TokenKey`: `Var` chases the alias chain; `Field`'s base
    /// var is canonicalized so `aCfg = cfg; aCfg.token` aliases to `cfg.token`.
    fn canonical_key(&self, k: TokenKey) -> TokenKey {
        match k {
            TokenKey::Var(v) => TokenKey::Var(self.canonical(v)),
            TokenKey::Field(v, s) => TokenKey::Field(self.canonical(v), s),
        }
    }

    /// Drops all facts about `v` (treats it as freshly assigned an unknown value).
    fn invalidate(&mut self, v: hir::VariableId) {
        self.safe_vars.remove(&v);
        self.self_vars.remove(&v);
        self.aliases.remove(&v);
        self.sum_of.remove(&v);
        self.kill_permits_for(v);
    }

    /// Returns state vars written by `callee` if it resolves to a function in the
    /// current contract; walks one level of nested internal calls. Resolution is
    /// conservative: any unresolved or non-local call returns `None`.
    fn call_state_writes(&self, callee: &hir::Expr<'_>) -> Option<HashSet<hir::VariableId>> {
        let fid = resolve_internal_fn(callee)?;
        let f = self.hir.function(fid);
        let body = f.body?;
        // Only same-contract calls to non-view/pure user functions can mutate `self`'s state.
        if matches!(f.state_mutability, ast::StateMutability::Pure | ast::StateMutability::View) {
            return Some(HashSet::new());
        }
        let mut writes = collect_state_writes(self.hir, body.stmts);
        // One nested level: pull in writes from internal callees within `body`.
        let mut nested = NestedCallCollector { hir: self.hir, out: HashSet::new() };
        for s in body.stmts {
            let _ = nested.visit_stmt(s);
        }
        for cid in nested.out {
            let nf = self.hir.function(cid);
            if let Some(nb) = nf.body {
                writes.extend(collect_state_writes(self.hir, nb.stmts));
            }
        }
        Some(writes)
    }

    fn permit_covers(&self, token: Option<TokenKey>, from: &hir::Expr<'_>) -> bool {
        let (Some(token), Some(owner)) = (token, underlying_var(from)) else { return false };
        self.permits.contains(&PermitRecord {
            token: self.canonical_key(token),
            owner: self.canonical(owner),
        })
    }

    /// `amount_arg` is `amount + fee` syntactically, or a local var bound to that sum.
    fn amount_matches(
        &self,
        amount_arg: &hir::Expr<'_>,
        amount: hir::VariableId,
        fee: hir::VariableId,
    ) -> bool {
        if is_amount_plus_fee(amount_arg, amount, fee) {
            return true;
        }
        let Some(v) = underlying_var(amount_arg) else { return false };
        matches!(self.sum_of.get(&v), Some((a, b))
            if (*a == amount && *b == fee) || (*a == fee && *b == amount))
    }

    /// Drops pending repayments referencing `target`.
    fn kill_repayments_for(&mut self, target: hir::VariableId) {
        self.repayments.retain(|r, _| {
            r.receiver != target && r.token != target && r.amount != target && r.fee != target
        });
    }

    /// Matches `from`/`token` plus a sink call with `to == address(this)` and amount
    /// `amount + fee` against a pending repayment, consuming one occurrence on hit.
    fn consume_repayment(
        &mut self,
        call_expr: &hir::Expr<'_>,
        from: &hir::Expr<'_>,
        token: Option<TokenKey>,
    ) -> bool {
        let Some(from_v) = underlying_var(from) else { return false };
        // Repayments are recorded as `(receiver, token)` raw `VariableId`s; `Field`
        // sinks cannot match a flash-loan token state var directly.
        let Some(TokenKey::Var(token_v)) = token else { return false };
        let ExprKind::Call(_, args, _) = &call_expr.kind else { return false };
        // Pick `to` and `amount` from whichever sink shape (3-arg member / 4-arg library).
        let (to_arg, amount_arg) = if let Some(a) =
            canonical_args(args.kind, &[&["from"], &["to"], &["value", "amount"]])
        {
            (a[1], a[2])
        } else if let Some(a) =
            canonical_args(args.kind, &[&["token"], &["from"], &["to"], &["value", "amount"]])
        {
            (a[2], a[3])
        } else {
            return false;
        };
        if !self.is_self_expr(to_arg) {
            return false;
        }
        let matched = self.repayments.keys().copied().find(|r| {
            r.receiver == from_v
                && r.token == token_v
                && self.amount_matches(amount_arg, r.amount, r.fee)
        });
        if let Some(rep) = matched {
            match self.repayments.get_mut(&rep) {
                Some(count) if *count > 1 => *count -= 1,
                _ => {
                    self.repayments.remove(&rep);
                }
            }
            true
        } else {
            false
        }
    }

    /// Records vars proven safe by `pred`. Handles `==`/`!=`, `&&`/`||` and `!` via
    /// De Morgan; disjunctions keep only facts true on both sides.
    fn add_facts(&mut self, pred: &hir::Expr<'_>, negate: bool) {
        match &pred.peel_parens().kind {
            ExprKind::Binary(lhs, op, rhs) => {
                let (eq, and, or) = if negate {
                    (ast::BinOpKind::Ne, ast::BinOpKind::Or, ast::BinOpKind::And)
                } else {
                    (ast::BinOpKind::Eq, ast::BinOpKind::And, ast::BinOpKind::Or)
                };
                if op.kind == and {
                    self.add_facts(lhs, negate);
                    self.add_facts(rhs, negate);
                } else if op.kind == or {
                    // Disjunction: keep facts true in both arms.
                    let baseline = self.snapshot();
                    self.add_facts(lhs, negate);
                    let after_lhs = self.snapshot();
                    self.restore(baseline);
                    self.add_facts(rhs, negate);
                    let after_rhs = self.snapshot();
                    self.restore(FlowState::intersection(&after_lhs, &after_rhs));
                } else if op.kind == eq {
                    for (a, b) in [(lhs, rhs), (rhs, lhs)] {
                        if let Some(v) = underlying_var(b)
                            && self.is_safe_target(v)
                        {
                            if self.is_safe(a) {
                                self.safe_vars.insert(v);
                            }
                            // Equality with `address(this)` also makes `b` a self alias.
                            if self.is_self_expr(a) {
                                self.self_vars.insert(v);
                            }
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

    fn is_safe_target(&self, v: hir::VariableId) -> bool {
        let var = self.hir.variable(v);
        !var.kind.is_state() || var.is_immutable() || var.is_constant()
    }

    /// Drops permits whose token is `Field(base, name)` when the LHS is the
    /// corresponding `<base>.<name>` (assignment or `delete`).
    fn kill_field_permits(&mut self, lhs: &hir::Expr<'_>) {
        let lhs = lhs.peel_parens();
        if let ExprKind::Member(base, ident) = &lhs.kind
            && let Some(base_v) = underlying_var(base)
        {
            let key = TokenKey::Field(self.canonical(base_v), ident.name);
            self.permits.retain(|p| p.token != key);
        }
    }

    /// Handles single-var and tuple LHS; tuple slots align with a tuple-literal RHS.
    fn handle_assign(&mut self, lhs: &hir::Expr<'_>, rhs: &hir::Expr<'_>) {
        let lhs = lhs.peel_parens();
        if let ExprKind::Tuple(lhs_elems) = &lhs.kind {
            let rhs_elems = match &rhs.peel_parens().kind {
                ExprKind::Tuple(r) => Some(*r),
                _ => None,
            };
            // Read all RHS facts before any write — handles `(x, y) = (y, x)` swaps.
            type EvaluatedSlot<'a> = (Option<&'a hir::Expr<'a>>, AssignRhs);
            let evaluated: Vec<EvaluatedSlot<'_>> = lhs_elems
                .iter()
                .enumerate()
                .map(|(i, lhs_elem)| {
                    let lhs_expr = lhs_elem.as_deref();
                    let rhs_expr = rhs_elems.and_then(|r| r.get(i).copied()).flatten();
                    (lhs_expr, self.eval_rhs(rhs_expr))
                })
                .collect();
            for (lhs_expr, eval) in evaluated {
                if let Some(lhs_expr) = lhs_expr {
                    self.assign_one_flags(lhs_expr, eval);
                }
            }
        } else {
            self.assign_one(lhs, Some(rhs));
        }
    }

    /// `rhs == None` (unknown slot) drops the target's safe-fact.
    fn assign_one(&mut self, lhs: &hir::Expr<'_>, rhs: Option<&hir::Expr<'_>>) {
        let eval = self.eval_rhs(rhs);
        self.assign_one_flags(lhs, eval);
    }

    /// Pre-evaluates RHS facts; the resulting `AssignRhs` is independent of later
    /// state changes (needed for tuple-swap assignments).
    fn eval_rhs(&self, rhs: Option<&hir::Expr<'_>>) -> AssignRhs {
        let Some(r) = rhs else { return AssignRhs::default() };
        let alias = underlying_var(r).map(|v| self.canonical(v));
        let sum = if let ExprKind::Binary(lhs, op, rhs_inner) = &r.peel_parens().kind
            && matches!(op.kind, ast::BinOpKind::Add)
        {
            underlying_var(lhs).zip(underlying_var(rhs_inner))
        } else {
            None
        };
        AssignRhs { is_safe: self.is_safe(r), is_self: self.is_self_expr(r), alias, sum }
    }

    /// Applies a pre-evaluated RHS to the LHS variable.
    fn assign_one_flags(&mut self, lhs: &hir::Expr<'_>, eval: AssignRhs) {
        let Some(target) = underlying_var(lhs) else { return };
        self.written.insert(target);
        self.kill_permits_for(target);
        self.kill_repayments_for(target);
        self.safe_vars.remove(&target);
        self.self_vars.remove(&target);
        // Drop alias / sum edges that reference the target on either side.
        self.aliases.remove(&target);
        self.aliases.retain(|_, dst| *dst != target);
        self.sum_of.remove(&target);
        self.sum_of.retain(|_, (a, b)| *a != target && *b != target);
        // Mutable storage can be rewritten between check and sink; only locals and
        // immutables/constants are safe to track.
        let var = self.hir.variable(target);
        if var.kind.is_state() && !var.is_immutable() && !var.is_constant() {
            return;
        }
        if eval.is_safe {
            self.safe_vars.insert(target);
        }
        if eval.is_self {
            self.self_vars.insert(target);
        }
        if let Some(alias) = eval.alias
            && alias != target
        {
            self.aliases.insert(target, alias);
        }
        if let Some(sum) = eval.sum {
            self.sum_of.insert(target, sum);
        }
    }

    /// Visits a body that may execute zero times or out-of-line (loops, try clauses):
    /// in-body kills survive, in-body additions don't.
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
            // Nested loops own their own `break` / `continue`; they do not exit this isolated body.
            StmtKind::Loop(..) => {
                let _ = self.visit_stmt(stmt);
                (!branch_always_exits(stmt)).then_some(())
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
            // Branch-sensitive join: positive facts flow into `then`; if the then-branch
            // always exits, negated facts (`!cond`) flow into the fall-through.
            StmtKind::If(cond, then, else_) => {
                let _ = self.visit_expr(cond);

                let baseline = self.snapshot();
                self.add_facts(cond, false);
                let _ = self.visit_stmt(then);
                let then_exits = branch_always_exits(then);
                let after_then = self.snapshot();

                // Both the explicit `else` body and the implicit fall-through inherit `!cond`.
                self.restore(baseline);
                self.add_facts(cond, true);
                let else_exits = match else_ {
                    Some(e) => {
                        let _ = self.visit_stmt(e);
                        branch_always_exits(e)
                    }
                    None => false,
                };
                let after_else = self.snapshot();

                let joined = match (then_exits, else_exits) {
                    // Both branches exit: downstream is unreachable, take a conservative
                    // union to match the previous behaviour.
                    (true, true) => FlowState {
                        safe_vars: after_then
                            .safe_vars
                            .union(&after_else.safe_vars)
                            .copied()
                            .collect(),
                        self_vars: after_then
                            .self_vars
                            .union(&after_else.self_vars)
                            .copied()
                            .collect(),
                        permits: after_then.permits.union(&after_else.permits).copied().collect(),
                        // Multiset union: max per key.
                        repayments: {
                            let mut m = after_then.repayments;
                            for (k, vb) in &after_else.repayments {
                                let entry = m.entry(*k).or_insert(0);
                                if *entry < *vb {
                                    *entry = *vb;
                                }
                            }
                            m
                        },
                        aliases: {
                            let mut m = after_then.aliases;
                            for (k, v) in after_else.aliases {
                                m.entry(k).or_insert(v);
                            }
                            m
                        },
                        sum_of: {
                            let mut m = after_then.sum_of;
                            for (k, v) in after_else.sum_of {
                                m.entry(k).or_insert(v);
                            }
                            m
                        },
                    },
                    (true, false) => after_else,
                    (false, true) => after_then,
                    (false, false) => FlowState::intersection(&after_then, &after_else),
                };
                self.restore(joined);
                return ControlFlow::Continue(());
            }

            StmtKind::Loop(block, source) => {
                // `do-while` runs the body at least once, so facts flow out — unless a user
                // `break`/`continue` can skip later assignments.
                if matches!(source, LoopSource::DoWhile)
                    && !body_has_break_or_continue(do_while_user_stmts(block.stmts))
                {
                    for s in block.stmts {
                        let _ = self.visit_stmt(s);
                        if branch_always_exits(s) {
                            break;
                        }
                    }
                } else if matches!(source, LoopSource::DoWhile) {
                    // do-while runs at least once: intersect actual break/continue
                    // exits and the body's fallthrough, without the pre-loop baseline.
                    let mut exits = vec![];
                    if let Some(fallthrough) =
                        self.visit_stmts_until_loop_exit(block.stmts, &mut exits)
                    {
                        exits.push(fallthrough);
                    }
                    if !exits.is_empty() {
                        self.restore(FlowState::intersection_all(exits.into_iter()));
                    }
                } else {
                    self.visit_isolated(block.stmts);
                }
                return ControlFlow::Continue(());
            }
            StmtKind::Try(t) => {
                // Success clause inherits `t.expr`'s facts; catch clauses don't,
                // since they only run if the call reverted.
                let baseline = self.snapshot();
                let _ = self.visit_expr(&t.expr);
                let after_call = self.snapshot();
                let mut post_clauses = Vec::with_capacity(t.clauses.len());
                for (i, clause) in t.clauses.iter().enumerate() {
                    self.restore(if i == 0 { after_call.clone() } else { baseline.clone() });
                    for s in clause.block.stmts {
                        let _ = self.visit_stmt(s);
                        if branch_always_exits(s) {
                            break;
                        }
                    }
                    // Clauses that always exit don't reach the post-try point and
                    // must not constrain the join.
                    if !clause.block.stmts.iter().any(branch_always_exits) {
                        post_clauses.push(self.snapshot());
                    }
                }
                let joined = if post_clauses.is_empty() {
                    // All clauses exit; downstream is unreachable.
                    after_call
                } else {
                    FlowState::intersection_all(post_clauses.into_iter())
                };
                self.restore(joined);
                return ControlFlow::Continue(());
            }

            // Stop at the first definite-exit inside a sequential block.
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                for s in block.stmts {
                    let _ = self.visit_stmt(s);
                    if branch_always_exits(s) {
                        break;
                    }
                }
                return ControlFlow::Continue(());
            }

            // `assign` only sets facts for vars whose RHS produces them, so it's
            // safe (and necessary, for `sum_of`) to call on all declarations.
            StmtKind::DeclSingle(vid) => {
                if let Some(init) = self.hir.variable(*vid).initializer {
                    self.assign(*vid, init);
                }
            }

            // Position-aligned propagation from a tuple literal RHS.
            StmtKind::DeclMulti(vars, init) => {
                if let ExprKind::Tuple(rhs) = &init.peel_parens().kind {
                    for (lhs, rhs) in vars.iter().zip(rhs.iter()) {
                        if let (Some(vid), Some(expr)) = (lhs, rhs) {
                            self.assign(*vid, expr);
                        }
                    }
                }
            }

            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        // Short-circuit `&&` / `||`: `rhs` may not execute, so its state mutations must
        // not survive the expression. `lhs` facts flow into `rhs` only. `hits` persist —
        // a sink in `rhs` is still reported.
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

        match &expr.kind {
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                // Walk the predicate before recording its facts so sinks inside the
                // predicate see the pre-guard state.
                let result = self.walk_expr(expr);
                if let Some(cond) = args.exprs().next() {
                    self.add_facts(cond, false);
                }
                return result;
            }
            ExprKind::Call(callee, ..) => {
                // EIP-3156: a matching repayment sink consumes the recorded obligation.
                if let Some(rep) = match_flash_loan_call(self.gcx, self.hir, expr) {
                    *self.repayments.entry(rep).or_insert(0) += 1;
                } else if let Some(p) = self.match_permit_call(expr) {
                    self.permits.insert(p);
                } else if let Some((from, token)) =
                    match_sink(self.gcx, self.hir, self.has_solady_lib, expr)
                    && !self.is_safe(from)
                    && !self.consume_repayment(expr, from, token)
                {
                    // A matching prior permit doesn't make the sink safe: a non-permit
                    // token with a fallback (e.g. WETH) silently accepts the permit.
                    let lint = if self.permit_covers(token, from) {
                        &ARBITRARY_SEND_ERC20_PERMIT
                    } else {
                        &ARBITRARY_SEND_ERC20
                    };
                    self.hits.push((expr.span, lint));
                } else if let Some(writes) = self.call_state_writes(callee) {
                    // Solidity evaluates args before invoking the callee. Walk first so
                    // nested sinks see the still-live facts, then invalidate.
                    let result = self.walk_expr(expr);
                    for v in writes {
                        self.invalidate(v);
                    }
                    return result;
                }
            }
            ExprKind::Assign(lhs, _, rhs) => {
                self.kill_field_permits(lhs);
                self.handle_assign(lhs, rhs);
            }
            // `delete x` resets `x`; treat as unknown reassignment.
            ExprKind::Delete(target) => {
                self.kill_field_permits(target);
                self.assign_one(target.peel_parens(), None);
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

/// EIP-3156 `onFlashLoan(address,address,uint256,uint256,bytes) returns (bytes32)`. Only
/// returns a record when the receiver type declares the exact sig and every tracked arg
/// resolves to a `VariableId`; literal args yield `None`.
fn match_flash_loan_call<'hir>(
    gcx: Gcx<'hir>,
    hir: &hir::Hir<'hir>,
    expr: &hir::Expr<'hir>,
) -> Option<PendingRepayment> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
    if ident.name.as_str() != "onFlashLoan" {
        return None;
    }
    let args =
        canonical_args(args.kind, &[&["initiator"], &["token"], &["amount"], &["fee"], &["data"]])?;
    let cid = receiver_contract_id(gcx, recv)?;
    if !contract_has_function(
        hir,
        cid,
        "onFlashLoan",
        &["address", "address", "uint256", "uint256", "bytes"],
        &["bytes32"],
    ) {
        return None;
    }
    Some(PendingRepayment {
        receiver: underlying_var(recv)?,
        token: underlying_var(args[1])?,
        amount: underlying_var(args[2])?,
        fee: underlying_var(args[3])?,
    })
}

/// True when `expr` is `amount + fee` or `fee + amount`, parens-tolerant.
fn is_amount_plus_fee(expr: &hir::Expr<'_>, amount: hir::VariableId, fee: hir::VariableId) -> bool {
    let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind else { return false };
    if !matches!(op.kind, ast::BinOpKind::Add) {
        return false;
    }
    let a = underlying_var(lhs);
    let b = underlying_var(rhs);
    (a == Some(amount) && b == Some(fee)) || (a == Some(fee) && b == Some(amount))
}

/// Matches an ERC20-like transfer sink. Returns `(from_arg, token_var)` where `token_var` is
/// the receiver's underlying variable id when available (used for permit correlation).
///
/// Recognised:
/// - `recv.transferFrom(from, to, amt)` / `recv.safeTransferFrom(from, to, amt)` where `recv` is
///   typed as a contract declaring ERC20's `transferFrom(address,address,uint256)→bool` (ERC721's
///   same-named, no-return overload is excluded).
/// - `Lib.safeTransferFrom(token, from, to, amt)` library form.
fn match_sink<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    has_solady_lib: bool,
    expr: &'hir hir::Expr<'hir>,
) -> Option<(&'hir hir::Expr<'hir>, Option<TokenKey>)> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let ExprKind::Member(recv, ident) = &callee.peel_parens().kind else { return None };
    let name = ident.name.as_str();

    if (name == "transferFrom" || name == "safeTransferFrom")
        && let Some(canonical) =
            canonical_args(args.kind, &[&["from"], &["to"], &["value", "amount"]])
    {
        // Contract-typed receiver: must actually declare ERC20's `transferFrom`.
        if let Some(cid) = receiver_contract_id(gcx, recv)
            && contract_has_function(
                hir,
                cid,
                "transferFrom",
                &["address", "address", "uint256"],
                &["bool"],
            )
        {
            return Some((canonical[0], token_key(recv)));
        }
        // `addr.safeTransferFrom(...)` via `using SafeTransferLib for address`. HIR doesn't
        // expose `using-for` bindings, so we proxy by requiring `SafeTransferLib` to be
        // declared in the compiled sources.
        if name == "safeTransferFrom" && has_solady_lib && expr_is_address(gcx, recv) {
            return Some((canonical[0], token_key(recv)));
        }
    }

    if name == "safeTransferFrom"
        && let Some(canonical) =
            canonical_args(args.kind, &[&["token"], &["from"], &["to"], &["value", "amount"]])
        && let Some(cid) = receiver_contract_id(gcx, recv)
        && hir.contract(cid).kind == ContractKind::Library
        && library_has_safe_transfer_from(hir, cid)
    {
        return Some((canonical[1], token_key(canonical[0])));
    }

    None
}

/// Constructs a `TokenKey` for the receiver of a permit/sink call. Supports
/// `<var>` (with cast / `payable` peeling via `underlying_var`) and
/// `<var>.<field>` (a struct-field path).
fn token_key(expr: &hir::Expr<'_>) -> Option<TokenKey> {
    if let Some(v) = underlying_var(expr) {
        return Some(TokenKey::Var(v));
    }
    let expr = expr.peel_parens();
    if let ExprKind::Member(base, ident) = &expr.kind
        && let Some(v) = underlying_var(base)
    {
        return Some(TokenKey::Field(v, ident.name));
    }
    None
}

/// Hoists `require(modParam == msg.sender | address(this))` guards from the modifier
/// prefix (statements before `_;`) to the caller's argument. `out_self` receives params
/// proven equal to `address(this)`.
fn collect_modifier_safety<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    has_solady_lib: bool,
    invocation: &hir::Modifier<'hir>,
    out_safe: &mut HashSet<hir::VariableId>,
    out_self: &mut HashSet<hir::VariableId>,
) {
    let ItemId::Function(fid) = invocation.id else { return };
    let modifier = hir.function(fid);
    if !matches!(modifier.kind, hir::FunctionKind::Modifier) {
        return;
    }
    let Some(body) = modifier.body else { return };

    // Skip multi-`_` or nested-placeholder modifiers — guard ordering can't be assumed sound.
    if count_placeholders(body.stmts) != 1 {
        return;
    }
    let Some(placeholder_idx) =
        body.stmts.iter().position(|s| matches!(s.kind, StmtKind::Placeholder))
    else {
        return;
    };

    // Modifier-param → caller-side var. Supports positional and named call shapes.
    let arg_map: Vec<(hir::VariableId, hir::VariableId)> = match invocation.args.kind {
        hir::CallArgsKind::Unnamed(exprs) => exprs
            .iter()
            .enumerate()
            .filter_map(|(i, arg)| Some((*modifier.parameters.get(i)?, underlying_var(arg)?)))
            .collect(),
        hir::CallArgsKind::Named(named) => named
            .iter()
            .filter_map(|na| {
                let mp = modifier.parameters.iter().find(|p| {
                    hir.variable(**p).name.is_some_and(|n| n.as_str() == na.name.as_str())
                })?;
                Some((*mp, underlying_var(&na.value)?))
            })
            .collect(),
    };
    if arg_map.is_empty() {
        return;
    }

    // Bail when the prefix contains stmts we can't track writes through (e.g. inline
    // assembly, which currently lowers to `StmtKind::Err`).
    if contains_unanalysable(&body.stmts[..placeholder_idx]) {
        return;
    }

    let mut a = Analyzer::new(gcx, hir, has_solady_lib);
    for stmt in &body.stmts[..placeholder_idx] {
        let _ = a.visit_stmt(stmt);
    }
    // Skip mutable-storage callers (no safe-fact) and params written inside the modifier
    // (the local-copy fact doesn't apply to the caller's var).
    for (mp, caller) in arg_map {
        if !a.is_safe_target(caller) || a.written.contains(&mp) {
            continue;
        }
        if a.safe_vars.contains(&mp) {
            out_safe.insert(caller);
        }
        if a.self_vars.contains(&mp) {
            out_self.insert(caller);
        }
    }
}

/// Harvests `self_vars` / `safe_vars` facts about immutable / constant state vars
/// of `cid`: both declaration initializers and direct constructor assignments.
fn seed_immutable_facts<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    has_solady_lib: bool,
    cid: hir::ContractId,
    out: &mut Analyzer<'hir>,
) {
    for item in hir.contract_item_ids(cid) {
        if let Some(vid) = item.as_variable() {
            let v = hir.variable(vid);
            if v.kind.is_state()
                && (v.is_immutable() || v.is_constant())
                && let Some(init) = v.initializer
            {
                if out.is_safe(init) {
                    out.safe_vars.insert(vid);
                }
                if out.is_self_expr(init) {
                    out.self_vars.insert(vid);
                }
            }
        }
    }
    for item in hir.contract_item_ids(cid) {
        let Some(fid) = item.as_function() else { continue };
        let f = hir.function(fid);
        if !f.is_constructor() {
            continue;
        }
        let Some(body) = f.body else { continue };
        let mut ctor = Analyzer::new(gcx, hir, has_solady_lib);
        for stmt in body.stmts {
            let _ = ctor.visit_stmt(stmt);
            if branch_always_exits(stmt) {
                break;
            }
        }
        for v in &ctor.safe_vars {
            let var = hir.variable(*v);
            if var.kind.is_state() && (var.is_immutable() || var.is_constant()) {
                out.safe_vars.insert(*v);
            }
        }
        for v in &ctor.self_vars {
            let var = hir.variable(*v);
            if var.kind.is_state() && (var.is_immutable() || var.is_constant()) {
                out.self_vars.insert(*v);
            }
        }
    }
}

fn seed_internal_callsite_facts<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    has_solady_lib: bool,
    func: &'hir hir::Function<'hir>,
    out: &mut Analyzer<'hir>,
) {
    if !is_internal_callsite_seed_candidate(func) {
        return;
    }

    let index = project_index_for(gcx, hir, has_solady_lib);
    let ptr = std::ptr::from_ref::<hir::Function<'_>>(func) as usize;
    let Some(fid) = index.function_ids_by_ptr.get(&ptr).copied() else { return };
    let Some(facts) = index.internal_callsites.get(&fid) else { return };
    if facts.unknown {
        return;
    }

    for (i, param) in func.parameters.iter().copied().enumerate() {
        if facts.seen.get(i).copied().unwrap_or(false)
            && facts.all_safe.get(i).copied().unwrap_or(false)
        {
            out.safe_vars.insert(param);
            if facts.all_self.get(i).copied().unwrap_or(false) {
                out.self_vars.insert(param);
            }
        }
    }
}

const fn is_internal_callsite_seed_candidate(func: &hir::Function<'_>) -> bool {
    func.kind.is_function()
        && matches!(func.visibility, ast::Visibility::Private | ast::Visibility::Internal)
        && !func.parameters.is_empty()
}

thread_local! {
    static PROJECT_INDEX: RefCell<Option<(usize, Rc<ProjectIndex>)>> = const { RefCell::new(None) };
}

fn project_index_for<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    has_solady_lib: bool,
) -> Rc<ProjectIndex> {
    let key = std::ptr::from_ref::<hir::Hir<'_>>(hir) as usize;
    PROJECT_INDEX.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some((cached_key, cached)) = slot.as_ref()
            && *cached_key == key
        {
            return cached.clone();
        }
        let fresh = Rc::new(build_project_index(gcx, hir, has_solady_lib));
        *slot = Some((key, fresh.clone()));
        fresh
    })
}

fn build_project_index<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    has_solady_lib: bool,
) -> ProjectIndex {
    let mut index = ProjectIndex::default();
    for fid in hir.function_ids() {
        let func = hir.function(fid);
        index
            .function_ids_by_ptr
            .insert(std::ptr::from_ref::<hir::Function<'_>>(func) as usize, fid);
    }

    let mut collector =
        InternalCallsiteCollector { gcx, hir, has_solady_lib, out: &mut index.internal_callsites };
    for fid in hir.function_ids() {
        let Some(body) = hir.function(fid).body else { continue };
        for stmt in body.stmts {
            let _ = collector.visit_stmt(stmt);
        }
    }
    index
}

struct InternalCallsiteCollector<'a, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    has_solady_lib: bool,
    out: &'a mut HashMap<hir::FunctionId, ParamCallsiteFacts>,
}

impl<'hir> hir::Visit<'hir> for InternalCallsiteCollector<'_, 'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Call(callee, args, _) = &expr.kind
            && let Some(fid) = resolve_internal_fn(callee)
        {
            self.record_call(fid, args);
        }
        self.walk_expr(expr)
    }
}

impl<'hir> InternalCallsiteCollector<'_, 'hir> {
    fn record_call(&mut self, fid: hir::FunctionId, args: &'hir hir::CallArgs<'hir>) {
        let func = self.hir.function(fid);
        if !is_internal_callsite_seed_candidate(func) {
            return;
        }

        let arity = func.parameters.len();
        let facts = self.out.entry(fid).or_insert_with(|| ParamCallsiteFacts::new(arity));
        if facts.unknown {
            return;
        }
        let Some(call_args) = internal_call_args(self.hir, func, args) else {
            facts.unknown = true;
            return;
        };
        if call_args.len() != arity || facts.seen.len() != arity {
            facts.unknown = true;
            return;
        }

        let analyzer = Analyzer::new(self.gcx, self.hir, self.has_solady_lib);
        for (i, arg) in call_args.into_iter().enumerate() {
            facts.seen[i] = true;
            facts.all_safe[i] &= analyzer.is_safe(arg);
            facts.all_self[i] &= analyzer.is_self_expr(arg);
        }
    }
}

fn internal_call_args<'hir>(
    hir: &'hir hir::Hir<'hir>,
    func: &'hir hir::Function<'hir>,
    args: &'hir hir::CallArgs<'hir>,
) -> Option<Vec<&'hir hir::Expr<'hir>>> {
    match args.kind {
        hir::CallArgsKind::Unnamed(exprs) => {
            (exprs.len() == func.parameters.len()).then(|| exprs.iter().collect())
        }
        hir::CallArgsKind::Named(named) => {
            if named.len() != func.parameters.len() {
                return None;
            }
            func.parameters
                .iter()
                .map(|param| {
                    let name = hir.variable(*param).name?;
                    named
                        .iter()
                        .find_map(|arg| (arg.name.as_str() == name.as_str()).then_some(&arg.value))
                })
                .collect()
        }
    }
}

/// Collects state-variable assignments / `delete`s reached from the given stmts.
/// Single-level: does not follow nested function calls.
struct StateWriteCollector<'hir> {
    hir: &'hir hir::Hir<'hir>,
    out: HashSet<hir::VariableId>,
}

impl<'hir> hir::Visit<'hir> for StateWriteCollector<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            ExprKind::Assign(lhs, _, _) => self.add_target(lhs),
            ExprKind::Delete(e) => self.add_target(e),
            _ => {}
        }
        self.walk_expr(expr)
    }
}

impl<'hir> StateWriteCollector<'hir> {
    fn add_target(&mut self, lhs: &hir::Expr<'_>) {
        let lhs = lhs.peel_parens();
        if let ExprKind::Tuple(elems) = &lhs.kind {
            for e in elems.iter().flatten() {
                self.add_target(e);
            }
            return;
        }
        if let Some(vid) = underlying_var(lhs) {
            let v = self.hir.variable(vid);
            if v.kind.is_state() {
                self.out.insert(vid);
            }
        }
    }
}

fn collect_state_writes<'hir>(
    hir: &'hir hir::Hir<'hir>,
    stmts: &'hir [hir::Stmt<'hir>],
) -> HashSet<hir::VariableId> {
    let mut c = StateWriteCollector { hir, out: HashSet::new() };
    for s in stmts {
        let _ = c.visit_stmt(s);
    }
    c.out
}

/// Resolves a call's callee to a `FunctionId` for plain `name()` / `this.name()`
/// patterns inside the same contract. Returns `None` for external / library /
/// member-of-state-var / unresolved calls.
fn resolve_internal_fn(callee: &hir::Expr<'_>) -> Option<hir::FunctionId> {
    let callee = callee.peel_parens();
    let reses: &[Res] = match &callee.kind {
        ExprKind::Ident(reses) => reses,
        ExprKind::Member(recv, _) => match &recv.peel_parens().kind {
            ExprKind::Ident(reses) => reses,
            _ => return None,
        },
        _ => return None,
    };
    reses.iter().find_map(|r| match r {
        Res::Item(ItemId::Function(fid)) => Some(*fid),
        _ => None,
    })
}

/// Walks an expression tree and records every internal callee resolvable to a
/// `FunctionId`. Used to widen `collect_state_writes` by one call level.
struct NestedCallCollector<'hir> {
    hir: &'hir hir::Hir<'hir>,
    out: HashSet<hir::FunctionId>,
}

impl<'hir> hir::Visit<'hir> for NestedCallCollector<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Call(callee, ..) = &expr.kind
            && let Some(fid) = resolve_internal_fn(callee)
        {
            self.out.insert(fid);
        }
        self.walk_expr(expr)
    }
}

/// `true` when the modifier body has a single `_;` and any preceding statement
/// definitely exits (making the calling function's body unreachable).
fn modifier_prefix_always_exits(hir: &hir::Hir<'_>, invocation: &hir::Modifier<'_>) -> bool {
    let ItemId::Function(fid) = invocation.id else { return false };
    let modifier = hir.function(fid);
    if !matches!(modifier.kind, hir::FunctionKind::Modifier) {
        return false;
    }
    let Some(body) = modifier.body else { return false };
    if count_placeholders(body.stmts) != 1 {
        return false;
    }
    let Some(placeholder_idx) =
        body.stmts.iter().position(|s| matches!(s.kind, StmtKind::Placeholder))
    else {
        return false;
    };
    body.stmts[..placeholder_idx].iter().any(branch_always_exits)
}

/// `true` if any statement is `StmtKind::Err` (currently catches inline assembly,
/// which solar doesn't yet lower to HIR).
fn contains_unanalysable(stmts: &[hir::Stmt<'_>]) -> bool {
    fn in_stmt(s: &hir::Stmt<'_>) -> bool {
        match &s.kind {
            StmtKind::Err(_) => true,
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => contains_unanalysable(b.stmts),
            StmtKind::If(_, t, e) => in_stmt(t) || e.as_ref().is_some_and(|s| in_stmt(s)),
            StmtKind::Loop(b, _) => contains_unanalysable(b.stmts),
            StmtKind::Try(t) => t.clauses.iter().any(|c| contains_unanalysable(c.block.stmts)),
            _ => false,
        }
    }
    stmts.iter().any(in_stmt)
}

/// Strips the synthesized trailing `if (!cond) break;` from the HIR `do-while` lowering.
fn do_while_user_stmts<'a, 'hir>(stmts: &'a [hir::Stmt<'hir>]) -> &'a [hir::Stmt<'hir>] {
    match stmts.split_last() {
        Some((last, rest)) if is_loop_termination_if(last) => rest,
        _ => stmts,
    }
}

fn is_loop_termination_if(stmt: &hir::Stmt<'_>) -> bool {
    let StmtKind::If(_, then_, else_) = &stmt.kind else { return false };
    is_break_stmt(then_) || else_.as_ref().is_some_and(|e| is_break_stmt(e))
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

/// `break`/`continue` targeting the current loop (nested loops shadow them).
fn body_has_break_or_continue(stmts: &[hir::Stmt<'_>]) -> bool {
    fn in_stmt(stmt: &hir::Stmt<'_>) -> bool {
        match &stmt.kind {
            StmtKind::Break | StmtKind::Continue => true,
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => body_has_break_or_continue(b.stmts),
            StmtKind::If(_, t, e) => in_stmt(t) || e.as_ref().is_some_and(|s| in_stmt(s)),
            StmtKind::Try(t) => t.clauses.iter().any(|c| body_has_break_or_continue(c.block.stmts)),
            StmtKind::Loop(..) => false,
            _ => false,
        }
    }
    stmts.iter().any(in_stmt)
}

fn count_placeholders(stmts: &[hir::Stmt<'_>]) -> usize {
    fn count_in_stmt(stmt: &hir::Stmt<'_>) -> usize {
        match &stmt.kind {
            StmtKind::Placeholder => 1,
            StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => count_placeholders(b.stmts),
            StmtKind::If(_, t, e) => count_in_stmt(t) + e.as_ref().map_or(0, |s| count_in_stmt(s)),
            StmtKind::Loop(b, _) => count_placeholders(b.stmts),
            StmtKind::Try(t) => t.clauses.iter().map(|c| count_placeholders(c.block.stmts)).sum(),
            _ => 0,
        }
    }
    stmts.iter().map(count_in_stmt).sum()
}

/// Resolves a `VariableId` for bare idents and type-cast / `payable(...)` / parens wraps.
///
/// Strips elementary-type casts (e.g. `address(x)`), contract / interface casts
/// (e.g. `IERC20(rawToken)` — encoded as `Call` with the contract ident as callee), and
/// `payable(...)` wrappers. Stripping interface casts lets permit and sink correlate when
/// both sides wrap the same underlying raw-address variable.
fn underlying_var(expr: &hir::Expr<'_>) -> Option<hir::VariableId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Variable(vid)) => Some(*vid),
            _ => None,
        }),
        ExprKind::Call(callee, args, _) if is_cast_callee(callee) => {
            // Type conversions are unary; bail out on anything else to keep this peel
            // strictly a cast-stripping helper.
            let mut exprs = args.exprs();
            let inner = exprs.next()?;
            if exprs.next().is_some() {
                return None;
            }
            underlying_var(inner)
        }
        ExprKind::Payable(inner) => underlying_var(inner),
        _ => None,
    }
}

/// `true` when `callee` is a type-cast head, i.e. `address(...)`, an elementary-type cast,
/// or an interface/contract cast like `IERC20(...)`.
fn is_cast_callee(callee: &hir::Expr<'_>) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Type(_) => true,
        ExprKind::Ident(reses) => reses.iter().any(|r| matches!(r, Res::Item(ItemId::Contract(_)))),
        _ => false,
    }
}

/// Resolves the static contract type of `recv`: a contract-typed expression, a direct contract
/// reference (e.g. a library), or an interface/contract cast.
fn receiver_contract_id<'hir>(gcx: Gcx<'hir>, recv: &hir::Expr<'hir>) -> Option<hir::ContractId> {
    expr_contract_id(gcx, recv).or_else(|| direct_contract_id(recv))
}

fn expr_contract_id<'hir>(gcx: Gcx<'hir>, expr: &hir::Expr<'hir>) -> Option<hir::ContractId> {
    expr_ty(gcx, expr).and_then(ty_contract_id)
}

fn ty_contract_id(ty: Ty<'_>) -> Option<hir::ContractId> {
    match ty.peel_refs().kind {
        TyKind::Contract(id) => Some(id),
        TyKind::Type(ty) => ty_contract_id(ty),
        _ => None,
    }
}

fn direct_contract_id(expr: &hir::Expr<'_>) -> Option<hir::ContractId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Contract(cid)) => Some(*cid),
            _ => None,
        }),
        ExprKind::Call(callee, ..) => direct_contract_id(callee),
        _ => None,
    }
}

/// Whether the sources declare a Solady-shaped `SafeTransferLib` library.
fn has_solady_safe_transfer_lib(hir: &hir::Hir<'_>) -> bool {
    hir.contracts_enumerated().any(|(cid, c)| {
        c.kind == ContractKind::Library
            && c.name.as_str() == "SafeTransferLib"
            && library_has_safe_transfer_from(hir, cid)
    })
}

fn contract_has_function(
    hir: &hir::Hir<'_>,
    cid: hir::ContractId,
    name: &str,
    params: &[&str],
    returns: &[&str],
) -> bool {
    hir.contract_item_ids(cid).any(|item| {
        let Some(fid) = item.as_function() else { return false };
        let f = hir.function(fid);
        f.name.is_some_and(|n| n.name.as_str() == name)
            && f.parameters.len() == params.len()
            && f.returns.len() == returns.len()
            && f.parameters.iter().zip(params).all(|(id, abi)| is_elementary(hir, *id, abi))
            && f.returns.iter().zip(returns).all(|(id, abi)| is_elementary(hir, *id, abi))
    })
}

/// 4-arg `safeTransferFrom(token, address, address, uint256)`. `token` is either `address`
/// (Solady) or a contract declaring ERC20's `transferFrom(...)→bool` (OZ SafeERC20);
/// ERC721/1155 helpers are excluded since their `transferFrom` has no return.
fn library_has_safe_transfer_from(hir: &hir::Hir<'_>, cid: hir::ContractId) -> bool {
    hir.contract_item_ids(cid).any(|item| {
        let Some(fid) = item.as_function() else { return false };
        let f = hir.function(fid);
        if f.parameters.len() != 4 || f.name.is_none_or(|n| n.name.as_str() != "safeTransferFrom") {
            return false;
        }
        let token_ok = match hir.variable(f.parameters[0]).ty.kind {
            TypeKind::Elementary(ElementaryType::Address(_)) => true,
            TypeKind::Custom(ItemId::Contract(token_cid)) => contract_has_function(
                hir,
                token_cid,
                "transferFrom",
                &["address", "address", "uint256"],
                &["bool"],
            ),
            _ => false,
        };
        token_ok
            && is_address(hir, f.parameters[1])
            && is_address(hir, f.parameters[2])
            && is_elementary(hir, f.parameters[3], "uint256")
    })
}

fn is_address(hir: &hir::Hir<'_>, id: hir::VariableId) -> bool {
    matches!(hir.variable(id).ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
}

/// True when `expr`'s static type is `address` / `address payable`.
fn expr_is_address<'hir>(gcx: Gcx<'hir>, expr: &hir::Expr<'hir>) -> bool {
    expr_ty(gcx, expr).is_some_and(ty_is_address)
}

fn expr_ty<'hir>(gcx: Gcx<'hir>, expr: &hir::Expr<'hir>) -> Option<Ty<'hir>> {
    gcx.type_of_expr(expr.peel_parens().id)
}

fn ty_is_address(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::Address(_)))
}

fn is_elementary(hir: &hir::Hir<'_>, id: hir::VariableId, abi: &str) -> bool {
    matches!(&hir.variable(id).ty.kind, TypeKind::Elementary(ty) if ty.to_abi_str() == abi)
}

/// Resolves positional or named call args to a fixed positional ordering. `aliases[i]`
/// holds the parameter names accepted for slot `i` in the named form. Returns `None` if
/// arity differs or any slot is unmatched.
fn canonical_args<'hir>(
    kind: hir::CallArgsKind<'hir>,
    aliases: &[&[&str]],
) -> Option<Vec<&'hir hir::Expr<'hir>>> {
    match kind {
        hir::CallArgsKind::Unnamed(exprs) => {
            (exprs.len() == aliases.len()).then(|| exprs.iter().collect())
        }
        hir::CallArgsKind::Named(named) => {
            if named.len() != aliases.len() {
                return None;
            }
            aliases
                .iter()
                .map(|accepted| {
                    named.iter().find_map(|a| {
                        accepted.iter().any(|n| a.name.as_str() == *n).then_some(&a.value)
                    })
                })
                .collect()
        }
    }
}

fn is_address_cast(callee: &hir::Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ElementaryType::Address(_)), .. })
    )
}

fn is_require_or_assert(callee: &hir::Expr<'_>) -> bool {
    let ExprKind::Ident(reses) = &callee.kind else { return false };
    reses.iter().any(
        |r| matches!(r, Res::Builtin(b) if b.name() == sym::require || b.name() == sym::assert),
    )
}

/// `address(this)` or bare `this`, including through `payable(...)` and parens wraps.
fn is_address_self(expr: &hir::Expr<'_>) -> bool {
    let expr = expr.peel_parens();
    if is_builtin(expr, sym::this) {
        return true;
    }
    if let ExprKind::Payable(inner) = &expr.kind {
        return is_address_self(inner);
    }
    matches!(&expr.kind, ExprKind::Call(callee, args, _) if is_address_cast(callee)
        && args.exprs().next().is_some_and(is_address_self))
}

fn is_builtin(expr: &hir::Expr<'_>, name: solar::interface::Symbol) -> bool {
    let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return false };
    reses.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == name))
}

/// `return`, custom-error `revert`, `revert(...)`, or `assert(false)` / `require(false, ...)`.
fn branch_always_exits(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Revert(_) => true,
        StmtKind::Expr(expr) => is_exit_call(expr),
        // Any sequential definite-exit makes the block exit.
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => b.stmts.iter().any(branch_always_exits),
        StmtKind::If(_, t, Some(e)) => branch_always_exits(t) && branch_always_exits(e),
        // `do-while` runs the body once: if it can't break/continue out and any stmt
        // definitely exits, the loop does too.
        StmtKind::Loop(block, LoopSource::DoWhile) => {
            let user = do_while_user_stmts(block.stmts);
            !body_has_break_or_continue(user) && user.iter().any(branch_always_exits)
        }
        // `try { ... } catch { ... }` exits the enclosing block only when every clause
        // (success and all catches) definitely exits.
        StmtKind::Try(t) => t.clauses.iter().all(|c| c.block.stmts.iter().any(branch_always_exits)),
        _ => false,
    }
}

fn is_exit_call(expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return false };
    if is_builtin(callee, kw::Revert) {
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
