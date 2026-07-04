use super::UnsafeOzErc721Mint;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx,
    hir::{self, Expr, ExprKind, FunctionId, Hir, Visit},
    ty::TyKind,
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
        {
            return;
        }
        // A user `_mint` override is part of the mint primitive itself: `super._mint` there is
        // delegation (the capped/pausable pattern), the receiver check belongs to the
        // override's callers, and `_safeMint` there would re-enter the override through the
        // virtual dispatch.
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

    /// Whether `function_id` is a function named `_mint` declared in a non-library contract
    /// named exactly like an OZ ERC721 base. Extensions (`ERC721Enumerable`, ...) inherit
    /// `_mint` rather than redeclare it, so resolution still lands on the canonical base; exact
    /// names avoid flagging a safe override just because its contract name contains the
    /// substring `ERC721`.
    fn is_erc721_mint(&self, function_id: FunctionId) -> bool {
        let function = self.hir.function(function_id);
        function.name.is_some_and(|name| name.as_str() == "_mint")
            && function.contract.is_some_and(|id| {
                let contract = self.hir.contract(id);
                !contract.kind.is_library() && is_canonical_erc721(contract.name.as_str())
            })
    }
}

/// The OpenZeppelin contracts that declare the unchecked `_mint`.
fn is_canonical_erc721(name: &str) -> bool {
    matches!(name, "ERC721" | "ERC721Upgradeable")
}
