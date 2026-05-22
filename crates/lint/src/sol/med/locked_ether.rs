use super::LockedEther;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, ElementaryType, LitKind, StateMutability, Visibility},
    interface::{Symbol, kw, sym},
    sema::{
        builtins::Builtin,
        hir::{
            self, CallArgs, CallArgsKind, ExprKind, FunctionId, FunctionKind, ItemId, Res,
            StmtKind, TypeKind, VariableId, Visit as _,
        },
    },
};
use std::{collections::HashSet, fmt::Write as _, ops::ControlFlow};

declare_forge_lint!(
    LOCKED_ETHER,
    Severity::Med,
    "locked-ether",
    "contract can receive ETH but has no mechanism to send it out"
);

impl<'hir> LateLintPass<'hir> for LockedEther {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        contract_id: hir::ContractId,
    ) {
        if !ctx.is_lint_enabled(LOCKED_ETHER.id) {
            return;
        }

        let contract = hir.contract(contract_id);

        // Libraries and interfaces cannot hold ETH.
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract) {
            return;
        }
        if contract.linearization_failed() {
            return;
        }

        // Effective dispatch surface: most-derived implementation per signature,
        // plus the most-derived `receive`/`fallback`. Constructors are excluded.
        let runtime_entries = effective_runtime_dispatch_surface(hir, contract.linearized_bases);

        // Inflow channels are tracked independently: a runtime payable entry vs. a
        // payable constructor. Constructor inflows are deployer-controlled and have
        // no runtime exit path, so they must not be conflated with runtime ETH flow.
        let has_runtime_inflow = runtime_entries.iter().any(|&fid| {
            let f = hir.function(fid);
            f.state_mutability == StateMutability::Payable && !function_always_reverts(hir, f)
        });
        // Only the leaf contract's own constructor receives deployment value; a
        // non-payable derived ctor rejects ETH regardless of any payable base ctor.
        let has_ctor_inflow = contract.ctor.is_some_and(|fid| {
            let f = hir.function(fid);
            f.state_mutability == StateMutability::Payable && !function_always_reverts(hir, f)
        });
        if !has_runtime_inflow && !has_ctor_inflow {
            return;
        }

        // Seed runtime entries only; internal helpers are reached transitively by
        // `SendChecker`. Constructor bodies are excluded so their exits don't count.
        let mut visited: HashSet<FunctionId> = HashSet::new();
        let mut worklist: Vec<FunctionId> = runtime_entries;

        while let Some(fid) = worklist.pop() {
            if !visited.insert(fid) {
                continue;
            }
            let func = hir.function(fid);
            // Any ETH movement inside an always-reverting function rolls back, so it
            // cannot exfiltrate funds. Skip its body and modifier args entirely.
            if function_always_reverts(hir, func) {
                continue;
            }
            // Contract that defines the function being visited; used to resolve `super`.
            let call_site = func.contract;

            for modifier in func.modifiers {
                for arg in modifier.args.exprs() {
                    let mut checker = SendChecker {
                        hir,
                        bases: contract.linearized_bases,
                        call_site,
                        worklist: &mut worklist,
                        visited: &visited,
                    };
                    if checker.visit_expr(arg).is_break() {
                        return;
                    }
                }
                if let Some(modifier_fid) = modifier.id.as_function() {
                    worklist.push(modifier_fid);
                }
            }

            if let Some(body) = func.body {
                let mut checker = SendChecker {
                    hir,
                    bases: contract.linearized_bases,
                    call_site,
                    worklist: &mut worklist,
                    visited: &visited,
                };
                for stmt in body.stmts {
                    if checker.visit_stmt(stmt).is_break() {
                        return;
                    }
                }
            }
        }

        ctx.emit(&LOCKED_ETHER, contract.name.span);
    }
}

