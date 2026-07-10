use super::UnsafeOzErc721Mint;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::primitives::is_require_or_assert},
};
use alloy_primitives::U256;
use solar::{
    ast::{ElementaryType, LitKind},
    interface::{kw, source_map::FileName},
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind, FunctionId, Hir, Visit},
        ty::TyKind,
    },
};
use std::{convert::Infallible, ops::ControlFlow};

declare_forge_lint!(
    UNSAFE_OZ_ERC721_MINT,
    Severity::Med,
    "unsafe-oz-erc721-mint",
    "`ERC721._mint` does not check that the recipient can receive the token; use `_safeMint`"
);

impl<'hir> LateLintPass<'hir> for UnsafeOzErc721Mint {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        // Only the canonical OZ `_safeMint` wrapper is exempt: it legitimately calls `_mint`
        // next to its receiver check. A user-defined `_safeMint` override stays analyzed, since
        // it can call `_mint` directly without any check.
        if func.name.is_some_and(|name| name.as_str() == "_safeMint")
            && func.contract.is_some_and(|id| is_canonical_erc721(hir.contract(id).name.as_str()))
            && is_openzeppelin_source(hir, func.source)
        {
            return;
        }
        // A user `_mint` override is part of the mint primitive itself: `super._mint` there is
        // delegation (the capped/pausable pattern), and `_safeMint` there would re-enter the
        // override through the virtual dispatch. A delegating override reports at its call
        // sites instead, where `_safeMint` is the fix.
        if func.name.is_some_and(|name| name.as_str() == "_mint") && func.override_ {
            return;
        }
        // `ERC721._mint` credits the token without calling `onERC721Received`, so minting to a
        // contract that cannot handle ERC721 tokens locks the token; `_safeMint` performs the
        // check. Flag calls that resolve to a `_mint` declared in an ERC721 contract.
        if let Some(body) = &func.body {
            let mut finder = MintCallFinder { gcx, hir, ctx };
            for stmt in body.stmts {
                let _ = finder.visit_stmt(stmt);
            }
        }
    }
}

struct MintCallFinder<'ctx, 's, 'c, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    ctx: &'ctx LintContext<'s, 'c>,
}

impl<'hir> Visit<'hir> for MintCallFinder<'_, '_, '_, 'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Call(callee, _, _) = &expr.kind
            && let Some(function_id) = self.resolved_callee(callee)
            && self.is_erc721_mint(function_id)
        {
            self.ctx.emit(&UNSAFE_OZ_ERC721_MINT, expr.span);
        }
        self.walk_expr(expr)
    }
}

impl MintCallFinder<'_, '_, '_, '_> {
    /// The single function a call dispatches to. `type_of_expr` on the callee is the function the
    /// type checker resolved, so overload selection by argument types (`_mint(to, data)` vs
    /// `_mint(to, id)`), override shadowing (a contract that overrides `_mint` resolves to its own
    /// declaration, not the base it hides) and `super._mint(...)` are all already accounted for.
    fn resolved_callee(&self, callee: &hir::Expr<'_>) -> Option<FunctionId> {
        let ty = self.gcx.type_of_expr(callee.peel_parens().id)?;
        match ty.kind {
            TyKind::Fn(function_ty) => function_ty.function_id,
            _ => None,
        }
    }

    /// Whether `function_id` is a `_mint` whose execution skips the receiver check: the
    /// canonical OZ declaration, or a user override that transitively delegates to one.
    fn is_erc721_mint(&self, function_id: FunctionId) -> bool {
        self.is_unsafe_mint_target(function_id, &mut Vec::new())
    }

