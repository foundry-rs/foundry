use super::UnsafeOzErc721Mint;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::primitives::is_require_or_assert},
};
use alloy_primitives::U256;
use solar::{
    ast::{ElementaryType, LitKind, Visibility},
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
        // `type_of_expr` on the callee is the function the type checker resolved, so overload
        // selection by argument types (`_mint(to, data)` vs `_mint(to, id)`), override shadowing
        // (a contract that overrides `_mint` resolves to its own declaration, not the base it
        // hides) and `super._mint(...)` are all already accounted for.
        if let Some(function_id) = resolved_callee(self.gcx, expr)
            && self.is_erc721_mint(function_id)
        {
            self.ctx.emit(&UNSAFE_OZ_ERC721_MINT, expr.span);
        }
        self.walk_expr(expr)
    }
}

impl MintCallFinder<'_, '_, '_, '_> {
    /// Whether `function_id` is a `_mint` whose execution skips the receiver check: the
    /// canonical OZ declaration, or a user override that transitively delegates to one.
    fn is_erc721_mint(&self, function_id: FunctionId) -> bool {
        self.unsafe_mint_target(function_id, &mut Vec::new()).is_some()
    }

    /// The canonical case requires the exact OZ contract name AND an OpenZeppelin source
    /// path, so a local contract reusing a name like `ERC721Consecutive` stays out. The
    /// delegation case covers a `_mint` override whose body calls a `_mint` that is itself
    /// unsafe (the capped/pausable pattern forwarding through `super._mint`): direct calls
    /// dispatch to the override, but the path still reaches the unchecked base. `seen`
    /// cuts override cycles.
    fn unsafe_mint_target(
        &self,
        function_id: FunctionId,
        seen: &mut Vec<FunctionId>,
    ) -> Option<UnsafeMintTarget> {
        // A cycle of overrides never reaches the canonical declaration.
        if seen.contains(&function_id) {
            return None;
        }
        seen.push(function_id);
        let function = self.hir.function(function_id);
        if function.name.is_none_or(|name| name.as_str() != "_mint") {
            return None;
        }
        let contract_id = function.contract?;
        let contract = self.hir.contract(contract_id);
        if contract.kind.is_library() {
            return None;
        }
        // The canonical unchecked `_mint`: exact OZ name, OZ package provenance. Most
        // extensions (`ERC721Enumerable`, ...) inherit `_mint` rather than redeclare it, so
        // resolution still lands here.
        if is_canonical_erc721(contract.name.as_str())
            && is_openzeppelin_source(self.hir, function.source)
        {
            return Some(UnsafeMintTarget { preserves_recipient: true, preserves_token: true });
        }
        // A delegating override: any call in its body dispatching to an unsafe `_mint`. An
        // override that reverts when the recipient refuses the token is a safe wrapper like the
        // canonical `_safeMint`, so it is not an unsafe target. See [`walk_guards`].
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
            let mut targets_preserve_recipient = true;
            let mut targets_preserve_token = true;
            for (callee, _) in &scan.calls {
                if judged.contains(callee) {
                    continue;
                }
                judged.push(*callee);
                // Each callee gets its own `seen`: a cycle is a property of one path, and two
                // siblings sharing a transitive target would otherwise silence the second.
                let mut branch = seen.clone();
                if let Some(target) = self.unsafe_mint_target(*callee, &mut branch) {
                    unsafe_targets.push(*callee);
                    targets_preserve_recipient &= target.preserves_recipient;
                    targets_preserve_token &= target.preserves_token;
                }
            }
            let mut delegates = false;
            let mut only_to_recipient = true;
            // The token every delegation credits, when they all name the same variable: the
            // recipient may accept one token and refuse another, so a guard is only about the
            // token it was asked about, and two delegations minting different tokens have no
            // single answer to share.
            let mut token = None;
            let mut token_consistent = true;
            for (callee, args) in &scan.calls {
                if !unsafe_targets.contains(callee) {
                    continue;
                }
                delegates = true;
                // A guard covers the recipient it names, and no other. The recipient is what
                // the delegation binds to the callee's first parameter, the token what it binds
                // to the second, as the canonical `_mint(address to, uint256 tokenId)` orders
                // them.
                let handed_the_recipient = recipient.is_some_and(|recipient| {
                    argument_bound_to_parameter(self.hir, *callee, args, 0)
                        .is_some_and(|argument| is_exactly_var(argument, recipient))
                });
                if !handed_the_recipient {
                    only_to_recipient = false;
                }
                match argument_bound_to_parameter(self.hir, *callee, args, 1)
                    .and_then(variable_of)
                    .filter(|&minted| keeps_its_value(self.hir, minted))
                {
                    Some(minted) => {
                        if token.is_some_and(|token| token != minted) {
                            token_consistent = false;
                        }
                        token = Some(minted);
                    }
                    // A token that is not a plain variable, a literal or an expression, cannot
                    // be matched against what a guard was asked about; neither can one the
                    // recipient may move under the guard's feet, see [`keeps_its_value`].
                    None => token_consistent = false,
                }
            }
            if !delegates {
                return None;
            }
            if only_to_recipient
                && targets_preserve_recipient
                && let Some(recipient) = recipient
            {
                // A proof that the recipient has no code is independent of the token being
                // minted. Reuse the guard walk with the recipient in both identity slots: a
                // callback guard cannot type-check with an address as its token, so the only
                // coverage this pass can establish is a code-length proof. This also lets an
                // address-only helper or modifier establish coverage without inventing a token
                // parameter, and permits computed or remapped token arguments.
                let covered =
                    modifiers_revert_on_refusal(self.gcx, self.hir, function, recipient, recipient);
                let mut walk = GuardWalk { covered, ..GuardWalk::default() };
                walk_guards(
                    self.gcx,
                    self.hir,
                    body.stmts,
                    recipient,
                    recipient,
                    &unsafe_targets,
                    false,
                    &mut Vec::new(),
                    &mut walk,
                );
                if !walk.failed && !walk.pending {
                    return None;
                }
            }
            if only_to_recipient
                && token_consistent
                && targets_preserve_recipient
                && targets_preserve_token
                && let Some(recipient) = recipient
                && let Some(token) = token
            {
                // A modifier guard runs at the call boundary, so it starts the body covered:
                // it acknowledged the values passed in. But the body is still read in order,
                // because it holds a copy of those values, and reassigning the recipient or the
                // token before a delegation credits a value the modifier never saw. Seeding the
                // walk covered lets the body's own reassignments retire it, the way a body guard
                // is retired. See [`walk_guards`].
                let covered =
                    modifiers_revert_on_refusal(self.gcx, self.hir, function, recipient, token);
                let mut walk = GuardWalk { covered, ..GuardWalk::default() };
                walk_guards(
                    self.gcx,
                    self.hir,
                    body.stmts,
                    recipient,
                    token,
                    &unsafe_targets,
                    false,
                    &mut Vec::new(),
                    &mut walk,
                );
                if !walk.failed && !walk.pending {
                    return None;
                }
            }
            // A guard in an outer override needs every intermediate override to preserve the
            // identities it relies on: the recipient for a code-less proof, and both recipient
            // and token for a callback. The current override's own guard above may legitimately
            // check a remapped local value, so this summary is propagated only to its callers;
            // it does not gate its own guard correlation.
            let recipient_parameter = function.parameters.first().copied();
            let token_parameter = function.parameters.get(1).copied();
            let recipient_unchanged = recipient_parameter.is_some_and(|recipient| {
                !body.stmts.iter().any(|stmt| mutates_var(self.hir, stmt, recipient))
            });
            let token_unchanged = token_parameter.is_some_and(|token| {
                !body.stmts.iter().any(|stmt| mutates_var(self.hir, stmt, token))
            });
            let forwards_recipient = recipient_parameter.is_some_and(|recipient| {
                scan.calls.iter().filter(|(callee, _)| unsafe_targets.contains(callee)).all(
                    |(callee, args)| {
                        argument_bound_to_parameter(self.hir, *callee, args, 0)
                            .is_some_and(|argument| is_exactly_var(argument, recipient))
                    },
                )
            });
            let forwards_token = token_parameter.is_some_and(|token| {
                scan.calls.iter().filter(|(callee, _)| unsafe_targets.contains(callee)).all(
                    |(callee, args)| {
                        argument_bound_to_parameter(self.hir, *callee, args, 1)
                            .is_some_and(|argument| is_exactly_var(argument, token))
                    },
                )
            });
            return Some(UnsafeMintTarget {
                preserves_recipient: targets_preserve_recipient
                    && recipient_unchanged
                    && forwards_recipient,
                preserves_token: targets_preserve_token && token_unchanged && forwards_token,
            });
        }
        None
    }
}