/// Returns `true` if invoking `func` always reverts, either via its body or an attached modifier.
fn function_always_reverts(hir: &hir::Hir<'_>, func: &hir::Function<'_>) -> bool {
    if func
        .modifiers
        .iter()
        .any(|m| m.id.as_function().is_some_and(|mid| modifier_always_reverts(hir.function(mid))))
    {
        return true;
    }
    func.body.is_some_and(|body| stmts_always_revert(body.stmts))
}

/// Returns `true` if the modifier always reverts: before the first `_`, or after the last one.
fn modifier_always_reverts(modifier: &hir::Function<'_>) -> bool {
    let Some(body) = modifier.body else { return false };
    let Some(first) = body.stmts.iter().position(|s| matches!(s.kind, StmtKind::Placeholder))
    else {
        return stmts_always_revert(body.stmts);
    };
    let last = body.stmts.iter().rposition(|s| matches!(s.kind, StmtKind::Placeholder)).unwrap();
    stmts_always_revert(&body.stmts[..first]) || stmts_always_revert(&body.stmts[last + 1..])
}

/// What execution of a statement (or list) may do. A block "always reverts" iff its
/// outcome set is exactly `REVERT` — no `FALLTHROUGH` and no `NON_REVERT_EXIT`.
const REVERT: u8 = 1 << 0;
const NON_REVERT_EXIT: u8 = 1 << 1;
const FALLTHROUGH: u8 = 1 << 2;

fn stmts_always_revert(stmts: &[hir::Stmt<'_>]) -> bool {
    stmts_outcomes(stmts) == REVERT
}

/// Walks `stmts` left-to-right. Each statement's outcome set replaces the prior
/// `FALLTHROUGH` bit (since we only reach the next stmt by falling through). We stop as
/// soon as a stmt cannot fall through, because nothing after it is reachable.
fn stmts_outcomes(stmts: &[hir::Stmt<'_>]) -> u8 {
    let mut acc = FALLTHROUGH;
    for stmt in stmts {
        let o = stmt_outcomes(stmt);
        acc = (acc & !FALLTHROUGH) | o;
        if o & FALLTHROUGH == 0 {
            break;
        }
    }
    acc
}

fn stmt_outcomes(stmt: &hir::Stmt<'_>) -> u8 {
    match &stmt.kind {
        StmtKind::Revert(_) => REVERT,
        StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue => NON_REVERT_EXIT,
        StmtKind::Expr(expr) if is_unconditional_revert_call(expr) => REVERT,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => stmts_outcomes(block.stmts),
        // `if` without `else`: the missing branch falls through.
        StmtKind::If(_, t, None) => stmt_outcomes(t) | FALLTHROUGH,
        StmtKind::If(_, t, Some(e)) => stmt_outcomes(t) | stmt_outcomes(e),
        // Loops, try, decls, emits, unknowns: assume control may continue past them.
        _ => FALLTHROUGH,
    }
}

/// Matches `revert()`/`revert("msg")`, `require(false[, "msg"])`, and `assert(false)`.
fn is_unconditional_revert_call(expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return false };
    let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return false };
    reses.iter().any(|r| match r {
        Res::Builtin(Builtin::Revert | Builtin::RevertMsg) => true,
        Res::Builtin(Builtin::Require | Builtin::RequireMsg | Builtin::Assert) => {
            args.exprs().next().is_some_and(is_literal_false)
        }
        _ => false,
    })
}

fn is_literal_false(expr: &hir::Expr<'_>) -> bool {
    if let ExprKind::Lit(lit) = &expr.peel_parens().kind
        && let LitKind::Bool(b) = &lit.kind
    {
        return !b;
    }
    false
}