    /// The canonical case requires the exact OZ contract name AND an OpenZeppelin source
    /// path, so a local contract reusing a name like `ERC721Consecutive` stays out. The
    /// delegation case covers a `_mint` override whose body calls a `_mint` that is itself
    /// unsafe (the capped/pausable pattern forwarding through `super._mint`): direct calls
    /// dispatch to the override, but the path still reaches the unchecked base. `seen`
    /// cuts override cycles.
    fn is_unsafe_mint_target(&self, function_id: FunctionId, seen: &mut Vec<FunctionId>) -> bool {
        // A cycle of overrides never reaches the canonical declaration.
        if seen.contains(&function_id) {
            return false;
        }
        seen.push(function_id);
        let function = self.hir.function(function_id);
        if !function.name.is_some_and(|name| name.as_str() == "_mint") {
            return false;
        }
        let Some(contract_id) = function.contract else { return false };
        let contract = self.hir.contract(contract_id);
        if contract.kind.is_library() {
            return false;
        }
        // The canonical unchecked `_mint`: exact OZ name, OZ package provenance. Most
        // extensions (`ERC721Enumerable`, ...) inherit `_mint` rather than redeclare it, so
        // resolution still lands here.
        if is_canonical_erc721(contract.name.as_str())
            && is_openzeppelin_source(self.hir, function.source)
        {
            return true;
        }
        // A delegating override: any call in its body dispatching to an unsafe `_mint`. An
        // override that reverts when the recipient refuses the token is a safe wrapper like the
        // canonical `_safeMint`, so it is not an unsafe target. See [`reverts_on_refusal`].
        if function.override_
            && let Some(body) = &function.body
        {
            // The minted recipient is the override's first address-typed parameter.
            let recipient = function.parameters.iter().copied().find(|&vid| {
                matches!(
                    self.hir.variable(vid).ty.kind,
                    hir::TypeKind::Elementary(ElementaryType::Address(_))
                )
            });
            let mut scan = CalleeCollector { gcx: self.gcx, hir: self.hir, calls: Vec::new() };
            for stmt in body.stmts {
                let _ = scan.visit_stmt(stmt);
            }
            // Each distinct callee is judged once, with the shared `seen` set so override cycles
            // stop. Judging it per call site would answer `false` for every repeat, `seen` having
            // recorded the first.
            let mut unsafe_targets: Vec<FunctionId> = Vec::new();
            let mut judged: Vec<FunctionId> = Vec::new();
            for (callee, _) in &scan.calls {
                if judged.contains(callee) {
                    continue;
                }
                judged.push(*callee);
                // Each callee gets its own `seen`: a cycle is a property of one path, and two
                // siblings sharing a transitive target would otherwise silence the second.
                let mut branch = seen.clone();
                if self.is_unsafe_mint_target(*callee, &mut branch) {
                    unsafe_targets.push(*callee);
                }
            }
            let mut delegates = false;
            let mut only_to_recipient = true;
            for (callee, first_argument) in &scan.calls {
                if !unsafe_targets.contains(callee) {
                    continue;
                }
                delegates = true;
                // A guard covers the recipient it names, and no other.
                let handed_the_recipient = recipient.is_some_and(|recipient| {
                    first_argument.is_some_and(|argument| is_exactly_recipient(argument, recipient))
                });
                if !handed_the_recipient {
                    only_to_recipient = false;
                }
            }
            if !delegates {
                return false;
            }
            if only_to_recipient
                && let Some(recipient) = recipient
                && (reverts_on_refusal(self.gcx, self.hir, body.stmts, recipient, &mut Vec::new())
                    || modifiers_revert_on_refusal(self.gcx, self.hir, function, recipient))
            {
                return false;
            }
            return true;
        }
        false
    }
}

/// The package-root directory names of the OpenZeppelin distributions: the npm scope and the
/// git-submodule roots. Provenance is judged against a full path component so a same-name
/// contract under a merely-substring path such as `src/not-openzeppelin/` is not recognized.
const OPENZEPPELIN_PACKAGE_ROOTS: [&str; 3] =
    ["@openzeppelin", "openzeppelin-contracts", "openzeppelin-contracts-upgradeable"];

