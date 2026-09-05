use super::UnsafeOzErc721Mint;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            OPENZEPPELIN_ROOTS, arg_for_param, for_each_lhs_var, is_address_type, is_builtin,
            is_literal_false, is_require_or_assert, loop_stmts, resolved_function,
            source_in_package, underlying_var, unique, write_target,
        },
    },
};
use alloy_primitives::U256;
use solar::{
    ast::{ElementaryType, LitKind, StateMutability, Visibility},
    interface::{Span, kw},
    sema::{
        Gcx,
        hir::{
            self, BinOpKind, CallArgs, Expr, ExprKind, FunctionId, Hir, ItemId, Res, Stmt,
            StmtKind, TypeKind, VariableId, Visit,
        },
        ty::{TyFn, TyKind},
    },
};
use std::{ops::ControlFlow, slice};

declare_forge_lint!(
    UNSAFE_OZ_ERC721_MINT,
    Severity::Med,
    "unsafe-oz-erc721-mint",
    "`ERC721._mint` does not check that the recipient can receive the token; use `_safeMint`"
);

impl<'gcx> LateLintPass<'gcx> for UnsafeOzErc721Mint {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        func: &'gcx hir::Function<'gcx>,
    ) {
        let cx = Cx { gcx };
        // Only the canonical OZ `_safeMint` wrapper is exempt: it legitimately calls `_mint`
        // next to its receiver check. A user-defined `_safeMint` override stays analyzed, since
        // it can call `_mint` directly without any check.
        if named(func, "_safeMint")
            && func
                .contract
                .is_some_and(|id| is_canonical_erc721(gcx.hir.contract(id).name.as_str()))
            && source_in_package(&gcx.hir, func.source, OPENZEPPELIN_ROOTS)
        {
            return;
        }
        // A user `_mint` override is part of the mint primitive itself: `super._mint` there is
        // delegation (the capped/pausable pattern), and `_safeMint` there would re-enter the
        // override through the virtual dispatch. A delegating override reports at its call
        // sites instead, where `_safeMint` is the fix. The same holds for a helper such an
        // override delegates through.
        if (named(func, "_mint") && func.override_) || cx.is_override_delegation_helper(func) {
            return;
        }
        // `ERC721._mint` credits the token without calling `onERC721Received`, so minting to a
        // contract that cannot handle ERC721 tokens locks the token; `_safeMint` performs the
        // check. Flag calls that resolve to a `_mint` declared in an ERC721 contract. The type
        // checker's resolution already accounts for overload selection, override shadowing and
        // `super._mint(...)`.
        let Some(body) = &func.body else { return };
        for (callee, _, span) in cx.calls(body.stmts) {
            let helper = cx.is_override_delegation_helper(gcx.hir.function(callee));
            if cx.unsafe_mint_target(callee, helper, &mut Vec::new()).is_some() {
                ctx.emit(&UNSAFE_OZ_ERC721_MINT, span);
            }
        }
    }
}

/// An unsafe mint target and whether every recursive hop preserves the recipient and token it
/// receives. A callback guard needs both guarantees, while a code-less-recipient proof needs
/// only the first.
#[derive(Clone, Copy)]
struct UnsafeMintTarget {
    preserves_recipient: bool,
    preserves_token: bool,
    preserves_code_length: bool,
}

/// A resolved call: its target, arguments and span.
type Call<'gcx> = (FunctionId, &'gcx CallArgs<'gcx>, Span);

/// The analysis context.
#[derive(Clone, Copy)]
struct Cx<'gcx> {
    gcx: Gcx<'gcx>,
}

impl<'gcx> Cx<'gcx> {
    /// Whether an internal/private function is reached from a user `_mint` override of a
    /// derived contract, making it part of the mint primitive rather than a call site.
    fn is_override_delegation_helper(self, function: &'gcx hir::Function<'gcx>) -> bool {
        if !is_internal(function) || (function.override_ && named(function, "_mint")) {
            return false;
        }
        let Some(contract_id) = function.contract else { return false };
        let Some(function_id) = self
            .gcx
            .hir
            .contract(contract_id)
            .all_functions()
            .find(|&id| std::ptr::eq(self.gcx.hir.function(id), function))
        else {
            return false;
        };
        self.gcx.hir.contract_ids().any(|candidate| {
            let candidate = self.gcx.hir.contract(candidate);
            candidate.linearized_bases.contains(&contract_id)
                && candidate.all_functions().any(|id| {
                    let f = self.gcx.hir.function(id);
                    f.override_
                        && named(f, "_mint")
                        && self.function_reaches(id, function_id, &mut Vec::new())
                })
        })
    }

    /// Whether `function_id` calls `target`, directly or through internal functions.
    fn function_reaches(
        self,
        function_id: FunctionId,
        target: FunctionId,
        seen: &mut Vec<FunctionId>,
    ) -> bool {
        if seen.contains(&function_id) {
            return false;
        }
        seen.push(function_id);
        let Some(body) = self.gcx.hir.function(function_id).body else { return false };
        self.calls(body.stmts).iter().any(|&(callee, ..)| {
            callee == target
                || (is_internal(self.gcx.hir.function(callee))
                    && self.function_reaches(callee, target, seen))
        })
    }