/// An unsafe mint target and whether every recursive hop preserves the recipient and token it
/// receives. A callback guard needs both guarantees, while a code-less-recipient proof needs
/// only the first.
#[derive(Clone, Copy)]
struct UnsafeMintTarget {
    preserves_recipient: bool,
    preserves_token: bool,
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

/// Collects every call in a subtree as its resolved target and its argument list, from which
/// the caller reads what the delegation binds to the mint's parameters.
struct CalleeCollector<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    calls: Vec<(FunctionId, &'hir hir::CallArgs<'hir>)>,
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
            self.calls.push((function_id, args));
        }
        self.walk_expr(expr)
    }
}

/// The argument a call binds to the callee's parameter at `index`. Named arguments come in
/// source order, which is not the parameter order: `_mint({id: tokenId, to: to})` hands the
/// token to `to` all the same.
fn argument_bound_to_parameter<'hir>(
    hir: &'hir Hir<'hir>,
    function_id: FunctionId,
    args: &'hir hir::CallArgs<'hir>,
    index: usize,
) -> Option<&'hir Expr<'hir>> {
    match &args.kind {
        hir::CallArgsKind::Unnamed(exprs) => exprs.get(index),
        hir::CallArgsKind::Named(named) => {
            let parameter = *hir.function(function_id).parameters.get(index)?;
            let name = hir.variable(parameter).name?;
            named
                .iter()
                .find(|argument| argument.name.as_str() == name.as_str())
                .map(|argument| &argument.value)
        }
    }
}

