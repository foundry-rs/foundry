use super::UnsafeOzErc721Mint;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::sym,
    sema::{
        Gcx,
        hir::{self, ContractId, Expr, ExprKind, FunctionId, Hir, ItemId, Res, Visit},
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
        _gcx: Gcx<'hir>,
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
            let bases = match func.contract {
                Some(contract_id) => hir.contract(contract_id).linearized_bases,
                None => &[],
            };
            let mut finder = MintCallFinder { hir, ctx, bases };
            for stmt in body.stmts {
                let _ = finder.visit_stmt(stmt);
            }
        }
    }
}

struct MintCallFinder<'ctx, 's, 'c, 'hir> {
    hir: &'hir Hir<'hir>,
    ctx: &'ctx LintContext<'s, 'c>,
    /// The linearization of the enclosing contract, used to resolve `super._mint(...)`.
    bases: &'hir [ContractId],
}

impl<'hir> Visit<'hir> for MintCallFinder<'_, '_, '_, 'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        // Flag the call as soon as one resolution the call can dispatch to is the `_mint` of an
        // ERC721 contract. Overload sets are filtered by arity: a one-argument call cannot reach
        // the two-parameter base `_mint`, only a same-arity local overload.
        if let ExprKind::Call(callee, args, _) = &expr.kind
            && self
                .resolved_function_ids(callee)
                .into_iter()
                .any(|id| self.is_erc721_mint(id, args.len()))
        {
            self.ctx.emit(&UNSAFE_OZ_ERC721_MINT, expr.span);
        }
        self.walk_expr(expr)
    }
}

impl MintCallFinder<'_, '_, '_, '_> {
    /// Whether `function_id` is a function named `_mint`, callable with `arg_count` arguments,
    /// declared in a non-library contract whose name contains `ERC721` (covers `ERC721`,
    /// `ERC721Upgradeable`, `ERC721Enumerable`, ...). Libraries are excluded: OpenZeppelin's
    /// unchecked `_mint` lives in the `ERC721` contract, not in a library.
    fn is_erc721_mint(&self, function_id: FunctionId, arg_count: usize) -> bool {
        let function = self.hir.function(function_id);
        function.name.is_some_and(|name| name.as_str() == "_mint")
            && function.parameters.len() == arg_count
            && function.contract.is_some_and(|id| {
                let contract = self.hir.contract(id);
                !contract.kind.is_library() && contract.name.as_str().contains("ERC721")
            })
    }

    /// Resolves the functions a callee can designate: a resolved identifier (`_mint(...)`,
    /// inherited functions included), `super._mint(...)` through the linearization, or the
    /// qualified `ERC721._mint(...)`.
    fn resolved_function_ids(&self, callee: &hir::Expr<'_>) -> Vec<FunctionId> {
        match &callee.peel_parens().kind {
            ExprKind::Ident(resolutions) => resolutions
                .iter()
                .filter_map(|res| match res {
                    Res::Item(ItemId::Function(function_id)) => Some(*function_id),
                    _ => None,
                })
                .collect(),
            ExprKind::Member(base, method) => {
                let ExprKind::Ident(resolutions) = &base.peel_parens().kind else {
                    return Vec::new();
                };
                // `super.f()` dispatches into the linearization past the contract itself.
                let is_super = resolutions.iter().any(
                    |res| matches!(res, Res::Builtin(builtin) if builtin.name() == sym::super_),
                );

                let contracts: Vec<ContractId> = if is_super {
                    self.bases.get(1..).unwrap_or_default().to_vec()
                } else {
                    resolutions
                        .iter()
                        .filter_map(|res| match res {
                            Res::Item(ItemId::Contract(contract_id)) => Some(*contract_id),
                            _ => None,
                        })
                        .collect()
                };

                contracts
                    .into_iter()
                    .flat_map(|contract_id| self.hir.contract(contract_id).all_functions())
                    .filter(|&function_id| {
                        self.hir
                            .function(function_id)
                            .name
                            .is_some_and(|name| name.as_str() == method.as_str())
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }
}