    /// Whether `function_id` is a `_mint` whose execution skips the receiver check: the
    /// canonical OZ declaration (exact OZ contract name AND an OpenZeppelin source path, so a
    /// local contract reusing a name like `ERC721Consecutive` stays out), or a user override
    /// whose body calls a `_mint` that is itself unsafe (the capped/pausable pattern forwarding
    /// through `super._mint`). An override whose successful paths prove the recipient code-less
    /// or reject it after the delegation is a safe wrapper like canonical `_safeMint`. `seen`
    /// cuts override cycles, which never reach the canonical declaration.
    fn unsafe_mint_target(
        self,
        function_id: FunctionId,
        helper: bool,
        seen: &mut Vec<FunctionId>,
    ) -> Option<UnsafeMintTarget> {
        if seen.contains(&function_id) {
            return None;
        }
        seen.push(function_id);
        let function = self.gcx.hir.function(function_id);
        let is_mint = named(function, "_mint");
        if !(is_mint || (helper && is_internal(function))) {
            return None;
        }
        let contract = self.gcx.hir.contract(function.contract?);
        if contract.kind.is_library() {
            return None;
        }
        let canonical = is_canonical_erc721(contract.name.as_str())
            && source_in_package(&self.gcx.hir, function.source, OPENZEPPELIN_ROOTS);
        if canonical && named(function, "_safeMint") {
            return None;
        }
        // Most extensions (`ERC721Enumerable`, ...) inherit `_mint` rather than redeclare it, so
        // resolution still lands here.
        if canonical && is_mint {
            return Some(UnsafeMintTarget {
                preserves_recipient: true,
                preserves_token: true,
                preserves_code_length: true,
            });
        }
        if !(function.override_ || helper) {
            return None;
        }
        let body = function.body.as_ref()?;
        // The minted recipient is the override's first address-typed parameter.
        let recipient =
            function.parameters.iter().copied().find(|&vid| is_address_type(&self.gcx.hir, vid));
        let calls = self.calls(body.stmts);
        // Each distinct callee is judged once, with its own copy of `seen`: a cycle is a property
        // of one path, and two siblings sharing a transitive target would otherwise silence the
        // second.
        let mut unsafe_targets = Vec::new();
        let mut unstable_code_targets = Vec::new();
        let mut judged = Vec::new();
        let (mut targets_preserve_recipient, mut targets_preserve_token) = (true, true);
        for &(callee, ..) in &calls {
            if judged.contains(&callee) {
                continue;
            }
            judged.push(callee);
            if let Some(target) = self.unsafe_mint_target(callee, true, &mut seen.clone()) {
                unsafe_targets.push(callee);
                if !target.preserves_code_length {
                    unstable_code_targets.push(callee);
                }
                targets_preserve_recipient &= target.preserves_recipient;
                targets_preserve_token &= target.preserves_token;
            }
        }
        let delegations: Vec<_> =
            calls.iter().filter(|(callee, ..)| unsafe_targets.contains(callee)).collect();
        if delegations.is_empty() {
            return None;
        }
        // A guard covers the recipient it names, and no other. The recipient is what every
        // delegation binds to the callee's first parameter, the token what it binds to the
        // second, as the canonical `_mint(address to, uint256 tokenId)` orders them.
        let forwards = |index: usize, var: Option<VariableId>| {
            var.is_some_and(|var| {
                delegations.iter().all(|&&(callee, args, _)| {
                    self.arg(callee, args, index).and_then(underlying_var) == Some(var)
                })
            })
        };
        let only_to_recipient = forwards(0, recipient);
        // The token every delegation credits, when they all name the same variable: the
        // recipient may accept one token and refuse another, so a guard is only about the token
        // it was asked about. A token that is not a plain variable, or a mutable state variable
        // that an intervening call may move under the guard's feet, cannot be matched.
        let mut token = None;
        let mut token_consistent = true;
        for &&(callee, args, _) in &delegations {
            let minted = self.arg(callee, args, 1).and_then(underlying_var);
            match minted.filter(|&minted| keeps_its_value(self.gcx, minted)) {
                Some(minted) => {
                    token_consistent &= token.is_none_or(|token| token == minted);
                    token = Some(minted);
                }
                None => token_consistent = false,
            }
        }
        // Modifier expansion may supply a code-less proof before the body or a callback guard
        // after it. The body is still read in order because reassigning the recipient or token
        // can make either guard name a different value from the delegation.
        let guarded = |recipient, token, seed| {
            let mut walk = self.modifier_coverage_at_body(function, recipient, token, seed);
            let mut walker = GuardWalker {
                cx: self,
                recipient,
                token,
                delegations: &unsafe_targets,
                unstable_code_delegations: &unstable_code_targets,
                seen: &mut Vec::new(),
            };
            walker.walk(body.stmts, &mut walk);
            !walk.failed && !walk.pending
        };
        // A proof that the recipient has no code is independent of the token being minted, so
        // the recipient stands in both identity slots: a callback guard cannot type-check with
        // an address as its token. This lets an address-only helper or modifier establish
        // coverage and permits computed or remapped token arguments.
        if only_to_recipient
            && targets_preserve_recipient
            && let Some(recipient) = recipient
            && guarded(recipient, recipient, GuardCoverage::None)
        {
            return None;
        }
        if only_to_recipient
            && token_consistent
            && targets_preserve_recipient
            && targets_preserve_token
            && let Some(recipient) = recipient
            && let Some(token) = token
            && guarded(recipient, token, GuardCoverage::None)
        {
            return None;
        }
        // A guard in an outer override needs every intermediate override to preserve the
        // identities it relies on: the recipient for a code-less proof, and both recipient and
        // token for a callback. This summary is propagated only to callers; the current
        // override's own guard above may legitimately check a remapped local value.
        let preserves = |index: usize| {
            function.parameters.get(index).is_some_and(|&var| {
                !body.stmts.iter().any(|stmt| self.mutates_var(stmt, var))
                    && !function.modifiers.iter().any(|modifier| {
                        modifier.args.exprs().any(|arg| self.expr_mutates_var(arg, var))
                    })
                    && forwards(index, Some(var))
            })
        };
        // A caller's code-less proof remains valid through this override only when no path can
        // change account code before reaching a delegated mint.
        let preserves_code_length = recipient
            .is_some_and(|recipient| guarded(recipient, recipient, GuardCoverage::CodeLess));
        Some(UnsafeMintTarget {
            preserves_recipient: targets_preserve_recipient && preserves(0),
            preserves_token: targets_preserve_token && preserves(1),
            preserves_code_length,
        })
    }