/// Whether a source file belongs to an OpenZeppelin package, judged by a full path component.
fn is_openzeppelin_source(hir: &Hir<'_>, source_id: hir::SourceId) -> bool {
    match &hir.source(source_id).file.name {
        FileName::Real(path) => path.components().any(|component| {
            matches!(component, std::path::Component::Normal(name)
                if OPENZEPPELIN_PACKAGE_ROOTS.iter().any(|root| name.eq_ignore_ascii_case(root)))
        }),
        _ => false,
    }
}

/// Collects every call in a subtree as its resolved target and the argument bound to the
/// callee's first parameter, which for a mint is the address the token goes to.
struct CalleeCollector<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    calls: Vec<(FunctionId, Option<&'hir Expr<'hir>>)>,
}

impl<'hir> Visit<'hir> for CalleeCollector<'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let Some(function_id) = resolved_callee(self.gcx, expr)
            && let ExprKind::Call(_, args, _) = &expr.kind
        {
            self.calls.push((function_id, first_parameter_argument(self.hir, function_id, args)));
        }
        self.walk_expr(expr)
    }
}

/// The argument a call binds to the callee's first parameter. Named arguments come in source
/// order, which is not the parameter order: `_mint({id: tokenId, to: to})` hands the token to
/// `to` all the same.
fn first_parameter_argument<'hir>(
    hir: &'hir Hir<'hir>,
    function_id: FunctionId,
    args: &'hir hir::CallArgs<'hir>,
) -> Option<&'hir Expr<'hir>> {
    match &args.kind {
        hir::CallArgsKind::Unnamed(exprs) => exprs.first(),
        hir::CallArgsKind::Named(named) => {
            let first = *hir.function(function_id).parameters.first()?;
            let name = hir.variable(first).name?;
            named
                .iter()
                .find(|argument| argument.name.as_str() == name.as_str())
                .map(|argument| &argument.value)
        }
    }
}

/// The function a call expression dispatches to, as the type checker resolved it.
fn resolved_callee(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<FunctionId> {
    let ExprKind::Call(callee, ..) = &expr.kind else { return None };
    let ty = gcx.type_of_expr(callee.peel_parens().id)?;
    let TyKind::Fn(function_ty) = ty.kind else { return None };
    function_ty.function_id
}

/// Whether a resolved declaration is the ERC721 receiver hook: the exact name and the exact
/// `(address, address, uint256, bytes)` shape. The name alone would let a call to a same-name
/// function of an unrelated interface, which answers on a different selector, pass as the check.
fn is_receiver_hook(hir: &Hir<'_>, function_id: FunctionId) -> bool {
    let function = hir.function(function_id);
    let Some(name) = function.name else { return false };
    if name.as_str() != "onERC721Received" {
        return false;
    }
    let params = function.parameters;
    if params.len() != 4 {
        return false;
    }
    let is = |index: usize, expected: fn(&hir::TypeKind<'_>) -> bool| {
        expected(&hir.variable(params[index]).ty.kind)
    };
    is(0, |kind| matches!(kind, hir::TypeKind::Elementary(ElementaryType::Address(_))))
        && is(1, |kind| matches!(kind, hir::TypeKind::Elementary(ElementaryType::Address(_))))
        && is(2, |kind| matches!(kind, hir::TypeKind::Elementary(ElementaryType::UInt(_))))
        && is(3, |kind| matches!(kind, hir::TypeKind::Elementary(ElementaryType::Bytes)))
}

/// The recipient itself, behind parentheses or a single interface cast: `to`, `(to)`,
/// `IERC721Receiver(to)`. Anything else, `guardians[to]` or `IERC721Receiver(other)`, is a
/// different address, and asking it about the token says nothing about the recipient.
fn is_exactly_recipient<'hir>(expr: &'hir Expr<'hir>, recipient: hir::VariableId) -> bool {
    let expr = expr.peel_parens();
    if let ExprKind::Ident(resolutions) = &expr.kind {
        return resolutions.iter().any(
            |res| matches!(res, hir::Res::Item(hir::ItemId::Variable(vid)) if *vid == recipient),
        );
    }
    // A cast keeps the address: `IERC721Receiver(to)` wraps the callee as an identifier resolved
    // to the contract, `address(to)` as a type.
    if let ExprKind::Call(callee, args, _) = &expr.kind
        && matches!(
            &callee.peel_parens().kind,
            ExprKind::Type(..) | ExprKind::Ident([hir::Res::Item(hir::ItemId::Contract(_)), ..])
        )
    {
        let mut operands = args.exprs();
        if let Some(inner) = operands.next()
            && operands.next().is_none()
        {
            return is_exactly_recipient(inner, recipient);
        }
    }
    false
}