/// Runtime entry points reachable on the deployed contract: the most-derived
/// implementation of each `(name, parameter signature)` plus the most-derived
/// `receive` / `fallback`. `bases` must be the C3 linearization (leaf first).
/// Later entries with the same key are overridden and dropped. Constructors and
/// modifiers are excluded.
fn effective_runtime_dispatch_surface<'hir>(
    hir: &'hir hir::Hir<'hir>,
    bases: &[hir::ContractId],
) -> Vec<FunctionId> {
    let mut seen_funcs: HashSet<(Symbol, String)> = HashSet::new();
    let mut seen_receive = false;
    let mut seen_fallback = false;
    let mut out: Vec<FunctionId> = Vec::new();
    for &cid in bases {
        for fid in hir.contract(cid).all_functions() {
            let f = hir.function(fid);
            match f.kind {
                FunctionKind::Function => {
                    if !matches!(f.visibility, Visibility::Public | Visibility::External) {
                        continue;
                    }
                    let Some(name) = f.name else { continue };
                    let sig = parameter_signature(hir, f.parameters);
                    if seen_funcs.insert((name.name, sig)) {
                        out.push(fid);
                    }
                }
                FunctionKind::Receive => {
                    if !seen_receive {
                        seen_receive = true;
                        out.push(fid);
                    }
                }
                FunctionKind::Fallback => {
                    if !seen_fallback {
                        seen_fallback = true;
                        out.push(fid);
                    }
                }
                FunctionKind::Constructor | FunctionKind::Modifier => {}
            }
        }
    }
    out
}

/// Structural string for a parameter list, used as a hash key to dedup
/// overloads across the inheritance chain.
fn parameter_signature(hir: &hir::Hir<'_>, params: &[VariableId]) -> String {
    let mut s = String::new();
    for (i, &p) in params.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        write_type_signature(&hir.variable(p).ty.kind, &mut s);
    }
    s
}

fn write_type_signature(ty: &TypeKind<'_>, out: &mut String) {
    match ty {
        TypeKind::Elementary(e) => write!(out, "{e:?}").unwrap(),
        TypeKind::Array(a) => {
            write_type_signature(&a.element.kind, out);
            out.push_str("[]");
        }
        TypeKind::Function(_) => out.push_str("fn"),
        TypeKind::Mapping(m) => {
            out.push_str("map(");
            write_type_signature(&m.key.kind, out);
            out.push(',');
            write_type_signature(&m.value.kind, out);
            out.push(')');
        }
        TypeKind::Custom(id) => write!(out, "{id:?}").unwrap(),
        TypeKind::Err(_) => out.push('?'),
    }
}

/// HIR visitor that short-circuits on the first ETH-sending expression and queues
/// internally-resolved callees for transitive exploration by the outer worklist loop.
struct SendChecker<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    /// Linearization of the contract being linted; used to resolve `this`.
    bases: &'a [hir::ContractId],
    /// Contract that defines the function whose body is being visited; used to resolve
    /// `super`. `None` for free functions.
    call_site: Option<hir::ContractId>,
    worklist: &'a mut Vec<FunctionId>,
    visited: &'a HashSet<FunctionId>,
}