/// The function a bare expression names, as the type checker resolved it.
fn resolved_function(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<FunctionId> {
    let ty = gcx.type_of_expr(expr.peel_parens().id)?;
    let TyKind::Fn(function_ty) = ty.kind else { return None };
    function_ty.function_id
}

/// The function a call expression dispatches to, as the type checker resolved it.
fn resolved_callee(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<FunctionId> {
    let ExprKind::Call(callee, ..) = &expr.kind else { return None };
    resolved_function(gcx, callee)
}

/// The declaration a call executes in the current EVM frame. A public function called by name
/// is internal here, while `this.f()` and other external calls run in another frame whose
/// assembly `return` cannot bypass the caller's later statements.
fn resolved_internal_callee(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<FunctionId> {
    let ExprKind::Call(callee, ..) = &expr.kind else { return None };
    let ty = gcx.type_of_expr(callee.peel_parens().id)?;
    let TyKind::Fn(function_ty) = ty.kind else { return None };
    function_ty.is_internal().then_some(function_ty.function_id).flatten()
}

/// Whether a call dispatches through an internal function-pointer variable whose target is not
/// available from the callee type. Such a target may contain assembly that leaves the frame, so
/// exit analysis must treat the call conservatively.
fn is_unresolved_internal_pointer_call(gcx: Gcx<'_>, expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, ..) = &expr.kind else { return false };
    let Some(ty) = gcx.type_of_expr(callee.peel_parens().id) else { return false };
    let TyKind::Fn(function_ty) = ty.kind else { return false };
    function_ty.is_internal()
        && function_ty.function_id.is_none()
        && matches!(&callee.peel_parens().kind, ExprKind::Ident(resolutions)
        if resolutions.iter().any(|resolution| matches!(
            resolution,
            hir::Res::Item(hir::ItemId::Variable(_))
        )))
}

/// Whether a resolved declaration is the ERC721 receiver hook: the exact name, the exact
/// `(address, address, uint256, bytes)` shape, and an externally callable declaration of a
/// non-library contract. The name alone would let a call to a same-name function of an
/// unrelated interface, which answers on a different selector, pass as the check, and an
/// attached library or free function runs in the minting contract without any external call,
/// so resolving to one says nothing about the recipient.
fn is_receiver_hook(hir: &Hir<'_>, function_id: FunctionId) -> bool {
    let function = hir.function(function_id);
    let Some(name) = function.name else { return false };
    if name.as_str() != "onERC721Received" {
        return false;
    }
    let Some(contract_id) = function.contract else { return false };
    if hir.contract(contract_id).kind.is_library() {
        return false;
    }
    if !matches!(function.visibility, Visibility::Public | Visibility::External) {
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

/// The expression carrying the same value, behind parentheses, `payable(...)` and casts to an
/// address or contract type, which map every operand to itself. A numeric cast such as
/// `uint8(...)` truncates: the minted address is then usually not the checked one, so it is
/// not peeled.
fn peel_value_preserving<'hir>(expr: &'hir Expr<'hir>) -> &'hir Expr<'hir> {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Payable(inner) => peel_value_preserving(inner),
        // A cast wraps the callee as an identifier resolved to the contract for
        // `IERC721Receiver(to)`, as a type for `address(to)`.
        ExprKind::Call(callee, args, _)
            if matches!(&callee.peel_parens().kind,
                ExprKind::Type(ty)
                    if matches!(ty.kind, hir::TypeKind::Elementary(ElementaryType::Address(_))))
                || matches!(
                    &callee.peel_parens().kind,
                    ExprKind::Ident([hir::Res::Item(hir::ItemId::Contract(_)), ..])
                ) =>
        {
            let mut operands = args.exprs();
            if let Some(inner) = operands.next()
                && operands.next().is_none()
            {
                return peel_value_preserving(inner);
            }
            expr
        }
        _ => expr,
    }
}

/// The variable itself, behind parentheses and value-preserving conversions: `to`, `(to)`,
/// `IERC721Receiver(to)`, `payable(to)`. Anything else, `guardians[to]` or
/// `IERC721Receiver(other)`, is a different value, and asking it about the token says nothing
/// about the recipient.
fn is_exactly_var<'hir>(expr: &'hir Expr<'hir>, variable: hir::VariableId) -> bool {
    if let ExprKind::Ident(resolutions) = &peel_value_preserving(expr).kind {
        return resolutions.iter().any(
            |res| matches!(res, hir::Res::Item(hir::ItemId::Variable(vid)) if *vid == variable),
        );
    }
    false
}

/// Whether nothing can reassign the variable between the guard and the delegation. The hook is
/// an external call: the recipient answering it can reenter and move a mutable state variable
/// before the mint credits it, so the value the guard asked about and the one minted may
/// differ. A local, a parameter, a `constant` or an `immutable` cannot be moved that way.
fn keeps_its_value(hir: &Hir<'_>, variable: hir::VariableId) -> bool {
    let variable = hir.variable(variable);
    !variable.is_state_variable() || variable.is_constant() || variable.is_immutable()
}

/// The variable an expression is, behind the same conversions [`is_exactly_var`] sees through,
/// or nothing when the expression is not a plain variable.
fn variable_of(expr: &Expr<'_>) -> Option<hir::VariableId> {
    if let ExprKind::Ident(resolutions) = &peel_value_preserving(expr).kind {
        return resolutions.iter().find_map(|res| match res {
            hir::Res::Item(hir::ItemId::Variable(vid)) => Some(*vid),
            _ => None,
        });
    }
    None
}

/// `recipient.onERC721Received(..., token, ...)`: the hook, asked of the recipient itself and
/// about the delegated token itself. A recipient may accept one token and refuse another, so an
/// answer about a different id, the hook's third parameter, decides nothing for the minted one.
fn is_hook_call_on<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    recipient: hir::VariableId,
    token: hir::VariableId,
) -> bool {
    let expr = expr.peel_parens();
    let ExprKind::Call(callee, args, _) = &expr.kind else { return false };
    let Some(function_id) = resolved_callee(gcx, expr) else { return false };
    if !is_receiver_hook(hir, function_id) {
        return false;
    }
    let ExprKind::Member(receiver, _) = &callee.peel_parens().kind else { return false };
    is_exactly_var(receiver, recipient)
        && argument_bound_to_parameter(hir, function_id, args, 2)
            .is_some_and(|asked| is_exactly_var(asked, token))
}

/// The ERC721 answer meaning the recipient accepts the token, `onERC721Received`'s selector.
const ERC721_RECEIVED: u64 = 0x150b_7a02;

/// Whether an expression is the accepting answer. Comparing the hook's answer against anything
/// else mints to a recipient that answered something the ERC721 receiver interface calls a
/// refusal, so only an operand this reads and finds equal to the selector is credited: the
/// literal, a conversion of it, a `constant` holding it, or a `selector` member resolving to
/// the receiver hook itself. The member is resolved rather than matched by name: spelled on a
/// same-name function of another shape, `.selector` is a different value, which a recipient
/// answering it never accepted with. A value settled at deployment or at runtime, an
/// `immutable` or a state variable, is unknown here and does not exempt.
fn is_received_selector<'hir>(gcx: Gcx<'hir>, hir: &'hir Hir<'hir>, expr: &Expr<'hir>) -> bool {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Lit(lit) => {
            matches!(&lit.kind, LitKind::Number(value) if *value == U256::from(ERC721_RECEIVED))
        }
        // `bytes4(0x150b7a02)` and other conversions that preserve the four-byte value at
        // every hop. Fixed bytes are left-aligned while integers are right-aligned, so merely
        // peeling casts by width would accept lossy chains through a narrower integer.
        ExprKind::Call(callee, args, _)
            if matches!(callee.peel_parens().kind, ExprKind::Type(..)) =>
        {
            let mut operands = args.exprs();
            match (operands.next(), operands.next()) {
                (Some(inner), None) => {
                    selector_cast_preserves(gcx, expr, inner)
                        && is_received_selector(gcx, hir, inner)
                }
                _ => false,
            }
        }
        // `IERC721Receiver.onERC721Received.selector`, the answer named rather than spelled.
        ExprKind::Member(base, member) => {
            member.as_str() == "selector"
                && resolved_function(gcx, base)
                    .is_some_and(|function_id| is_receiver_hook(hir, function_id))
        }
        // A constant is worth what it holds.
        ExprKind::Ident(resolutions) => resolutions.iter().any(|res| match res {
            hir::Res::Item(hir::ItemId::Variable(vid)) => {
                let variable = hir.variable(*vid);
                variable.is_constant()
                    && variable
                        .initializer
                        .is_some_and(|initializer| is_received_selector(gcx, hir, initializer))
            }
            _ => false,
        }),
        _ => false,
    }
}