    /// Runs `stmt_matches`/`expr_matches` over a subtree and reports whether either held.
    fn any_in_stmts(
        self,
        stmts: &'gcx [Stmt<'gcx>],
        stmt_matches: impl FnMut(&'gcx Stmt<'gcx>) -> bool,
        expr_matches: impl FnMut(&'gcx Expr<'gcx>) -> bool,
    ) -> bool {
        let mut finder = Finder { gcx: self.gcx, stmt_matches, expr_matches };
        stmts.iter().any(|stmt| finder.visit_stmt(stmt).is_break())
    }

    fn any_in_expr(
        self,
        expr: &'gcx Expr<'gcx>,
        expr_matches: impl FnMut(&'gcx Expr<'gcx>) -> bool,
    ) -> bool {
        Finder { gcx: self.gcx, stmt_matches: |_| false, expr_matches }.visit_expr(expr).is_break()
    }

    /// Every resolved call in a subtree, in source order.
    fn calls(self, stmts: &'gcx [Stmt<'gcx>]) -> Vec<Call<'gcx>> {
        let mut calls = Vec::new();
        self.any_in_stmts(
            stmts,
            |_| false,
            |expr| {
                if let ExprKind::Call(_, args, _) = &expr.kind
                    && let Some(function_id) = self.resolved_callee(expr)
                {
                    calls.push((function_id, args, expr.span));
                }
                false
            },
        );
        calls
    }

    /// The function a call expression dispatches to, as the type checker resolved it.
    fn resolved_callee(self, expr: &Expr<'_>) -> Option<FunctionId> {
        let ExprKind::Call(callee, ..) = &expr.kind else { return None };
        resolved_function(self.gcx, callee)
    }

    fn callee_fn(self, expr: &Expr<'_>) -> Option<&'gcx TyFn<'gcx>> {
        let ExprKind::Call(callee, ..) = &expr.kind else { return None };
        match self.gcx.type_of_expr(callee.peel_parens().id)?.kind {
            TyKind::Fn(function_ty) => Some(function_ty),
            _ => None,
        }
    }

    /// The declaration a call executes in the current EVM frame. A public function called by
    /// name is internal here, while `this.f()` and other external calls run in another frame
    /// whose assembly `return` cannot bypass the caller's later statements.
    fn resolved_internal_callee(self, expr: &Expr<'_>) -> Option<FunctionId> {
        let function_ty = self.callee_fn(expr)?;
        function_ty.is_internal().then_some(function_ty.function_id).flatten()
    }

    /// Whether a call dispatches through an internal function-pointer variable whose target is
    /// not available from the callee type. Such a target may contain assembly that leaves the
    /// frame, so exit analysis must treat the call conservatively.
    fn is_unresolved_internal_pointer_call(self, expr: &Expr<'_>) -> bool {
        let ExprKind::Call(callee, ..) = &expr.kind else { return false };
        self.callee_fn(expr).is_some_and(|f| f.is_internal() && f.function_id.is_none())
            && matches!(&callee.peel_parens().kind, ExprKind::Ident(reses)
                if reses.iter().any(|res| res.as_variable().is_some()))
    }

    /// The argument a call binds to the callee's parameter at `index`, positional or named.
    fn arg(
        self,
        function_id: FunctionId,
        args: &'gcx CallArgs<'gcx>,
        index: usize,
    ) -> Option<&'gcx Expr<'gcx>> {
        let function = self.gcx.hir.function(function_id);
        arg_for_param(&self.gcx.hir, function, *function.parameters.get(index)?, args)
    }

    /// Whether a resolved declaration is the ERC721 receiver hook: the exact name, the exact
    /// `(address, address, uint256, bytes)` shape, and an externally callable declaration of a
    /// non-library contract. A same-name function of an unrelated interface answers on a
    /// different selector, and an attached library or free function runs in the minting
    /// contract without any external call.
    fn is_receiver_hook(self, function_id: FunctionId) -> bool {
        let function = self.gcx.hir.function(function_id);
        let Some(contract) = function.contract else { return false };
        let &[from, to, id, data] = function.parameters else { return false };
        let kind = |vid: VariableId| &self.gcx.hir.variable(vid).ty.kind;
        named(function, "onERC721Received")
            && !self.gcx.hir.contract(contract).kind.is_library()
            && matches!(function.visibility, Visibility::Public | Visibility::External)
            && is_address_type(&self.gcx.hir, from)
            && is_address_type(&self.gcx.hir, to)
            && matches!(kind(id), TypeKind::Elementary(ElementaryType::UInt(_)))
            && matches!(kind(data), TypeKind::Elementary(ElementaryType::Bytes))
    }

    /// Whether an expression is the accepting answer, `onERC721Received`'s selector: the
    /// literal, a conversion of it, a `constant` holding it, or a `selector` member resolving to
    /// the receiver hook itself. The member is resolved rather than matched by name: spelled on
    /// a same-name function of another shape, `.selector` is a different value. An `immutable`
    /// or a state variable is unknown here and does not exempt.
    fn is_received_selector(self, expr: &Expr<'gcx>) -> bool {
        let expr = expr.peel_parens();
        match &expr.kind {
            ExprKind::Lit(lit) => {
                matches!(&lit.kind, LitKind::Number(value) if *value == U256::from(ERC721_RECEIVED))
            }
            ExprKind::Call(callee, args, _)
                if matches!(callee.peel_parens().kind, ExprKind::Type(..)) =>
            {
                args.len() == 1
                    && args.exprs().next().is_some_and(|inner| {
                        self.selector_cast_preserves(expr, inner)
                            && self.is_received_selector(inner)
                    })
            }
            ExprKind::Member(base, member) => {
                member.as_str() == "selector"
                    && resolved_function(self.gcx, base).is_some_and(|id| self.is_receiver_hook(id))
            }
            // A constant is worth what it holds.
            ExprKind::Ident(reses) => reses.iter().filter_map(Res::as_variable).any(|vid| {
                let variable = self.gcx.hir.variable(vid);
                variable.is_constant()
                    && variable.initializer.is_some_and(|init| self.is_received_selector(init))
            }),
            _ => false,
        }
    }

    /// Whether a cast preserves the recognized selector's value and byte alignment. A recognized
    /// integer is exactly the positive selector, so any integer width of at least 32 bits keeps
    /// it. Fixed bytes are left-aligned while integers are right-aligned, so crossing between
    /// them is only trusted at the four-byte boundary.
    fn selector_cast_preserves(self, cast: &Expr<'_>, inner: &Expr<'_>) -> bool {
        let encoding = |expr: &Expr<'_>| match self.gcx.type_of_expr(expr.peel_parens().id)?.kind {
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
                | (
                    Some(SelectorEncoding::FixedBytes(4..)),
                    Some(SelectorEncoding::FixedBytes(4..))
                )
        )
    }

    /// Whether executing `stmt` always reverts, undoing everything the transaction did. Only a
    /// revert counts, see [`Self::may_return`] for the escapes that leave the transaction
    /// standing.
    fn branch_always_reverts(self, stmt: &'gcx Stmt<'gcx>) -> bool {
        match &stmt.kind {
            StmtKind::Revert(_) => !self.may_return(stmt),
            StmtKind::Expr(expr) => is_revert_call(expr) && !self.may_return(stmt),
            // Read in order: a `revert` further down is only reached when nothing before it can
            // leave the function on its own.
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => block
                .stmts
                .iter()
                .find_map(|stmt| {
                    self.branch_always_reverts(stmt)
                        .then_some(true)
                        .or_else(|| self.may_return(stmt).then_some(false))
                })
                .unwrap_or(false),
            StmtKind::If(cond, then, Some(otherwise)) => {
                !self.expr_contains_frame_ending_assembly(cond)
                    && self.branch_always_reverts(then)
                    && self.branch_always_reverts(otherwise)
            }
            _ => false,
        }
    }

    /// Whether a statement may leave the function while keeping what the transaction already
    /// did: a `return`, or the EVM `return`/`stop` an assembly block can hold. Only statements
    /// that provably cannot leave answer no.
    fn may_return(self, stmt: &'gcx Stmt<'gcx>) -> bool {
        self.contains_frame_ending_assembly(slice::from_ref(stmt), &mut Vec::new())
            || match &stmt.kind {
                StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                    block.stmts.iter().any(|stmt| self.may_return(stmt))
                }
                StmtKind::Loop(block, source) => {
                    loop_stmts(*block, *source).any(|stmt| self.may_return(stmt))
                }
                StmtKind::If(_, then, otherwise) => {
                    self.may_return(then) || otherwise.is_some_and(|stmt| self.may_return(stmt))
                }
                StmtKind::Return(_)
                | StmtKind::AssemblyBlock(_)
                | StmtKind::Try(_)
                | StmtKind::Switch(_) => true,
                _ => false,
            }
    }

    /// Whether a subtree can reach an assembly block in the same EVM frame, directly or through
    /// an internal call. An assembly `return` leaves the frame without running a later revert or
    /// what an outer modifier holds after its placeholder. Every assembly block is treated as
    /// capable of doing so.
    fn contains_frame_ending_assembly(
        self,
        stmts: &'gcx [Stmt<'gcx>],
        seen: &mut Vec<FunctionId>,
    ) -> bool {
        self.any_in_stmts(stmts, is_assembly, |expr| self.call_leaves_frame(expr, seen))
    }

    fn expr_contains_frame_ending_assembly(self, expr: &'gcx Expr<'gcx>) -> bool {
        let mut seen = Vec::new();
        self.any_in_expr(expr, |expr| self.call_leaves_frame(expr, &mut seen))
    }

    fn call_leaves_frame(self, expr: &Expr<'_>, seen: &mut Vec<FunctionId>) -> bool {
        self.is_unresolved_internal_pointer_call(expr)
            || self
                .resolved_internal_callee(expr)
                .is_some_and(|id| self.callable_contains_frame_ending_assembly(id, seen))
    }

    /// Whether a same-frame callable or one of its applied modifiers can reach assembly. The
    /// recursion set is a path stack so independent calls are summarized independently.
    fn callable_contains_frame_ending_assembly(
        self,
        function_id: FunctionId,
        seen: &mut Vec<FunctionId>,
    ) -> bool {
        if seen.contains(&function_id) {
            return false;
        }
        seen.push(function_id);
        let function = self.gcx.hir.function(function_id);
        let in_modifiers = function.modifiers.iter().any(|modifier| {
            matches!(modifier.id, ItemId::Function(id)
                if self.callable_contains_frame_ending_assembly(id, seen))
        });
        let in_body = function
            .body
            .as_ref()
            .is_some_and(|body| self.contains_frame_ending_assembly(body.stmts, seen));
        seen.pop();
        in_modifiers || in_body
    }

    /// Whether a statement assigns to `var`: `var = x`, `var += x`, `var++`, `delete var`, or
    /// `var` as a component of a tuple assignment. An assembly block is treated as an opaque
    /// assignment, since it can rewrite Solidity locals outside the HIR expression tree.
    /// Identity is by variable, not by value, so a guard that checked `var` says nothing once
    /// `var` is reassigned.
    fn mutates_var(self, stmt: &'gcx Stmt<'gcx>, var: VariableId) -> bool {
        self.any_in_stmts(slice::from_ref(stmt), is_assembly, |expr| assigns_to(expr, var))
    }

    fn expr_mutates_var(self, expr: &'gcx Expr<'gcx>, var: VariableId) -> bool {
        self.any_in_expr(expr, |expr| assigns_to(expr, var))
    }

    /// Whether a subtree may change the code installed at an account. Inline assembly is opaque
    /// and may deploy code even when it contains no HIR call expression.
    fn stmts_may_change_account_code(
        self,
        stmts: &'gcx [Stmt<'gcx>],
        delegations: &[FunctionId],
        unstable_code_delegations: &[FunctionId],
        seen: &mut Vec<FunctionId>,
    ) -> bool {
        self.any_in_stmts(stmts, is_assembly, |expr| {
            self.call_may_change_account_code(expr, delegations, unstable_code_delegations, seen)
        })
    }

    fn expr_may_change_account_code(
        self,
        expr: &'gcx Expr<'gcx>,
        delegations: &[FunctionId],
        unstable_code_delegations: &[FunctionId],
        seen: &mut Vec<FunctionId>,
    ) -> bool {
        self.any_in_expr(expr, |expr| {
            self.call_may_change_account_code(expr, delegations, unstable_code_delegations, seen)
        })
    }

    /// Whether a call may change the code installed at an account. Pure and view calls are
    /// stable (external ones execute through `STATICCALL`), while nonpayable/payable calls and
    /// contract creation can run `CREATE`/`CREATE2`. Calls to the delegated mint itself are
    /// excluded unless it is recursively unstable: the code-length proof is needed precisely
    /// until that call begins.
    fn call_may_change_account_code(
        self,
        expr: &Expr<'_>,
        delegations: &[FunctionId],
        unstable_code_delegations: &[FunctionId],
        seen: &mut Vec<FunctionId>,
    ) -> bool {
        let ExprKind::Call(callee, ..) = &expr.kind else { return false };
        let resolved = self.resolved_callee(expr);
        if resolved.is_some_and(|id| delegations.contains(&id))
            && !resolved.is_some_and(|id| unstable_code_delegations.contains(&id))
        {
            return false;
        }
        if matches!(callee.peel_parens().kind, ExprKind::New(_)) {
            return true;
        }
        if !self.callee_fn(expr).is_some_and(|f| {
            matches!(f.state_mutability, StateMutability::NonPayable | StateMutability::Payable)
        }) {
            return false;
        }
        match self.resolved_internal_callee(expr).filter(|&id| !self.gcx.hir.function(id).virtual_)
        {
            Some(id) => self.callable_may_change_account_code(id, seen),
            None => true,
        }
    }

    /// Whether a statically known same-frame callable can create code, directly, through an
    /// applied modifier, or through another internal call. Recursion cycles alone do not create
    /// code; any opaque, virtual, or external state-changing call reached remains conservative.
    fn callable_may_change_account_code(
        self,
        function_id: FunctionId,
        seen: &mut Vec<FunctionId>,
    ) -> bool {
        if seen.contains(&function_id) {
            return false;
        }
        seen.push(function_id);
        let function = self.gcx.hir.function(function_id);
        let may_change = function.modifiers.iter().any(|modifier| {
            modifier.args.exprs().any(|arg| self.expr_may_change_account_code(arg, &[], &[], seen))
        }) || function.modifiers.iter().any(|modifier| {
            matches!(modifier.id, ItemId::Function(id)
                if self.callable_may_change_account_code(id, seen))
        }) || function
            .body
            .as_ref()
            .is_some_and(|body| self.stmts_may_change_account_code(body.stmts, &[], &[], seen));
        seen.pop();
        may_change
    }

    /// The callee parameters that receive the caller's recipient and token identities.
    fn bound_guard_parameters(
        self,
        function_id: FunctionId,
        args: &'gcx CallArgs<'gcx>,
        recipient: VariableId,
        token: VariableId,
    ) -> Option<(VariableId, VariableId)> {
        let parameters = self.gcx.hir.function(function_id).parameters;
        let bound_to = |var| {
            parameters
                .iter()
                .enumerate()
                .find(|&(index, _)| {
                    self.arg(function_id, args, index).and_then(underlying_var) == Some(var)
                })
                .map(|(_, &parameter)| parameter)
        };
        bound_to(recipient).zip(bound_to(token))
    }

    /// Guard coverage a callee's body establishes for the parameters the recipient and the token
    /// landed on: the callee guards when a guard ran before any possible successful exit. `seen`
    /// cuts recursion cycles.
    fn body_guards(
        self,
        function_id: FunctionId,
        recipient: VariableId,
        token: VariableId,
        seen: &mut Vec<FunctionId>,
    ) -> GuardCoverage {
        if seen.contains(&function_id) {
            return GuardCoverage::None;
        }
        seen.push(function_id);
        let function = self.gcx.hir.function(function_id);
        // A `virtual` callee may be replaced by an override that drops the guard, and a helper
        // carrying modifiers is not credited until their expansion is modeled: one may skip the
        // placeholder and let the helper return without ever running its body. The caller
        // relies on the values it passed in, so a body that mutates either bound parameter is
        // rejected too.
        let guarded = match &function.body {
            Some(body)
                if !function.virtual_
                    && function.modifiers.is_empty()
                    && !body.stmts.iter().any(|stmt| {
                        self.mutates_var(stmt, recipient) || self.mutates_var(stmt, token)
                    }) =>
            {
                let mut walk = GuardWalk::default();
                let mut walker = GuardWalker {
                    cx: self,
                    recipient,
                    token,
                    delegations: &[],
                    unstable_code_delegations: &[],
                    seen,
                };
                walker.walk(body.stmts, &mut walk);
                if walk.escaped {
                    GuardCoverage::None
                } else if walk.future_coverage == GuardCoverage::CodeLess {
                    GuardCoverage::CodeLess
                } else if walk.coverage == GuardCoverage::CodeLess {
                    // The code-less observation can discharge a mint that preceded the helper,
                    // but later work invalidated it for a mint after the helper. The mixed
                    // marker is treated by callers as discharge-only, like a callback.
                    GuardCoverage::CallbackOrCodeLess
                } else {
                    walk.coverage
                }
            }
            _ => GuardCoverage::None,
        };
        seen.pop();
        guarded
    }

    /// Coverage in effect when a function body starts after expanding its modifiers in
    /// declaration order. Prefixes are walked in execution order so calls in an inner modifier
    /// can retire an outer code-length snapshot. A proven tail guard is represented as stable
    /// callback coverage while walking the body: it runs after the body and can revert every
    /// mint the body made, unless assembly in the body or an inner modifier can bypass it.
    fn modifier_coverage_at_body(
        self,
        function: &'gcx hir::Function<'gcx>,
        recipient: VariableId,
        token: VariableId,
        seed: GuardCoverage,
    ) -> GuardWalk {
        let mut state = GuardWalk { coverage: seed, future_coverage: seed, ..GuardWalk::default() };
        let body_bypass = function
            .body
            .as_ref()
            .is_some_and(|body| self.contains_frame_ending_assembly(body.stmts, &mut Vec::new()));
        let mut has_tail_guard = false;
        for (index, modifier) in function.modifiers.iter().enumerate() {
            if modifier.args.exprs().any(|arg| {
                self.expr_mutates_var(arg, recipient) || self.expr_mutates_var(arg, token)
            }) {
                state.coverage = GuardCoverage::None;
                state.future_coverage = GuardCoverage::None;
                has_tail_guard = false;
            }
            state.retire_code_snapshots_if(|| {
                modifier
                    .args
                    .exprs()
                    .any(|arg| self.expr_may_change_account_code(arg, &[], &[], &mut Vec::new()))
            });
            let ItemId::Function(modifier_id) = modifier.id else { continue };
            let Some(body) = &self.gcx.hir.function(modifier_id).body else { continue };
            let Some((prefix, suffix)) = modifier_body_sides(body.stmts) else {
                // Without a single top-level placeholder, the precise prefix is unknown. Still
                // retire an inherited snapshot when any path through the modifier may change
                // code.
                state.retire_code_snapshots_if(|| {
                    self.stmts_may_change_account_code(body.stmts, &[], &[], &mut Vec::new())
                });
                continue;
            };
            let prefix_may_change_code =
                || self.stmts_may_change_account_code(prefix, &[], &[], &mut Vec::new());
            let Some((modifier_recipient, modifier_token)) =
                self.bound_guard_parameters(modifier_id, &modifier.args, recipient, token)
            else {
                state.retire_code_snapshots_if(prefix_may_change_code);
                continue;
            };
            let parameters_unchanged = !body.stmts.iter().any(|stmt| {
                self.mutates_var(stmt, modifier_recipient) || self.mutates_var(stmt, modifier_token)
            });
            let mut walker = GuardWalker {
                cx: self,
                recipient: modifier_recipient,
                token: modifier_token,
                delegations: &[],
                unstable_code_delegations: &[],
                seen: &mut Vec::new(),
            };
            if parameters_unchanged {
                walker.walk(prefix, &mut state);
            } else {
                state.retire_code_snapshots_if(prefix_may_change_code);
            }
            let inner_modifier_bypass = function.modifiers[index + 1..].iter().any(|inner| {
                matches!(inner.id, ItemId::Function(id)
                    if self.callable_contains_frame_ending_assembly(id, &mut Vec::new()))
            });
            if parameters_unchanged && !body_bypass && !inner_modifier_bypass {
                // The body has already minted when the suffix starts. Seed one pending
                // delegation: a guard anywhere before a successful suffix exit clears it, while a
                // call after that guard cannot make the earlier mint retroactively unsafe.
                let mut suffix_walk = GuardWalk { pending: true, ..GuardWalk::default() };
                walker.walk(suffix, &mut suffix_walk);
                has_tail_guard |= !suffix_walk.failed && !suffix_walk.pending;
            }
        }
        if has_tail_guard {
            state.cover(GuardCoverage::Callback, true);
        }
        GuardWalk {
            coverage: state.coverage,
            future_coverage: state.future_coverage,
            ..GuardWalk::default()
        }
    }
}