impl<'hir> SendChecker<'_, 'hir> {
    /// Queues the overload of `member` actually invoked on `receiver`.
    fn queue_member_callee(
        &mut self,
        receiver: &hir::Expr<'_>,
        member: solar::interface::Ident,
        args: &CallArgs<'_>,
    ) {
        let ExprKind::Ident(reses) = &receiver.peel_parens().kind else { return };
        for res in *reses {
            match res {
                Res::Builtin(Builtin::Super) => {
                    // Resolve `super` against the call-site contract's own linearization,
                    // skipping the call-site contract itself.
                    if let Some(cid) = self.call_site {
                        let cs = self.hir.contract(cid);
                        if !cs.linearization_failed() && cs.linearized_bases.len() > 1 {
                            self.queue_resolved(&cs.linearized_bases[1..], member.name, args);
                        }
                    }
                }
                Res::Builtin(Builtin::This) => {
                    self.queue_resolved(self.bases, member.name, args);
                }
                Res::Item(ItemId::Contract(cid)) => {
                    self.queue_resolved(std::slice::from_ref(cid), member.name, args);
                }
                _ => {}
            }
        }
    }

    /// Redirects an unqualified internal call resolved to `fid` to the leaf contract's
    /// most-derived override of the same `(name, parameter signature)`. If `fid` is not
    /// inheritable from the linted contract (free function, library helper, private,
    /// constructor/modifier), it is returned as-is.
    fn resolve_virtual(&self, fid: FunctionId, args: &CallArgs<'_>) -> FunctionId {
        let func = self.hir.function(fid);
        let Some(origin) = func.contract else { return fid };
        if !self.bases.contains(&origin)
            || func.visibility == Visibility::Private
            || !matches!(func.kind, FunctionKind::Function)
        {
            return fid;
        }
        let Some(name) = func.name else { return fid };
        let sig = parameter_signature(self.hir, func.parameters);
        for &cid in self.bases {
            for cand in self.hir.contract(cid).all_functions() {
                let c = self.hir.function(cand);
                if matches!(c.kind, FunctionKind::Function)
                    && c.name.is_some_and(|n| n.name == name.name)
                    && parameter_signature(self.hir, c.parameters) == sig
                    && args_match(self.hir, args, c.parameters)
                {
                    return cand;
                }
            }
        }
        fid
    }

    /// Queues arity-matching overloads of `name` from the most-derived contract that defines any.
    fn queue_resolved(
        &mut self,
        contracts: &[hir::ContractId],
        name: solar::interface::Symbol,
        args: &CallArgs<'_>,
    ) {
        for &cid in contracts {
            let mut found = false;
            for fid in self.hir.contract(cid).all_functions() {
                let func = self.hir.function(fid);
                if func.name.is_some_and(|n| n.name == name)
                    && args_match(self.hir, args, func.parameters)
                {
                    found = true;
                    if !self.visited.contains(&fid) {
                        self.worklist.push(fid);
                    }
                }
            }
            if found {
                return;
            }
        }
    }
}

/// Returns `true` if `args` can target `params` by arity and (when inferable) by type at each
/// position. Arguments whose type cannot be inferred do not reject a candidate.
fn args_match<'hir>(
    hir: &'hir hir::Hir<'hir>,
    args: &CallArgs<'hir>,
    params: &[VariableId],
) -> bool {
    if args.len() != params.len() {
        return false;
    }
    let compatible = |arg: &hir::Expr<'hir>, param: VariableId| -> bool {
        match expr_type(hir, arg) {
            Some(at) => types_compatible(&at, &hir.variable(param).ty.kind),
            None => true,
        }
    };
    match &args.kind {
        CallArgsKind::Unnamed(exprs) => {
            exprs.iter().zip(params.iter()).all(|(a, &p)| compatible(a, p))
        }
        CallArgsKind::Named(named) => named.iter().all(|arg| {
            let Some(&param) = params
                .iter()
                .find(|&&p| hir.variable(p).name.is_some_and(|n| n.name == arg.name.name))
            else {
                return false;
            };
            compatible(&arg.value, param)
        }),
    }
}