/// The representation of a selector-sized constant at one conversion step.
#[derive(Clone, Copy)]
enum SelectorEncoding {
    Literal,
    Integer(u16),
    FixedBytes(u8),
}

/// Whether a cast preserves the recognized selector's value and byte alignment. A recognized
/// integer is exactly the positive selector, so any integer width of at least 32 bits keeps it.
/// Crossing between right-aligned integers and left-aligned fixed bytes is only trusted at the
/// four-byte boundary.
fn selector_cast_preserves(gcx: Gcx<'_>, cast: &Expr<'_>, inner: &Expr<'_>) -> bool {
    let encoding = |expr: &Expr<'_>| match gcx.type_of_expr(expr.peel_parens().id)?.kind {
        TyKind::IntLiteral(..) => Some(SelectorEncoding::Literal),
        TyKind::Elementary(ElementaryType::Int(size) | ElementaryType::UInt(size)) => {
            Some(SelectorEncoding::Integer(size.bits()))
        }
        TyKind::Elementary(ElementaryType::FixedBytes(size)) => {
            Some(SelectorEncoding::FixedBytes(size.bytes()))
        }
        _ => None,
    };
    matches!(
        (encoding(inner), encoding(cast)),
        (
            Some(SelectorEncoding::Literal | SelectorEncoding::Integer(_)),
            Some(SelectorEncoding::Integer(32..) | SelectorEncoding::FixedBytes(4))
        ) | (Some(SelectorEncoding::FixedBytes(4)), Some(SelectorEncoding::Integer(32)))
            | (Some(SelectorEncoding::FixedBytes(4..)), Some(SelectorEncoding::FixedBytes(4..)))
    )
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
    token: hir::VariableId,
    want: hir::BinOpKind,
) -> bool {
    let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind else { return false };
    if op.kind != want {
        return false;
    }
    let compares = |hook: &'hir Expr<'hir>, answer: &'hir Expr<'hir>| {
        is_hook_call_on(gcx, hir, hook, recipient, token)
            && !is_hook_call_on(gcx, hir, answer, recipient, token)
            && is_received_selector(gcx, hir, answer)
    };
    compares(lhs, rhs) || compares(rhs, lhs)
}

/// Whether executing `stmt` always reverts, undoing everything the transaction did. Only a
/// revert counts, the guards being matched without an order: see [`may_return`] for the escapes
/// that leave the transaction standing.
fn branch_always_reverts<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
) -> bool {
    match &stmt.kind {
        hir::StmtKind::Revert(_) => !may_return(gcx, hir, stmt),
        hir::StmtKind::Expr(expr) => is_revert_call(expr) && !may_return(gcx, hir, stmt),
        hir::StmtKind::Block(block) | hir::StmtKind::UncheckedBlock(block) => {
            // Read in order: a `revert` further down is only reached when nothing before it can
            // leave the function on its own, and a `return` is exactly such an escape.
            for stmt in block.stmts {
                if branch_always_reverts(gcx, hir, stmt) {
                    return true;
                }
                if may_return(gcx, hir, stmt) {
                    return false;
                }
            }
            false
        }
        hir::StmtKind::If(cond, then, Some(else_)) => {
            !expr_contains_frame_ending_assembly(gcx, hir, cond)
                && branch_always_reverts(gcx, hir, then)
                && branch_always_reverts(gcx, hir, else_)
        }
        _ => false,
    }
}

/// Whether a statement may leave the function while keeping what the transaction already did.
/// A `return` does, and so does the EVM `return`/`stop` an assembly block can hold. Only the
/// statements that provably cannot leave answer no, so a guard is never credited on a path this
/// analysis does not read.
fn may_return<'hir>(gcx: Gcx<'hir>, hir: &'hir Hir<'hir>, stmt: &'hir hir::Stmt<'hir>) -> bool {
    if contains_frame_ending_assembly(gcx, hir, std::slice::from_ref(stmt), &mut Vec::new()) {
        return true;
    }
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
            block.stmts.iter().any(|stmt| may_return(gcx, hir, stmt))
        }
        hir::StmtKind::If(_, then, else_) => {
            may_return(gcx, hir, then) || else_.is_some_and(|else_| may_return(gcx, hir, else_))
        }
        hir::StmtKind::Loop(block, _) => block.stmts.iter().any(|stmt| may_return(gcx, hir, stmt)),
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
    member.as_str() == "code" && is_exactly_var(base, recipient)
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

