use super::UnsafeOzErc721Mint;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::ElementaryType,
    interface::source_map::FileName,
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
        // override that performs the receiver check itself before forwarding (a resolved
        // `onERC721Received` call or an `address.code` inspection) is a safe wrapper like
        // the canonical `_safeMint`, so it is not an unsafe target.
        if function.override_
            && let Some(body) = &function.body
        {
            let mut scan = BodyScanner {
                gcx: self.gcx,
                hir: self.hir,
                callees: Vec::new(),
                has_receiver_check: false,
            };
            // The body's resolved callees and check markers are collected first.
            for stmt in body.stmts {
                let _ = scan.visit_stmt(stmt);
            }
            // Each callee is judged with the shared `seen` set, so override cycles stop.
            if !scan.has_receiver_check {
                for callee in scan.callees {
                    if self.is_unsafe_mint_target(callee, seen) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Whether a source file belongs to an OpenZeppelin package, judged by its path. A
/// vendored copy under a path that does not name OpenZeppelin is not recognized.
fn is_openzeppelin_source(hir: &Hir<'_>, source_id: hir::SourceId) -> bool {
    match &hir.source(source_id).file.name {
        FileName::Real(path) => path.to_string_lossy().to_lowercase().contains("openzeppelin"),
        _ => false,
    }
}

/// Collects the resolved targets of every call in a subtree, and whether the subtree
/// performs a receiver check: a call resolving to a function named `onERC721Received`, or
/// a `.code` inspection on an address value.
struct BodyScanner<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    callees: Vec<FunctionId>,
    has_receiver_check: bool,
}

impl<'hir> Visit<'hir> for BodyScanner<'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            // Each call's dispatch target, as the type checker resolved it. A callee named
            // `onERC721Received` is the receiver check itself.
            ExprKind::Call(callee, _, _) => {
                if let Some(ty) = self.gcx.type_of_expr(callee.peel_parens().id)
                    && let TyKind::Fn(function_ty) = ty.kind
                    && let Some(function_id) = function_ty.function_id
                {
                    self.callees.push(function_id);
                    // The name is judged on the resolved declaration, not the call site.
                    if self
                        .hir
                        .function(function_id)
                        .name
                        .is_some_and(|name| name.as_str() == "onERC721Received")
                    {
                        self.has_receiver_check = true;
                    }
                }
            }
            // `to.code.length` style inspections: the `code` member of an address value,
            // judged on the base's type so a user field named `code` does not count.
            ExprKind::Member(base, member) => {
                if member.as_str() == "code"
                    && let Some(ty) = self.gcx.type_of_expr(base.peel_parens().id)
                    && matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::Address(_)))
                {
                    self.has_receiver_check = true;
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
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