/// Best-effort static type of an expression. Returns `None` when the type cannot be inferred
/// from the expression's shape alone; callers treat that as "do not narrow on this position".
fn expr_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &hir::Expr<'hir>,
) -> Option<hir::TypeKind<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Payable(_) => Some(TypeKind::Elementary(ElementaryType::Address(true))),
        ExprKind::Lit(lit) => match &lit.kind {
            LitKind::Address(_) => Some(TypeKind::Elementary(ElementaryType::Address(false))),
            LitKind::Bool(_) => Some(TypeKind::Elementary(ElementaryType::Bool)),
            // Numeric / string / hex literals are implicitly convertible to many widths; leave
            // unknown so they don't reject candidates.
            _ => None,
        },
        ExprKind::Call(callee, args, _) => match &callee.peel_parens().kind {
            // `T(x)` elementary cast.
            ExprKind::Type(ty) => Some(ty.kind.clone()),
            // `f(...)` — single-return function call.
            ExprKind::Ident(reses) => reses.iter().find_map(|res| match res {
                Res::Item(ItemId::Function(fid)) => single_return_type(hir, *fid),
                _ => None,
            }),
            // `obj.method(...)` — single-return method on a contract-typed receiver.
            ExprKind::Member(base, member) => {
                let TypeKind::Custom(ItemId::Contract(cid)) = expr_type(hir, base)? else {
                    return None;
                };
                resolve_member_return_type(hir, cid, member.name, args)
            }
            _ => None,
        },
        ExprKind::New(ty) => Some(ty.kind.clone()),
        ExprKind::Ident(reses) => reses.iter().find_map(|res| match res {
            Res::Item(ItemId::Variable(id)) => Some(hir.variable(*id).ty.kind.clone()),
            Res::Item(ItemId::Contract(id)) => Some(TypeKind::Custom(ItemId::Contract(*id))),
            _ => None,
        }),
        ExprKind::Member(base, member) => {
            if is_address_builtin_member(base, member.name) {
                return Some(TypeKind::Elementary(ElementaryType::Address(false)));
            }
            // Struct field access: `s.field`.
            match expr_type(hir, base)? {
                TypeKind::Custom(ItemId::Struct(sid)) => struct_field_type(hir, sid, member.name),
                _ => None,
            }
        }
        // `m[i]` for mappings and arrays.
        ExprKind::Index(base, _) => match expr_type(hir, base)? {
            TypeKind::Mapping(m) => Some(m.value.kind.clone()),
            TypeKind::Array(a) => Some(a.element.kind.clone()),
            _ => None,
        },
        // `c ? a : b` — branches must agree per Solidity, so either type suffices.
        ExprKind::Ternary(_, then_e, else_e) => {
            expr_type(hir, then_e).or_else(|| expr_type(hir, else_e))
        }
        _ => None,
    }
}

/// Type of struct field `name` declared in `sid`.
fn struct_field_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    sid: hir::StructId,
    name: Symbol,
) -> Option<hir::TypeKind<'hir>> {
    hir.strukt(sid).fields.iter().find_map(|&fid| {
        let var = hir.variable(fid);
        (var.name?.name == name).then(|| var.ty.kind.clone())
    })
}

/// Return type of `fid` when it has exactly one return value.
fn single_return_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    fid: FunctionId,
) -> Option<hir::TypeKind<'hir>> {
    let func = hir.function(fid);
    (func.returns.len() == 1).then(|| hir.variable(func.returns[0]).ty.kind.clone())
}

/// Single-return type of `name` defined on `cid` or any of its bases, restricted to
/// overloads compatible with `args`. Walks the linearization most-derived first.
fn resolve_member_return_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    cid: hir::ContractId,
    name: Symbol,
    args: &CallArgs<'hir>,
) -> Option<hir::TypeKind<'hir>> {
    let contract = hir.contract(cid);
    let bases: &[hir::ContractId] = if contract.linearization_failed() {
        std::slice::from_ref(&cid)
    } else {
        contract.linearized_bases
    };
    for &bid in bases {
        for fid in hir.contract(bid).all_functions() {
            let func = hir.function(fid);
            if func.name.is_some_and(|n| n.name == name)
                && args_match(hir, args, func.parameters)
                && let Some(ty) = single_return_type(hir, fid)
            {
                return Some(ty);
            }
        }
    }
    None
}

