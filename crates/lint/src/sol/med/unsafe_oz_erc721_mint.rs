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
        // A `_safeMint` implementation is the wrapper itself: it legitimately calls `_mint`
        // after (or before) its receiver check.
        if func.name.is_some_and(|name| name.as_str() == "_safeMint") {
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

    /// Whether `function_id` is a function named `_mint` declared in a non-library contract whose
    /// name contains `ERC721` (covers `ERC721`, `ERC721Upgradeable`, `ERC721Enumerable`, ...).
    /// Libraries are excluded: OpenZeppelin's unchecked `_mint` lives in the `ERC721` contract.
    fn is_erc721_mint(&self, function_id: FunctionId) -> bool {
        let function = self.hir.function(function_id);
        function.name.is_some_and(|name| name.as_str() == "_mint")
            && function.contract.is_some_and(|id| {
                let contract = self.hir.contract(id);
                !contract.kind.is_library() && contract.name.as_str().contains("ERC721")
            })
    }
}