/// The condition of a `require`/`assert` that passes only if the recipient can receive the token:
/// the recipient is proven code-less, the hook comparison succeeds, or the account short circuit
/// `to.code.length == 0 || hook == sel` accepts either case.
fn is_acceptance_condition<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    cond: &'hir Expr<'hir>,
    recipient: hir::VariableId,
    token: hir::VariableId,
) -> bool {
    let cond = cond.peel_parens();
    if is_code_length_test(cond, recipient, false)
        || is_hook_comparison(gcx, hir, cond, recipient, token, hir::BinOpKind::Eq)
    {
        return true;
    }
    let ExprKind::Binary(lhs, op, rhs) = &cond.kind else { return false };
    if op.kind != hir::BinOpKind::Or {
        return false;
    }
    let accepts = |skip: &'hir Expr<'hir>, check: &'hir Expr<'hir>| {
        is_code_length_test(skip, recipient, false)
            && is_hook_comparison(gcx, hir, check, recipient, token, hir::BinOpKind::Eq)
    };
    accepts(lhs, rhs) || accepts(rhs, lhs)
}

/// A call handing the checked identities to a same-frame function or modifier whose body proves
/// acceptance for the parameters they land on. Callback guards receive distinct recipient and
/// token identities; the recipient-only pass supplies the same identity in both slots, allowing
/// an address-only helper to prove that it has no code. Arguments are matched by [`is_exactly_var`]
/// against the callee's parameters and by parameter name for named arguments.
#[expect(clippy::too_many_arguments)]
fn callee_guards_recipient<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    function_id: FunctionId,
    args: &'hir hir::CallArgs<'hir>,
    recipient: hir::VariableId,
    token: hir::VariableId,
    bypass: bool,
    seen: &mut Vec<FunctionId>,
) -> bool {
    let parameters = hir.function(function_id).parameters;
    let mut recipient_parameter = None;
    let mut token_parameter = None;
    // The parameters the call binds the recipient and the token to, if any: the callee's own
    // guard is then judged against those.
    for (index, &parameter) in parameters.iter().enumerate() {
        let Some(argument) = argument_bound_to_parameter(hir, function_id, args, index) else {
            continue;
        };
        if recipient_parameter.is_none() && is_exactly_var(argument, recipient) {
            recipient_parameter = Some(parameter);
        }
        if token_parameter.is_none() && is_exactly_var(argument, token) {
            token_parameter = Some(parameter);
        }
    }
    if let (Some(recipient), Some(token)) = (recipient_parameter, token_parameter) {
        return body_guards(gcx, hir, function_id, recipient, token, bypass, seen);
    }
    false
}

/// The straight-line reading of a body: `covered` once a guard has run on the path, `pending`
/// while a delegated mint has run with no guard before or after it yet, `failed` once a path
/// may leave the function successfully with such a mint standing, `escaped` once one may leave
/// before any guard ran.
#[derive(Clone, Default)]
struct GuardWalk {
    covered: bool,
    pending: bool,
    failed: bool,
    escaped: bool,
}