/// Breaks out of a subtree at the first statement or expression matching a predicate.
struct Finder<'gcx, S, E> {
    gcx: Gcx<'gcx>,
    stmt_matches: S,
    expr_matches: E,
}

impl<'gcx, S, E> Visit<'gcx> for Finder<'gcx, S, E>
where
    S: FnMut(&'gcx Stmt<'gcx>) -> bool,
    E: FnMut(&'gcx Expr<'gcx>) -> bool,
{
    type BreakValue = ();

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<()> {
        if (self.stmt_matches)(stmt) { ControlFlow::Break(()) } else { self.walk_stmt(stmt) }
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<()> {
        if (self.expr_matches)(expr) { ControlFlow::Break(()) } else { self.walk_expr(expr) }
    }
}

/// How a path established that the recipient can receive the mint. Callback evidence remains
/// valid when summarizing a guard helper, while a code-less proof must be retired once a call
/// could deploy code at the recipient address. Whether the evidence can cover a future mint is
/// tracked separately by [`GuardWalk::future_coverage`].
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum GuardCoverage {
    #[default]
    None,
    Callback,
    CodeLess,
    CallbackOrCodeLess,
}

impl GuardCoverage {
    fn is_covered(self) -> bool {
        self != Self::None
    }

    const fn relies_on_code_length(self) -> bool {
        matches!(self, Self::CodeLess | Self::CallbackOrCodeLess)
    }

    /// The coverage guaranteed after either branch. If one path relies on a code-less proof,
    /// the merged proof does too and remains invalidatable by a later call.
    const fn merge_paths(self, other: Self) -> Self {
        match (self, other) {
            (Self::None, _) | (_, Self::None) => Self::None,
            (Self::Callback, Self::Callback) => Self::Callback,
            (Self::CodeLess, Self::CodeLess) => Self::CodeLess,
            _ => Self::CallbackOrCodeLess,
        }
    }

    /// Coverage from guards that all execute. A callback check remains valid when a later call
    /// invalidates a separate code-length observation.
    const fn combine_guards(self, other: Self) -> Self {
        match (self, other) {
            (Self::Callback, _) | (_, Self::Callback) => Self::Callback,
            (Self::CallbackOrCodeLess, _) | (_, Self::CallbackOrCodeLess) => {
                Self::CallbackOrCodeLess
            }
            (Self::CodeLess, _) | (_, Self::CodeLess) => Self::CodeLess,
            _ => Self::None,
        }
    }
}

/// The straight-line reading of a body: `coverage` once a guard has run on the path, `pending`
/// while a delegated mint has run with no guard before or after it yet, `failed` once a path
/// may leave the function successfully with such a mint standing, `escaped` once one may leave
/// before any guard ran.
#[derive(Clone, Default)]
struct GuardWalk {
    /// The guards that have run on every path. This is used when summarizing a guard helper.
    coverage: GuardCoverage,
    /// Coverage that can satisfy a delegation encountered later. A code-less proof can precede
    /// the mint; callback coverage can only appear here when a modifier tail guarantees that the
    /// callback runs after the body.
    future_coverage: GuardCoverage,
    pending: bool,
    failed: bool,
    escaped: bool,
}

impl GuardWalk {
    /// A guard ran: `coverage` is established, and also for later delegations when `future`.
    const fn cover(&mut self, coverage: GuardCoverage, future: bool) {
        self.coverage = self.coverage.combine_guards(coverage);
        if future {
            self.future_coverage = self.future_coverage.combine_guards(coverage);
        }
        self.pending = false;
    }

    /// The recipient or token was reassigned: every guard so far checked a value a later
    /// delegation no longer credits, and a mint already pending cannot be covered by a guard to
    /// come either.
    const fn retire(&mut self) {
        self.failed |= self.pending;
        self.coverage = GuardCoverage::None;
        self.future_coverage = GuardCoverage::None;
    }

    /// The path may leave the function successfully here.
    fn escape(&mut self) {
        self.failed |= self.pending;
        self.escaped |= !self.coverage.is_covered();
    }

    /// Retires code-length snapshots when `may_change_code()` holds; a callback acknowledgement
    /// is not a snapshot and stays. The check only runs when a snapshot exists.
    fn retire_code_snapshots_if(&mut self, may_change_code: impl FnOnce() -> bool) {
        let (coverage, future) =
            (self.coverage.relies_on_code_length(), self.future_coverage.relies_on_code_length());
        if (coverage || future) && may_change_code() {
            if coverage {
                self.coverage = GuardCoverage::None;
            }
            if future {
                self.future_coverage = GuardCoverage::None;
            }
        }
    }

    /// The state after either of two branches: coverage holds only when every path checked,
    /// while a pending or escaping path taints the whole.
    const fn merge(self, other: Self) -> Self {
        Self {
            coverage: self.coverage.merge_paths(other.coverage),
            future_coverage: self.future_coverage.merge_paths(other.future_coverage),
            pending: self.pending || other.pending,
            failed: self.failed || other.failed,
            escaped: self.escaped || other.escaped,
        }
    }
}

/// Reads a body in statement order and judges the delegated mints against the guards for
/// `recipient` and `token`. A code-less proof may cover a later delegation, but a callback must
/// run after ownership is established to match `_safeMint`: the receiver can inspect `ownerOf`,
/// balances, or reenter during the hook. Such a callback covers delegations still pending, the
/// revert undoing them, unless a statement in between may leave the function successfully,
/// keeping the unacknowledged token: `super._mint(to, id); if (id == 0) return; require(hook...)`
/// walks out with token zero standing.
///
/// The recognized guard shapes are a closed set, because a hook call that merely appears inside
/// a condition proves nothing about whether the revert depends on its answer. They are:
/// `require`/`assert` on an acceptance condition, `if (hook != selector) <exits>`,
/// `if (hook == selector) {} else <exits>`, and any of those reached through a function or
/// modifier. A callback helper receives both identities, the way OpenZeppelin factors
/// `_checkOnERC721Received` out of `_safeMint`; a code-less proof needs only the recipient.
///
/// Branches are read separately and merged. The branch a `to.code.length` test dedicates to
/// accounts starts covered, an account always accepting the token. A loop body may run zero
/// times, so nothing in one is credited, while the delegations and escapes it may hold still
/// count.
///
/// Everything else reports, a `try` whose `catch` may swallow the refusal included, and so are
/// an answer stored in a local and a helper returning it as a `bool`. Following the value
/// across statements would take a dataflow analysis this detector does not run.
struct GuardWalker<'a, 'gcx> {
    cx: Cx<'gcx>,
    recipient: VariableId,
    token: VariableId,
    /// The callees that are unsafe mints, and those among them that may change account code.
    delegations: &'a [FunctionId],
    unstable_code_delegations: &'a [FunctionId],
    seen: &'a mut Vec<FunctionId>,
}

impl<'gcx> GuardWalker<'_, 'gcx> {
    fn walk(&mut self, stmts: &'gcx [Stmt<'gcx>], walk: &mut GuardWalk) {
        let cx = self.cx;
        // Read in order: what a guard covers and what an exit walks out with depend on what
        // already ran.
        for stmt in stmts {
            let guard = match &stmt.kind {
                StmtKind::Expr(expr) => self.guard_expr_coverage(expr),
                _ => GuardCoverage::None,
            };
            match &stmt.kind {
                StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                    self.walk(block.stmts, walk);
                }
                StmtKind::Expr(expr) if guard.is_covered() => {
                    // A guard that also reassigns the recipient or the token cannot be trusted:
                    // evaluation order decides whether the hook read the value the mint credits.
                    // Nor can a guard that may leave the frame establish coverage: an assembly
                    // return in another argument can keep a pending mint before the builtin has
                    // a chance to revert.
                    if self.mutates(stmt) {
                        walk.retire();
                    } else if cx.may_return(stmt) {
                        walk.escape();
                    } else if guard.relies_on_code_length()
                        && self.guard_extra_args_may_change_account_code(expr)
                    {
                        if walk.future_coverage.relies_on_code_length() {
                            walk.future_coverage = GuardCoverage::None;
                        }
                    } else {
                        walk.cover(guard, guard == GuardCoverage::CodeLess);
                    }
                }
                StmtKind::If(cond, then, otherwise) => {
                    // The condition runs before either branch, so an assignment embedded in it,
                    // `if ((tokenId = tokenId + 1) > 0) {}`, retires coverage exactly as a bare
                    // assignment statement does, and prevents the comparison from covering.
                    let condition_mutates = cx.expr_mutates_var(cond, self.recipient)
                        || cx.expr_mutates_var(cond, self.token);
                    if condition_mutates {
                        walk.retire();
                    }
                    if walk.future_coverage.relies_on_code_length()
                        && self.may_change_account_code(slice::from_ref(stmt), Some(cond))
                    {
                        walk.future_coverage = GuardCoverage::None;
                    }
                    if cx.expr_contains_frame_ending_assembly(cond) {
                        walk.escape();
                    }
                    // The exiting branch must be the one a refusal takes, not the one an
                    // acceptance does. Continue reading the accepted branch from the covered
                    // state: it may still reassign the checked values before delegating.
                    let refusal_then = !condition_mutates
                        && self.is_hook_comparison(cond, BinOpKind::Ne)
                        && cx.branch_always_reverts(then);
                    let refusal_else = !condition_mutates
                        && self.is_hook_comparison(cond, BinOpKind::Eq)
                        && otherwise.is_some_and(|otherwise| cx.branch_always_reverts(otherwise));
                    if refusal_then || refusal_else {
                        walk.cover(GuardCoverage::Callback, false);
                        let accepted = if refusal_then { *otherwise } else { Some(*then) };
                        if let Some(accepted) = accepted {
                            self.walk(slice::from_ref(accepted), walk);
                        }
                        continue;
                    }
                    // Each branch is read on its own, from what already ran. The branch a
                    // `to.code.length` test dedicates to accounts starts covered with nothing
                    // pending: an account always accepts, so the mints already made are as
                    // satisfied on that path as the ones to come.
                    let mut then_walk = walk.clone();
                    let mut else_walk = walk.clone();
                    if self.is_code_length_test(cond, true) {
                        else_walk.cover(GuardCoverage::CodeLess, true);
                    } else if self.is_code_length_test(cond, false) {
                        then_walk.cover(GuardCoverage::CodeLess, true);
                    }
                    self.walk(slice::from_ref(then), &mut then_walk);
                    if let Some(otherwise) = otherwise {
                        self.walk(slice::from_ref(otherwise), &mut else_walk);
                    }
                    *walk = then_walk.merge(else_walk);
                }
                _ => {
                    // Reassignment runs first: an assignment inside a delegation's own arguments
                    // happens before the call.
                    if self.mutates(stmt) {
                        walk.retire();
                    }
                    // Unlike a callback acknowledgement, a code-length observation is only a
                    // snapshot. A later state-changing call can deploy code at that address, so
                    // it retires coverage for a subsequent delegation. A mint already discharged
                    // by the observation remains safe.
                    if walk.future_coverage.relies_on_code_length()
                        && self.may_change_account_code(slice::from_ref(stmt), None)
                    {
                        walk.future_coverage = GuardCoverage::None;
                    }
                    // An opaque statement: a delegation anywhere inside it mints, unchecked
                    // unless already covered, and a possible successful exit walks out with
                    // whatever is pending.
                    let delegations = self.delegations;
                    if !walk.future_coverage.is_covered()
                        && cx.any_in_stmts(
                            slice::from_ref(stmt),
                            |_| false,
                            |expr| {
                                cx.resolved_callee(expr).is_some_and(|id| delegations.contains(&id))
                            },
                        )
                    {
                        walk.pending = true;
                    }
                    if cx.may_return(stmt) {
                        walk.escape();
                    }
                }
            }
        }
    }

    fn mutates(&self, stmt: &'gcx Stmt<'gcx>) -> bool {
        self.cx.mutates_var(stmt, self.recipient) || self.cx.mutates_var(stmt, self.token)
    }

    /// [`Cx::stmts_may_change_account_code`] for the walked delegations, over a statement or
    /// only the given expression of it.
    fn may_change_account_code(
        &self,
        stmts: &'gcx [Stmt<'gcx>],
        expr: Option<&'gcx Expr<'gcx>>,
    ) -> bool {
        let (delegations, unstable) = (self.delegations, self.unstable_code_delegations);
        let mut seen = Vec::new();
        match expr {
            Some(expr) => {
                self.cx.expr_may_change_account_code(expr, delegations, unstable, &mut seen)
            }
            None => self.cx.stmts_may_change_account_code(stmts, delegations, unstable, &mut seen),
        }
    }

    /// A statement expression that guards the recipient and the token: `require`/`assert` on an
    /// acceptance condition, or an internal call handing both to a helper that does. Only the
    /// condition is read: a hook call sitting in the revert message decides nothing. An external
    /// helper would ask from a different contract and cannot establish that the recipient
    /// accepts the minting contract's callback.
    fn guard_expr_coverage(&mut self, expr: &'gcx Expr<'gcx>) -> GuardCoverage {
        let expr = expr.peel_parens();
        let ExprKind::Call(callee, args, _) = &expr.kind else { return GuardCoverage::None };
        if is_require_or_assert(callee) {
            return args
                .exprs()
                .next()
                .map_or(GuardCoverage::None, |cond| self.acceptance_coverage(cond));
        }
        let Some(function_id) = self.cx.resolved_internal_callee(expr) else {
            return GuardCoverage::None;
        };
        let Some((recipient, token)) =
            self.cx.bound_guard_parameters(function_id, args, self.recipient, self.token)
        else {
            return GuardCoverage::None;
        };
        self.cx.body_guards(function_id, recipient, token, self.seen)
    }

    /// Whether a recognized `require`/`assert` has another argument that may change account
    /// code. The first argument is the closed-form acceptance condition itself; its receiver
    /// callback is part of the proof. Solidity does not guarantee that the other arguments run
    /// before the condition's code-length snapshot.
    fn guard_extra_args_may_change_account_code(&self, expr: &'gcx Expr<'gcx>) -> bool {
        let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
        is_require_or_assert(callee)
            && args.exprs().skip(1).any(|arg| self.may_change_account_code(&[], Some(arg)))
    }

    /// The condition of a `require`/`assert` that passes only if the recipient can receive the
    /// token: the recipient is proven code-less, the hook comparison succeeds, or the short
    /// circuit `to.code.length == 0 || hook == sel` accepts either case.
    fn acceptance_coverage(&self, cond: &'gcx Expr<'gcx>) -> GuardCoverage {
        let cond = cond.peel_parens();
        if self.is_code_length_test(cond, false) {
            return GuardCoverage::CodeLess;
        }
        if self.is_hook_comparison(cond, BinOpKind::Eq) {
            return GuardCoverage::Callback;
        }
        let ExprKind::Binary(lhs, op, rhs) = &cond.kind else { return GuardCoverage::None };
        let accepts = |skip, check| {
            self.is_code_length_test(skip, false) && self.is_hook_comparison(check, BinOpKind::Eq)
        };
        if op.kind == BinOpKind::Or && (accepts(lhs, rhs) || accepts(rhs, lhs)) {
            GuardCoverage::CallbackOrCodeLess
        } else {
            GuardCoverage::None
        }
    }

    /// `recipient.onERC721Received(..., token, ...)`: the hook, asked of the recipient itself and
    /// about the delegated token itself. An answer about a different id decides nothing for the
    /// minted one.
    fn is_hook_call_on(&self, expr: &'gcx Expr<'gcx>) -> bool {
        let expr = expr.peel_parens();
        let ExprKind::Call(callee, args, _) = &expr.kind else { return false };
        let ExprKind::Member(receiver, _) = &callee.peel_parens().kind else { return false };
        let Some(function_id) = self.cx.resolved_callee(expr) else { return false };
        self.cx.is_receiver_hook(function_id)
            && underlying_var(receiver) == Some(self.recipient)
            && self.cx.arg(function_id, args, 2).and_then(underlying_var) == Some(self.token)
    }

    /// `recipient.onERC721Received(...) <op> x`, and nothing else. The comparison must be the
    /// whole expression: in `to == trusted || hook(to) == selector` the hook never runs for
    /// `trusted`. The other operand must be able to hold the accepting answer, and must not be a
    /// hook call itself, which would compare the recipient against itself.
    fn is_hook_comparison(&self, expr: &'gcx Expr<'gcx>, want: BinOpKind) -> bool {
        let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind else { return false };
        let compares = |hook, answer| {
            self.is_hook_call_on(hook)
                && !self.is_hook_call_on(answer)
                && self.cx.is_received_selector(answer)
        };
        op.kind == want && (compares(lhs, rhs) || compares(rhs, lhs))
    }

    /// `recipient.code.length` compared against zero, for one polarity. Nothing else may ride
    /// along: in `to.code.length > 0 && id == 5` the second operand decides whether the branch
    /// runs.
    fn is_code_length_test(&self, expr: &'gcx Expr<'gcx>, has_code: bool) -> bool {
        let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind else { return false };
        let is_code_length = |expr: &Expr<'_>| {
            let ExprKind::Member(code, length) = &expr.peel_parens().kind else { return false };
            let ExprKind::Member(base, member) = &code.peel_parens().kind else { return false };
            length.as_str() == "length"
                && member.as_str() == "code"
                && underlying_var(base) == Some(self.recipient)
        };
        let literal = |expr: &Expr<'_>| match &expr.peel_parens().kind {
            ExprKind::Lit(lit) => match &lit.kind {
                LitKind::Number(value) => u8::try_from(*value).ok(),
                _ => None,
            },
            _ => None,
        };
        let (bound, flipped) = if is_code_length(lhs) {
            (literal(rhs), false)
        } else if is_code_length(rhs) {
            (literal(lhs), true)
        } else {
            return false;
        };
        let Some(bound) = bound else { return false };
        // `length > 0`, `length != 0` and `length >= 1` all say the recipient carries code;
        // `== 0`, `< 1` and `<= 0` all say it carries none. Each has a mirror with the operands
        // swapped.
        match (has_code, op.kind, flipped) {
            (true, BinOpKind::Ne, _)
            | (true, BinOpKind::Gt, false)
            | (true, BinOpKind::Lt, true)
            | (false, BinOpKind::Eq, _)
            | (false, BinOpKind::Le, false)
            | (false, BinOpKind::Ge, true) => bound == 0,
            (true, BinOpKind::Ge, false)
            | (true, BinOpKind::Le, true)
            | (false, BinOpKind::Lt, false)
            | (false, BinOpKind::Gt, true) => bound == 1,
            _ => false,
        }
    }
}