/// Conservative type-compatibility check: only obvious matches and standard widenings count.
/// Anything else returns `false`.
fn types_compatible(arg: &hir::TypeKind<'_>, param: &hir::TypeKind<'_>) -> bool {
    match (arg, param) {
        // `address payable` fits an `address` slot; `address` does not fit `address payable`.
        (
            TypeKind::Elementary(ElementaryType::Address(a_pay)),
            TypeKind::Elementary(ElementaryType::Address(p_pay)),
        ) => !p_pay || *a_pay,
        // Contract values implicitly convert to `address` / `address payable`.
        (
            TypeKind::Custom(ItemId::Contract(_)),
            TypeKind::Elementary(ElementaryType::Address(_)),
        ) => true,
        (TypeKind::Array(a), TypeKind::Array(b)) => {
            a.size.is_some() == b.size.is_some()
                && types_compatible(&a.element.kind, &b.element.kind)
        }
        (TypeKind::Mapping(a), TypeKind::Mapping(b)) => {
            types_compatible(&a.key.kind, &b.key.kind)
                && types_compatible(&a.value.kind, &b.value.kind)
        }
        (TypeKind::Function(a), TypeKind::Function(b)) => {
            a.visibility == b.visibility
                && a.state_mutability == b.state_mutability
                && a.parameters.len() == b.parameters.len()
                && a.returns.len() == b.returns.len()
        }
        (TypeKind::Elementary(a), TypeKind::Elementary(b)) => a == b,
        (TypeKind::Custom(a), TypeKind::Custom(b)) => a == b,
        // Don't reject when either side errored out in semantic analysis.
        (TypeKind::Err(_), _) | (_, TypeKind::Err(_)) => true,
        _ => false,
    }
}

impl<'hir> hir::Visit<'hir> for SendChecker<'_, 'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    /// Inline assembly is lowered to `StmtKind::Err` by Solar; we cannot soundly inspect it
    /// for ETH-sending opcodes (`call`, `selfdestruct`, ...). Bail conservatively to avoid
    /// false positives on contracts whose only exit lives in assembly. Reusing `Break(())`
    /// here is intentional: the outer loop treats it the same as "found an exit" — skip
    /// the warning for this contract.
    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        if matches!(stmt.kind, StmtKind::Err(_)) {
            return ControlFlow::Break(());
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if expr_sends_ether(self.hir, expr) {
            return ControlFlow::Break(());
        }

        // Queue calls whose callee resolves statically to a `FunctionId`.
        if let ExprKind::Call(callee, args, _) = &expr.kind {
            match &callee.peel_parens().kind {
                ExprKind::Ident(reses) => {
                    for res in *reses {
                        match res {
                            Res::Item(ItemId::Function(fid))
                                if args_match(
                                    self.hir,
                                    args,
                                    self.hir.function(*fid).parameters,
                                ) =>
                            {
                                // Unqualified internal call: dispatch through the leaf's
                                // linearization so a leaf override of a `virtual` hook
                                // replaces the base implementation.
                                let effective = self.resolve_virtual(*fid, args);
                                if !self.visited.contains(&effective) {
                                    self.worklist.push(effective);
                                }
                            }
                            // Function-typed state/local variable: the bound target isn't
                            // statically known to us, so treat the call as opaque.
                            Res::Item(ItemId::Variable(id))
                                if matches!(
                                    self.hir.variable(*id).ty.kind,
                                    TypeKind::Function(_)
                                ) =>
                            {
                                return ControlFlow::Break(());
                            }
                            _ => {}
                        }
                    }
                }
                ExprKind::Member(receiver, member) => {
                    self.queue_member_callee(receiver, *member, args);
                }
                _ => {}
            }
        }

        self.walk_expr(expr)
    }
}