/// `recipient.onERC721Received(...)`: the hook, asked of the recipient itself.
fn is_hook_call_on<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    recipient: hir::VariableId,
) -> bool {
    let expr = expr.peel_parens();
    let ExprKind::Call(callee, ..) = &expr.kind else { return false };
    let Some(function_id) = resolved_callee(gcx, expr) else { return false };
    if !is_receiver_hook(hir, function_id) {
        return false;
    }
    let ExprKind::Member(receiver, _) = &callee.peel_parens().kind else { return false };
    is_exactly_recipient(receiver, recipient)
}

/// The ERC721 answer meaning the recipient accepts the token, `onERC721Received`'s selector.
const ERC721_RECEIVED: u64 = 0x150b_7a02;

/// Whether an expression is the accepting answer. Comparing the hook's answer against anything
/// else mints to a recipient that answered something the ERC721 receiver interface calls a
/// refusal, so only an operand this reads and finds equal to the selector is credited: the
/// literal, a conversion of it, a `constant` holding it, or an `onERC721Received.selector`
/// member. A value settled at deployment or at runtime, an `immutable` or a state variable, is
/// unknown here and does not exempt.
fn is_received_selector(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Lit(lit) => {
            matches!(&lit.kind, LitKind::Number(value) if *value == U256::from(ERC721_RECEIVED))
        }
        // `bytes4(0x150b7a02)` and other conversions of a literal.
        ExprKind::Call(callee, args, _)
            if matches!(callee.peel_parens().kind, ExprKind::Type(..)) =>
        {
            let mut operands = args.exprs();
            match (operands.next(), operands.next()) {
                (Some(inner), None) => is_received_selector(hir, inner),
                _ => false,
            }
        }
        // `IERC721Receiver.onERC721Received.selector`, the answer named rather than spelled.
        ExprKind::Member(base, member) => {
            member.as_str() == "selector"
                && matches!(&base.peel_parens().kind, ExprKind::Member(_, hook)
                    if hook.as_str() == "onERC721Received")
        }
        // A constant is worth what it holds.
        ExprKind::Ident(resolutions) => resolutions.iter().any(|res| match res {
            hir::Res::Item(hir::ItemId::Variable(vid)) => {
                let variable = hir.variable(*vid);
                variable.is_constant()
                    && variable
                        .initializer
                        .is_some_and(|initializer| is_received_selector(hir, initializer))
            }
            _ => false,
        }),
        _ => false,
    }
}

/// `recipient.onERC721Received(...) <op> x`, and nothing else. The comparison must be the whole
/// expression: in `to == trusted || hook(to) == selector` the hook never runs for `trusted`. The
/// other operand must be able to hold the accepting answer, and must not be a hook call itself,
/// which would compare the recipient against itself.
fn is_hook_comparison<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    recipient: hir::VariableId,
    want: hir::BinOpKind,
) -> bool {
    let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind else { return false };
    if op.kind != want {
        return false;
    }
    let compares = |hook: &'hir Expr<'hir>, answer: &'hir Expr<'hir>| {
        is_hook_call_on(gcx, hir, hook, recipient)
            && !is_hook_call_on(gcx, hir, answer, recipient)
            && is_received_selector(hir, answer)
    };
    compares(lhs, rhs) || compares(rhs, lhs)
}