/// Reads a body in statement order and judges the delegated mints against the guards. A guard
/// executed before a delegation covers it, the recipient having accepted the token by the time
/// it is credited; one executed after covers the delegations still pending, the revert undoing
/// them, unless a statement in between may leave the function successfully, keeping the
/// unacknowledged token: `super._mint(to, id); if (id == 0) return; require(hook...)` walks out
/// with token zero standing.
///
/// The recognized guard shapes are a closed set, because a hook call that merely appears inside
/// a condition proves nothing about whether the revert depends on its answer. They are:
/// `require`/`assert` on an acceptance condition, `if (hook != selector) <exits>`,
/// `if (hook == selector) {} else <exits>`, and any of those reached through a function or
/// modifier. A callback helper receives both identities, the way OpenZeppelin factors
/// `_checkOnERC721Received` out of `_safeMint`; a code-less proof needs only the recipient.
///
/// Branches are read separately and merged: coverage holds only when every path checked, while
/// a pending or escaping path taints the whole. The branch a `to.code.length` test dedicates to
/// accounts starts covered, an account always accepting the token. A loop body may run zero
/// times, so nothing in one is credited, while the delegations and escapes it may hold still
/// count.
///
/// Everything else reports, a `try` whose `catch` may swallow the refusal included, and so are
/// an answer stored in a local and a helper returning it as a `bool`. Following the value
/// across statements would take a dataflow analysis this detector does not run.
#[expect(clippy::too_many_arguments)]
fn walk_guards<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    stmts: &'hir [hir::Stmt<'hir>],
    recipient: hir::VariableId,
    token: hir::VariableId,
    delegations: &[FunctionId],
    bypass: bool,
    seen: &mut Vec<FunctionId>,
    walk: &mut GuardWalk,
) {
    // Read in order: what a guard covers and what an exit walks out with depend on what already
    // ran.
    for stmt in stmts {
        match &stmt.kind {
            hir::StmtKind::Block(block) | hir::StmtKind::UncheckedBlock(block) => {
                walk_guards(
                    gcx,
                    hir,
                    block.stmts,
                    recipient,
                    token,
                    delegations,
                    bypass,
                    seen,
                    walk,
                );
            }
            hir::StmtKind::Expr(expr) if is_guard_expr(gcx, hir, expr, recipient, token, seen) => {
                // A guard that also reassigns the recipient or the token cannot be trusted:
                // evaluation order decides whether the hook read the value the mint credits, so
                // an assignment tucked in the guard's other arguments retires coverage instead
                // of granting it. Nor can a guard that may leave the frame establish coverage:
                // an assembly return in another argument can keep a pending mint before the
                // builtin has a chance to revert.
                let mutates = mutates_var(hir, stmt, recipient) || mutates_var(hir, stmt, token);
                let escapes = may_return(gcx, hir, stmt);
                if mutates {
                    if walk.pending {
                        walk.failed = true;
                    }
                    walk.covered = false;
                } else if escapes {
                    if walk.pending {
                        walk.failed = true;
                    }
                    if !walk.covered {
                        walk.escaped = true;
                    }
                } else {
                    walk.covered = true;
                    walk.pending = false;
                }
            }
            hir::StmtKind::If(cond, then, else_) => {
                // The condition runs before either branch, so an assignment embedded in it,
                // `if ((tokenId = tokenId + 1) > 0) {}`, retires coverage exactly as a bare
                // assignment statement does. Even a recognized hook comparison may hide one in
                // another argument, so mutation also prevents the comparison from covering.
                let condition_mutates =
                    expr_mutates_var(hir, cond, recipient) || expr_mutates_var(hir, cond, token);
                let condition_escapes = expr_contains_frame_ending_assembly(gcx, hir, cond);
                if condition_mutates {
                    if walk.pending {
                        walk.failed = true;
                    }
                    walk.covered = false;
                }
                if condition_escapes {
                    if walk.pending {
                        walk.failed = true;
                    }
                    if !walk.covered {
                        walk.escaped = true;
                    }
                }
                // The exiting branch must be the one a refusal takes, not the one an acceptance
                // does. Continue reading the accepted branch from the covered state: it may
                // still reassign the checked values before delegating. A condition that itself
                // reassigns one of them cannot establish coverage for the new value.
                let refusal_then = !condition_mutates
                    && is_hook_comparison(gcx, hir, cond, recipient, token, hir::BinOpKind::Ne)
                    && branch_always_reverts(gcx, hir, then);
                let refusal_else = !condition_mutates
                    && is_hook_comparison(gcx, hir, cond, recipient, token, hir::BinOpKind::Eq)
                    && else_.is_some_and(|else_| branch_always_reverts(gcx, hir, else_));
                if refusal_then || refusal_else {
                    walk.covered = true;
                    walk.pending = false;
                    if refusal_then {
                        if let Some(accepted) = else_ {
                            walk_one(
                                gcx,
                                hir,
                                accepted,
                                recipient,
                                token,
                                delegations,
                                bypass,
                                seen,
                                walk,
                            );
                        }
                    } else {
                        walk_one(gcx, hir, then, recipient, token, delegations, bypass, seen, walk);
                    }
                    continue;
                }
                // Each branch is read on its own, from what already ran. The branch a
                // `to.code.length` test dedicates to accounts starts covered with nothing
                // pending: an account always accepts, so the mints already made are as
                // satisfied on that path as the ones to come, and its exits break no promise.
                let mut then_walk = walk.clone();
                let mut else_walk = walk.clone();
                if is_code_length_test(cond, recipient, true) {
                    else_walk.covered = true;
                    else_walk.pending = false;
                } else if is_code_length_test(cond, recipient, false) {
                    then_walk.covered = true;
                    then_walk.pending = false;
                }
                walk_one(
                    gcx,
                    hir,
                    then,
                    recipient,
                    token,
                    delegations,
                    bypass,
                    seen,
                    &mut then_walk,
                );
                if let Some(else_) = else_ {
                    walk_one(
                        gcx,
                        hir,
                        else_,
                        recipient,
                        token,
                        delegations,
                        bypass,
                        seen,
                        &mut else_walk,
                    );
                }
                // A guard behind a condition only counts when every path passed one; anything
                // pending or escaped on either path stands.
                walk.covered = then_walk.covered && else_walk.covered;
                walk.pending = then_walk.pending || else_walk.pending;
                walk.failed = then_walk.failed || else_walk.failed;
                walk.escaped = then_walk.escaped || else_walk.escaped;
            }
            _ => {
                // Reassigning the recipient or the token retires every guard so far: a guard
                // checked their value, and identity here is by variable, so a later delegation
                // spelling the same variable now credits a value the guard never saw. A mint
                // already pending can no longer be covered by a guard to come either, the guard
                // reading the new value. This runs first: an assignment inside a delegation's
                // own arguments happens before the call.
                if mutates_var(hir, stmt, recipient) || mutates_var(hir, stmt, token) {
                    if walk.pending {
                        walk.failed = true;
                    }
                    walk.covered = false;
                }
                // An opaque statement: a delegation anywhere inside it mints, unchecked unless
                // already covered, and a possible successful exit walks out with whatever is
                // pending. The placeholder counts as one when the wrapped body can leave
                // through an assembly `return`, which skips everything after `_;`: a guard only
                // the modifier's tail holds is then not guaranteed.
                if !walk.covered && contains_delegation(gcx, hir, stmt, delegations) {
                    walk.pending = true;
                }
                if may_return(gcx, hir, stmt) || (bypass && contains_placeholder(hir, stmt)) {
                    if walk.pending {
                        walk.failed = true;
                    }
                    if !walk.covered {
                        walk.escaped = true;
                    }
                }
            }
        }
    }
}

/// [`walk_guards`] on a single statement.
#[expect(clippy::too_many_arguments)]
fn walk_one<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
    recipient: hir::VariableId,
    token: hir::VariableId,
    delegations: &[FunctionId],
    bypass: bool,
    seen: &mut Vec<FunctionId>,
    walk: &mut GuardWalk,
) {
    walk_guards(
        gcx,
        hir,
        std::slice::from_ref(stmt),
        recipient,
        token,
        delegations,
        bypass,
        seen,
        walk,
    );
}