/// Returns `true` if `expr` unambiguously moves ETH out of the contract: a non-zero `{value: x}`
/// call option, `.transfer`/`.send` with a non-zero amount, low-level `.delegatecall`/`.callcode`
/// (drainable via `selfdestruct`), or the `selfdestruct` builtin. Only literal `0` is treated as
/// a zero amount; any other expression is assumed non-zero.
fn expr_sends_ether(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, named_args) = &expr.kind else {
        return false;
    };
    let callee = callee.peel_parens();

    // `foo{value: x}(...)` / `new C{value: x}(...)` with `x != 0`. Targeting `self`
    // keeps the ETH in this contract, so it is not an exit.
    if let Some(opts) = named_args
        && opts.iter().any(|arg| arg.name.name == sym::value && !is_literal_zero(&arg.value))
    {
        let self_call =
            matches!(&callee.kind, ExprKind::Member(receiver, _) if is_self_address(receiver));
        if !self_call {
            return true;
        }
    }

    match &callee.kind {
        ExprKind::Member(receiver, member) => {
            // Only address-typed receivers that aren't `self` can move ETH out.
            if !receiver_is_address(hir, receiver) || is_self_address(receiver) {
                return false;
            }
            // Single-arg `.transfer`/`.send` to disambiguate from ERC20's 2-arg `transfer`.
            if matches!(member.name, sym::transfer | sym::send) && args.len() == 1 {
                let amt = args.exprs().next().expect("len == 1");
                if !is_literal_zero(amt) {
                    return true;
                }
            }
            if matches!(member.name, kw::Delegatecall | kw::Callcode) {
                return true;
            }
            // Unknown member on an address-typed receiver is only legal via a `using for`
            // binding (Solar's HIR doesn't expose those); assume conservatively that the
            // bound library function could move ETH.
            if !matches!(
                member.name,
                sym::transfer
                    | sym::send
                    | kw::Call
                    | kw::Delegatecall
                    | kw::Callcode
                    | kw::Staticcall
            ) {
                return true;
            }
        }
        ExprKind::Ident(reses)
            if reses.iter().any(|r| matches!(r, Res::Builtin(Builtin::Selfdestruct))) =>
        {
            // `selfdestruct(self)` burns balance in-place; not an exit.
            return !args.exprs().next().is_some_and(is_self_address);
        }
        _ => {}
    }

    false
}

/// Returns `true` when `expr` syntactically denotes this contract's own address:
/// `this`, `address(this)`, `payable(this)`, a contract/interface cast `IFoo(<self>)`,
/// or any nested combination thereof.
fn is_self_address(expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().any(|r| matches!(r, Res::Builtin(Builtin::This))),
        ExprKind::Payable(inner) => is_self_address(inner),
        // `address(<self>)`, `IFoo(<self>)` and similar single-arg type casts.
        ExprKind::Call(callee, args, _) if is_type_cast_callee(callee) => {
            args.exprs().next().is_some_and(is_self_address)
        }
        _ => false,
    }
}

/// `T(...)` callee where `T` names a type: an elementary type, a contract/interface, or
/// any other item used in a single-argument cast position.
fn is_type_cast_callee(callee: &hir::Expr<'_>) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Type(_) => true,
        ExprKind::Ident(reses) => reses.iter().any(|r| matches!(r, Res::Item(ItemId::Contract(_)))),
        _ => false,
    }
}

/// Returns `true` if `expr` is statically typed as `address`/`address payable`. Contract-typed
/// receivers are intentionally rejected: `.transfer` / `.send` on them dispatch to a user-defined
/// member, not the EVM opcode.
fn receiver_is_address(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    matches!(expr_type(hir, expr), Some(TypeKind::Elementary(ElementaryType::Address(_))))
}

/// `msg.sender`, `tx.origin`, `block.coinbase`.
fn is_address_builtin_member(base: &hir::Expr<'_>, member: Symbol) -> bool {
    let ExprKind::Ident(reses) = &base.peel_parens().kind else { return false };
    reses.iter().any(|res| {
        let Res::Builtin(builtin) = res else { return false };
        matches!(
            (builtin.name(), member),
            (sym::msg, sym::sender) | (sym::tx, kw::Origin) | (sym::block, kw::Coinbase)
        )
    })
}

/// Returns `true` if the expression is the integer literal `0`.
fn is_literal_zero(expr: &hir::Expr<'_>) -> bool {
    if let ExprKind::Lit(lit) = &expr.peel_parens().kind
        && let LitKind::Number(n) = &lit.kind
    {
        return n.is_zero();
    }
    false
}