/// Whether executing `stmt` always reverts, undoing everything the transaction did. Only a
/// revert counts, the guards being matched without an order: see [`may_return`] for the escapes
/// that leave the transaction standing.
fn branch_always_reverts(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        hir::StmtKind::Revert(_) => true,
        hir::StmtKind::Expr(expr) => is_revert_call(expr),
        hir::StmtKind::Block(block) | hir::StmtKind::UncheckedBlock(block) => {
            // Read in order: a `revert` further down is only reached when nothing before it can
            // leave the function on its own, and a `return` is exactly such an escape.
            for stmt in block.stmts {
                if branch_always_reverts(stmt) {
                    return true;
                }
                if may_return(stmt) {
                    return false;
                }
            }
            false
        }
        hir::StmtKind::If(_, then, Some(else_)) => {
            branch_always_reverts(then) && branch_always_reverts(else_)
        }
        _ => false,
    }
}

/// Whether a statement may leave the function while keeping what the transaction already did.
/// A `return` does, and so does the EVM `return`/`stop` an assembly block can hold. Only the
/// statements that provably cannot leave answer no, so a guard is never credited on a path this
/// analysis does not read.
fn may_return(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        hir::StmtKind::DeclSingle(_)
        | hir::StmtKind::DeclMulti(..)
        | hir::StmtKind::Emit(_)
        | hir::StmtKind::Revert(_)
        | hir::StmtKind::Break
        | hir::StmtKind::Continue
        | hir::StmtKind::Expr(_)
        | hir::StmtKind::Placeholder
        | hir::StmtKind::Err(_) => false,
        hir::StmtKind::Block(block) | hir::StmtKind::UncheckedBlock(block) => {
            block.stmts.iter().any(may_return)
        }
        hir::StmtKind::If(_, then, else_) => may_return(then) || else_.is_some_and(may_return),
        hir::StmtKind::Loop(block, _) => block.stmts.iter().any(may_return),
        hir::StmtKind::Return(_) | hir::StmtKind::AssemblyBlock(_) => true,
        hir::StmtKind::Try(_) | hir::StmtKind::Switch(_) => true,
    }
}

/// `revert(...)`, `require(false, ...)` and `assert(false)`.
fn is_revert_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
    if matches!(&callee.peel_parens().kind, ExprKind::Ident(idents)
        if idents.iter().any(|res| matches!(res, hir::Res::Builtin(builtin) if builtin.name() == kw::Revert)))
    {
        return true;
    }
    is_require_or_assert(callee)
        && args.exprs().next().is_some_and(|first| {
            matches!(&first.peel_parens().kind, ExprKind::Lit(lit)
                if matches!(lit.kind, LitKind::Bool(false)))
        })
}

/// `recipient.code.length`.
fn is_recipient_code_length<'hir>(expr: &'hir Expr<'hir>, recipient: hir::VariableId) -> bool {
    let ExprKind::Member(code, length) = &expr.peel_parens().kind else { return false };
    if length.as_str() != "length" {
        return false;
    }
    let ExprKind::Member(base, member) = &code.peel_parens().kind else { return false };
    member.as_str() == "code" && is_exactly_recipient(base, recipient)
}