/// The representation of a selector-sized constant at one conversion step.
#[derive(Clone, Copy)]
enum SelectorEncoding {
    Literal,
    Integer(u16),
    FixedBytes(u8),
}

/// The ERC721 answer meaning the recipient accepts the token, `onERC721Received`'s selector.
const ERC721_RECEIVED: u64 = 0x150b_7a02;

/// The OpenZeppelin contracts whose `_mint` skips the receiver check. `ERC721` and
/// `ERC721Upgradeable` declare the unchecked `_mint`; in the v4 line, `ERC721Consecutive` and
/// `ERC721ConsecutiveUpgradeable` override it with a construction guard that forwards to the
/// base through `super._mint`, still without a receiver check. In v5 the Consecutive extension
/// overrides `_update` instead, and the two extra names match nothing.
fn is_canonical_erc721(name: &str) -> bool {
    matches!(
        name,
        "ERC721" | "ERC721Upgradeable" | "ERC721Consecutive" | "ERC721ConsecutiveUpgradeable"
    )
}

fn named(function: &hir::Function<'_>, name: &str) -> bool {
    function.name.is_some_and(|n| n.as_str() == name)
}

const fn is_internal(function: &hir::Function<'_>) -> bool {
    matches!(function.visibility, Visibility::Internal | Visibility::Private)
}