/// Whether a subtree holds the `_;` placeholder of a modifier.
fn contains_placeholder<'hir>(hir: &'hir Hir<'hir>, stmt: &'hir hir::Stmt<'hir>) -> bool {
    struct PlaceholderFinder<'hir> {
        hir: &'hir Hir<'hir>,
    }
    impl<'hir> Visit<'hir> for PlaceholderFinder<'hir> {
        type BreakValue = ();

        fn hir(&self) -> &'hir Hir<'hir> {
            self.hir
        }

        fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
            if matches!(stmt.kind, hir::StmtKind::Placeholder) {
                return ControlFlow::Break(());
            }
            self.walk_stmt(stmt)
        }
    }
    let mut finder = PlaceholderFinder { hir };
    finder.visit_stmt(stmt).is_break()
}

/// Whether a subtree can reach an assembly block in the same EVM frame, directly or through an
/// internal call. An assembly `return` leaves the frame without running a later revert or what
/// an outer modifier holds after its placeholder. Every assembly block is treated as capable of
/// doing so, conservatively matching [`may_return`].
fn contains_frame_ending_assembly<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    stmts: &'hir [hir::Stmt<'hir>],
    seen: &mut Vec<FunctionId>,
) -> bool {
    struct AssemblyFinder<'a, 'hir> {
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        seen: &'a mut Vec<FunctionId>,
    }
    impl<'hir> Visit<'hir> for AssemblyFinder<'_, 'hir> {
        type BreakValue = ();

        fn hir(&self) -> &'hir Hir<'hir> {
            self.hir
        }

        fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
            if matches!(stmt.kind, hir::StmtKind::AssemblyBlock(_)) {
                return ControlFlow::Break(());
            }
            self.walk_stmt(stmt)
        }

        fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
            if is_unresolved_internal_pointer_call(self.gcx, expr)
                || resolved_internal_callee(self.gcx, expr).is_some_and(|function_id| {
                    callable_contains_frame_ending_assembly(
                        self.gcx,
                        self.hir,
                        function_id,
                        self.seen,
                    )
                })
            {
                return ControlFlow::Break(());
            }
            self.walk_expr(expr)
        }
    }
    let mut finder = AssemblyFinder { gcx, hir, seen };
    stmts.iter().any(|stmt| finder.visit_stmt(stmt).is_break())
}

/// [`contains_frame_ending_assembly`] over an expression, used for a refusal condition that may
/// itself leave before either branch runs.
fn expr_contains_frame_ending_assembly<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
) -> bool {
    struct ExprAssemblyFinder<'a, 'hir> {
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        seen: &'a mut Vec<FunctionId>,
    }
    impl<'hir> Visit<'hir> for ExprAssemblyFinder<'_, 'hir> {
        type BreakValue = ();

        fn hir(&self) -> &'hir Hir<'hir> {
            self.hir
        }

        fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
            if is_unresolved_internal_pointer_call(self.gcx, expr)
                || resolved_internal_callee(self.gcx, expr).is_some_and(|function_id| {
                    callable_contains_frame_ending_assembly(
                        self.gcx,
                        self.hir,
                        function_id,
                        self.seen,
                    )
                })
            {
                return ControlFlow::Break(());
            }
            self.walk_expr(expr)
        }
    }
    let mut seen = Vec::new();
    let mut finder = ExprAssemblyFinder { gcx, hir, seen: &mut seen };
    finder.visit_expr(expr).is_break()
}

/// Whether a resolved same-frame callable or one of its applied modifiers can reach assembly.
/// The recursion set is a path stack so independent calls are summarized independently.
fn callable_contains_frame_ending_assembly<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    function_id: FunctionId,
    seen: &mut Vec<FunctionId>,
) -> bool {
    if seen.contains(&function_id) {
        return false;
    }
    seen.push(function_id);
    let function = hir.function(function_id);
    let in_modifiers = function.modifiers.iter().any(|modifier| {
        matches!(modifier.id, hir::ItemId::Function(id)
            if callable_contains_frame_ending_assembly(gcx, hir, id, seen))
    });
    let in_body = function
        .body
        .as_ref()
        .is_some_and(|body| contains_frame_ending_assembly(gcx, hir, body.stmts, seen));
    seen.pop();
    in_modifiers || in_body
}

/// A statement expression that guards the recipient and the token: `require`/`assert` on an
/// acceptance condition, or an internal call handing both to a helper that does. Only the
/// condition is read: a hook call sitting in the revert message decides nothing, and neither
/// does one in any other argument. An external helper would ask from a different contract and
/// cannot establish that the recipient accepts the minting contract's callback.
fn is_guard_expr<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    recipient: hir::VariableId,
    token: hir::VariableId,
    seen: &mut Vec<FunctionId>,
) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
    if is_require_or_assert(callee) {
        return args
            .exprs()
            .next()
            .is_some_and(|cond| is_acceptance_condition(gcx, hir, cond, recipient, token));
    }
    resolved_internal_callee(gcx, expr.peel_parens()).is_some_and(|function_id| {
        callee_guards_recipient(gcx, hir, function_id, args, recipient, token, false, seen)
    })
}

/// Whether a statement assigns to `var`: `var = x`, `var += x`, `var++`, `delete var`, or `var`
/// as a component of a tuple assignment. An assembly block is treated as an opaque assignment,
/// since it can rewrite Solidity locals outside the HIR expression tree. Identity here is by
/// variable, not by value, so a guard that checked `var` says nothing once `var` is reassigned:
/// the delegation then credits a value or an address the guard never saw, though the two spell
/// the same variable.
fn mutates_var<'hir>(
    hir: &'hir Hir<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
    var: hir::VariableId,
) -> bool {
    let mut finder = MutationFinder { hir, var };
    finder.visit_stmt(stmt).is_break()
}

/// [`mutates_var`] over an expression, for the condition of an `if`, which the `If` arm reaches
/// before the statement arm ever sees it: `if ((tokenId = x) > 0) {}` reassigns as surely as a
/// bare statement, and the condition runs whichever branch is taken.
fn expr_mutates_var<'hir>(
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    var: hir::VariableId,
) -> bool {
    let mut finder = MutationFinder { hir, var };
    finder.visit_expr(expr).is_break()
}