/// `recipient.code.length` compared against zero, for one polarity. Nothing else may ride along:
/// in `to.code.length > 0 && id == 5` the second operand decides whether the branch runs.
fn is_code_length_test<'hir>(
    expr: &'hir Expr<'hir>,
    recipient: hir::VariableId,
    has_code: bool,
) -> bool {
    let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind else { return false };
    let literal = |e: &Expr<'_>| match &e.peel_parens().kind {
        ExprKind::Lit(lit) => match &lit.kind {
            LitKind::Number(value) => u8::try_from(*value).ok(),
            _ => None,
        },
        _ => None,
    };
    let (bound, flipped) = if is_recipient_code_length(lhs, recipient) {
        (literal(rhs), false)
    } else if is_recipient_code_length(rhs, recipient) {
        (literal(lhs), true)
    } else {
        return false;
    };
    let Some(bound) = bound else { return false };
    // `length > 0`, `length != 0` and `length >= 1` all say the recipient carries code; `== 0`,
    // `< 1` and `<= 0` all say it carries none. Each has a mirror with the operands swapped.
    if has_code {
        match (op.kind, flipped) {
            (hir::BinOpKind::Ne, _) => bound == 0,
            (hir::BinOpKind::Gt, false) | (hir::BinOpKind::Lt, true) => bound == 0,
            (hir::BinOpKind::Ge, false) | (hir::BinOpKind::Le, true) => bound == 1,
            _ => false,
        }
    } else {
        match (op.kind, flipped) {
            (hir::BinOpKind::Eq, _) => bound == 0,
            (hir::BinOpKind::Lt, false) | (hir::BinOpKind::Gt, true) => bound == 1,
            (hir::BinOpKind::Le, false) | (hir::BinOpKind::Ge, true) => bound == 0,
            _ => false,
        }
    }
}

/// The condition of a `require`/`assert` that passes only if the recipient accepted the token:
/// the hook comparison itself, or the account short circuit `to.code.length == 0 || hook == sel`,
/// whose skipped branch is the one an account takes, and an account always accepts.
fn is_acceptance_condition<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    cond: &'hir Expr<'hir>,
    recipient: hir::VariableId,
) -> bool {
    let cond = cond.peel_parens();
    if is_hook_comparison(gcx, hir, cond, recipient, hir::BinOpKind::Eq) {
        return true;
    }
    let ExprKind::Binary(lhs, op, rhs) = &cond.kind else { return false };
    if op.kind != hir::BinOpKind::Or {
        return false;
    }
    let accepts = |skip: &'hir Expr<'hir>, check: &'hir Expr<'hir>| {
        is_code_length_test(skip, recipient, false)
            && is_hook_comparison(gcx, hir, check, recipient, hir::BinOpKind::Eq)
    };
    accepts(lhs, rhs) || accepts(rhs, lhs)
}

/// A call handing the recipient itself to a function or a modifier whose body reverts on refusal
/// for the parameter it lands on. The argument is matched by [`is_exactly_recipient`].
fn callee_guards_recipient<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    function_id: FunctionId,
    args: &'hir hir::CallArgs<'hir>,
    recipient: hir::VariableId,
    seen: &mut Vec<FunctionId>,
) -> bool {
    let parameters = hir.function(function_id).parameters;
    for (index, arg) in args.exprs().enumerate() {
        if is_exactly_recipient(arg, recipient)
            && let Some(&param) = parameters.get(index)
            && body_reverts_on_refusal(gcx, hir, function_id, param, seen)
        {
            return true;
        }
    }
    false
}

/// Whether a body reverts when the recipient refuses the token, which is what makes a delegating
/// override a safe wrapper: the token cannot end up in a contract that never acknowledged it.
///
/// The recognized shapes are a closed set, because a hook call that merely appears inside a
/// condition proves nothing about whether the revert depends on its answer. They are:
/// `require`/`assert` on an acceptance condition, `if (hook != selector) <exits>`,
/// `if (hook == selector) {} else <exits>`, any of those nested under `if (to.code.length > 0)`,
/// and any of those reached through a function or a modifier the recipient itself is handed to,
/// the way OpenZeppelin factors `_checkOnERC721Received` out of `_safeMint`. The guard may follow
/// the delegation, since the revert undoes the mint.
///
/// Everything else reports, a `try` whose `catch` may swallow the refusal included, and so are an
/// answer stored in a local and a helper returning it as a `bool`. Following the value across
/// statements would take a dataflow analysis this detector does not run.
fn reverts_on_refusal<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    stmts: &'hir [hir::Stmt<'hir>],
    recipient: hir::VariableId,
    seen: &mut Vec<FunctionId>,
) -> bool {
    stmts.iter().any(|stmt| stmt_reverts_on_refusal(gcx, hir, stmt, recipient, seen))
}