const fn is_assembly(stmt: &Stmt<'_>) -> bool {
    matches!(stmt.kind, StmtKind::AssemblyBlock(_))
}

/// Whether the variable cannot change between the delegation and the callback guard. An
/// intervening call can reenter and mutate a state variable after the mint reads it but before
/// the guard does. A local, a parameter, a `constant` or an `immutable` cannot be moved that way.
fn keeps_its_value(gcx: Gcx<'_>, variable: VariableId) -> bool {
    let variable = gcx.hir.variable(variable);
    !variable.kind.is_state() || variable.mutability.is_some()
}

/// Whether an expression writes `var`: `var = x`, `var += x`, `var++` or `delete var`, directly
/// or as one component of a tuple.
fn assigns_to(expr: &Expr<'_>, var: VariableId) -> bool {
    let Some(target) = write_target(expr) else { return false };
    let mut hit = false;
    for_each_lhs_var(target, &mut |vid| hit |= vid == var);
    hit
}

/// `revert(...)`, `require(false, ...)` and `assert(false)`.
fn is_revert_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
    is_builtin(callee, kw::Revert)
        || (is_require_or_assert(callee) && args.exprs().next().is_some_and(is_literal_false))
}

/// The statements before and after a modifier's single top-level placeholder. More complicated
/// expansion shapes are left uncredited rather than guessing which paths execute the body.
fn modifier_body_sides<'gcx>(
    stmts: &'gcx [Stmt<'gcx>],
) -> Option<(&'gcx [Stmt<'gcx>], &'gcx [Stmt<'gcx>])> {
    let placeholders =
        stmts.iter().enumerate().filter(|(_, stmt)| matches!(stmt.kind, StmtKind::Placeholder));
    let index = unique(placeholders.map(|(index, _)| index))?;
    Some((&stmts[..index], &stmts[index + 1..]))
}