/// Finds an assignment to `var` anywhere in a subtree, conservatively including assembly.
struct MutationFinder<'hir> {
    hir: &'hir Hir<'hir>,
    var: hir::VariableId,
}

impl<'hir> Visit<'hir> for MutationFinder<'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        // Yul can assign to Solidity locals, but those assignments are not HIR expressions.
        // Treat an assembly block as opaque mutation of every tracked value rather than keep
        // coverage across a remap this visitor cannot inspect.
        if matches!(stmt.kind, hir::StmtKind::AssemblyBlock(_)) {
            return ControlFlow::Break(());
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        let target = match &expr.kind {
            ExprKind::Assign(lhs, _, _) => Some(*lhs),
            ExprKind::Delete(inner) => Some(*inner),
            ExprKind::Unary(op, inner)
                if matches!(
                    op.kind,
                    hir::UnOpKind::PreInc
                        | hir::UnOpKind::PreDec
                        | hir::UnOpKind::PostInc
                        | hir::UnOpKind::PostDec
                ) =>
            {
                Some(*inner)
            }
            _ => None,
        };
        if let Some(target) = target
            && assigns_to(target, self.var)
        {
            return ControlFlow::Break(());
        }
        self.walk_expr(expr)
    }
}

/// Whether an assignment target names `var`, directly or as one component of a tuple.
fn assigns_to(target: &Expr<'_>, var: hir::VariableId) -> bool {
    match &target.peel_parens().kind {
        ExprKind::Ident(resolutions) => resolutions
            .iter()
            .any(|res| matches!(res, hir::Res::Item(hir::ItemId::Variable(vid)) if *vid == var)),
        ExprKind::Tuple(elements) => {
            elements.iter().any(|element| element.is_some_and(|inner| assigns_to(inner, var)))
        }
        _ => false,
    }
}

/// Whether a subtree holds a call dispatching to one of the delegated mints.
fn contains_delegation<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
    delegations: &[FunctionId],
) -> bool {
    struct DelegationFinder<'a, 'hir> {
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        delegations: &'a [FunctionId],
    }
    impl<'hir> Visit<'hir> for DelegationFinder<'_, 'hir> {
        type BreakValue = ();

        fn hir(&self) -> &'hir Hir<'hir> {
            self.hir
        }

        fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
            if let Some(function_id) = resolved_callee(self.gcx, expr)
                && self.delegations.contains(&function_id)
            {
                return ControlFlow::Break(());
            }
            self.walk_expr(expr)
        }
    }
    let mut finder = DelegationFinder { gcx, hir, delegations };
    finder.visit_stmt(stmt).is_break()
}

/// [`walk_guards`] on a callee's body, against the parameters the recipient and the token
/// landed on: the callee guards when a guard ran before any possible successful exit, which is
/// what its caller relies on. `bypass` says the wrapped body can leave through an assembly
/// `return`, making an uncovered placeholder such an exit. `seen` cuts recursion cycles.
fn body_guards<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    function_id: FunctionId,
    recipient: hir::VariableId,
    token: hir::VariableId,
    bypass: bool,
    seen: &mut Vec<FunctionId>,
) -> bool {
    if seen.contains(&function_id) {
        return false;
    }
    seen.push(function_id);
    let function = hir.function(function_id);
    // A `virtual` callee may be replaced by an override that drops the guard, and the body seen
    // here is the statically resolved one, not the one dispatched. A helper carrying modifiers
    // is likewise not credited until their expansion is modeled: one may skip the placeholder
    // and let the helper return without ever running its body.
    let guarded = if function.virtual_ || !function.modifiers.is_empty() {
        false
    } else if let Some(body) = &function.body {
        // The caller relies on the values it passed in, not on a helper or modifier's local
        // parameter after reassignment. Conservatively reject any body that mutates either
        // bound parameter; a later guard may then be checking a different value.
        let parameters_unchanged = !body
            .stmts
            .iter()
            .any(|stmt| mutates_var(hir, stmt, recipient) || mutates_var(hir, stmt, token));
        if parameters_unchanged {
            let mut walk = GuardWalk::default();
            walk_guards(gcx, hir, body.stmts, recipient, token, &[], bypass, seen, &mut walk);
            walk.covered && !walk.escaped
        } else {
            false
        }
    } else {
        false
    };
    seen.pop();
    guarded
}

/// Whether one of the modifiers a function carries reverts on refusal for the recipient and the
/// token it is handed. A base-constructor call in the same list carries no body to analyze. A
/// wrapped body or inner modifier reaching assembly can leave the call frame without ever coming
/// back to an outer modifier, so a guard after `_;` is only guaranteed when the expansion inside
/// that placeholder cannot. Modifier order matters: an outer modifier runs after an inner tail
/// guard has already completed and cannot bypass it retroactively.
fn modifiers_revert_on_refusal<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    function: &'hir hir::Function<'hir>,
    recipient: hir::VariableId,
    token: hir::VariableId,
) -> bool {
    let body_bypass = function
        .body
        .as_ref()
        .is_some_and(|body| contains_frame_ending_assembly(gcx, hir, body.stmts, &mut Vec::new()));
    function.modifiers.iter().enumerate().any(|(index, modifier)| {
        let inner_modifier_bypass = function.modifiers[index + 1..].iter().any(|inner| {
            matches!(inner.id, hir::ItemId::Function(id)
                if callable_contains_frame_ending_assembly(gcx, hir, id, &mut Vec::new()))
        });
        let bypass = body_bypass || inner_modifier_bypass;
        matches!(modifier.id, hir::ItemId::Function(id)
        if callee_guards_recipient(
            gcx,
            hir,
            id,
            &modifier.args,
            recipient,
            token,
            bypass,
            &mut Vec::new(),
        ))
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