fn stmt_reverts_on_refusal<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
    recipient: hir::VariableId,
    seen: &mut Vec<FunctionId>,
) -> bool {
    match &stmt.kind {
        hir::StmtKind::Expr(expr) => {
            let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
            // Only the condition is read: a hook call sitting in the revert message decides
            // nothing, and neither does one in any other argument.
            if is_require_or_assert(callee) {
                return args
                    .exprs()
                    .next()
                    .is_some_and(|cond| is_acceptance_condition(gcx, hir, cond, recipient));
            }
            resolved_callee(gcx, expr.peel_parens()).is_some_and(|function_id| {
                callee_guards_recipient(gcx, hir, function_id, args, recipient, seen)
            })
        }
        hir::StmtKind::If(cond, then, else_) => {
            // The exiting branch must be the one a refusal takes, not the one an acceptance does.
            if is_hook_comparison(gcx, hir, cond, recipient, hir::BinOpKind::Ne)
                && branch_always_reverts(then)
            {
                return true;
            }
            if is_hook_comparison(gcx, hir, cond, recipient, hir::BinOpKind::Eq)
                && else_.is_some_and(branch_always_reverts)
            {
                return true;
            }
            // Skipping the hook for an account is the one admissible reason not to run it.
            is_code_length_test(cond, recipient, true)
                && stmt_reverts_on_refusal(gcx, hir, then, recipient, seen)
        }
        hir::StmtKind::Block(block) | hir::StmtKind::UncheckedBlock(block) => {
            reverts_on_refusal(gcx, hir, block.stmts, recipient, seen)
        }
        _ => false,
    }
}

/// [`reverts_on_refusal`] on a callee's body, against the parameter the recipient landed on.
/// `seen` cuts recursion cycles.
fn body_reverts_on_refusal<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    function_id: FunctionId,
    recipient: hir::VariableId,
    seen: &mut Vec<FunctionId>,
) -> bool {
    if seen.contains(&function_id) {
        return false;
    }
    seen.push(function_id);
    let function = hir.function(function_id);
    // A `virtual` callee may be replaced by an override that drops the guard, and the body seen
    // here is the statically resolved one, not the one dispatched.
    if function.virtual_ {
        return false;
    }
    let Some(body) = &function.body else { return false };
    reverts_on_refusal(gcx, hir, body.stmts, recipient, seen)
}

/// Whether one of the modifiers a function carries reverts on refusal for the recipient it is
/// handed. A base-constructor call in the same list carries no body to analyze.
fn modifiers_revert_on_refusal<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    function: &'hir hir::Function<'hir>,
    recipient: hir::VariableId,
) -> bool {
    function.modifiers.iter().any(|modifier| {
        matches!(modifier.id, hir::ItemId::Function(id)
            if callee_guards_recipient(gcx, hir, id, &modifier.args, recipient, &mut Vec::new()))
    })
}

/// The OpenZeppelin contracts whose `_mint` skips the receiver check. `ERC721` and
/// `ERC721Upgradeable` declare the unchecked `_mint`; in the v4 line, `ERC721Consecutive` and
/// `ERC721ConsecutiveUpgradeable` override it with a construction guard that forwards to the
/// base through `super._mint`, still without a receiver check, so resolving to them is just
/// as unsafe. In v5 the Consecutive extension overrides `_update` instead, and the two extra
/// names match nothing.
fn is_canonical_erc721(name: &str) -> bool {
    matches!(
        name,
        "ERC721" | "ERC721Upgradeable" | "ERC721Consecutive" | "ERC721ConsecutiveUpgradeable"
    )
}
