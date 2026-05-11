use super::OptimismDeprecation;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::LitKind,
    sema::hir::{self, ExprKind, ItemId, Res},
};

declare_forge_lint!(
    OPTIMISM_DEPRECATION,
    Severity::Low,
    "optimism-deprecation",
    "usage of a deprecated Optimism predeploy address or GasPriceOracle function that reverts post-Ecotone"
);

/// Addresses of predeploys that were fully removed in the Bedrock upgrade.
const DEPRECATED_PREDEPLOYS: &[&str] = &[
    "0x4200000000000000000000000000000000000000", // LegacyMessagePasser
    "0x4200000000000000000000000000000000000001", // L1MessageSender
    "0x4200000000000000000000000000000000000002", // DeployerWhitelist
    "0x4200000000000000000000000000000000000013", // L1BlockNumber
];

/// GasPriceOracle predeploy (`0x420000000000000000000000000000000000000F`), still deployed but
/// `overhead`, `scalar`, and `getL1GasUsed` revert unconditionally since the Ecotone upgrade.
const GPO_ADDRESS: &str = "0x420000000000000000000000000000000000000f";

const DEPRECATED_GPO_FUNCTIONS: &[&str] = &["overhead", "scalar", "getL1GasUsed"];

impl<'hir> LateLintPass<'hir> for OptimismDeprecation {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        match &expr.kind {
            // Deprecated predeploy address literal.
            ExprKind::Lit(lit)
                if matches!(lit.kind, LitKind::Address(_))
                    && is_deprecated_predeploy(lit.symbol.as_str()) =>
            {
                ctx.emit(&OPTIMISM_DEPRECATION, lit.span);
            }

            // Deprecated GasPriceOracle function call.
            ExprKind::Call(callee, _, _) => {
                let ExprKind::Member(receiver, member) = &callee.kind else { return };
                if DEPRECATED_GPO_FUNCTIONS.contains(&member.as_str())
                    && receiver_resolves_to_gpo(hir, receiver)
                {
                    ctx.emit(&OPTIMISM_DEPRECATION, expr.span);
                }
            }

            _ => {}
        }
    }
}

fn is_deprecated_predeploy(sym: &str) -> bool {
    let lower = sym.to_lowercase();
    DEPRECATED_PREDEPLOYS.iter().any(|&addr| addr == lower)
}

/// Returns true if `expr` evaluates to the GasPriceOracle predeploy address.
///
/// Handles direct literals, contract type casts (`IGasPriceOracle(0x...000F)`), and
/// single-assignment local variable aliases. Does not track reassignment or state variables.
fn receiver_resolves_to_gpo(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    match &expr.kind {
        // Direct address literal.
        ExprKind::Lit(lit) => {
            matches!(lit.kind, LitKind::Address(_))
                && lit.symbol.as_str().to_lowercase() == GPO_ADDRESS
        }

        // `IGasPriceOracle(0x...000F)`: callee resolves to a Contract type cast.
        // This is unambiguously a cast (not a function call) because the callee is a Contract.
        ExprKind::Call(callee, args, _) => {
            if !matches!(
                callee.kind,
                ExprKind::Ident([Res::Item(ItemId::Contract(_)), ..]) | ExprKind::Type(_)
            ) {
                return false;
            }
            let mut iter = args.exprs();
            match (iter.next(), iter.next()) {
                (Some(arg), None) => receiver_resolves_to_gpo(hir, arg),
                _ => false,
            }
        }

        // Variable reference: follow single-assignment locals back to their initializer.
        ExprKind::Ident(reses) => {
            for res in *reses {
                if let Res::Item(ItemId::Variable(vid)) = res {
                    let var = hir.variable(*vid);
                    if var.kind.is_state() {
                        continue;
                    }
                    if let Some(init) = var.initializer
                        && receiver_resolves_to_gpo(hir, init)
                    {
                        return true;
                    }
                }
            }
            false
        }

        // Tuple elements (uncommon, but handle gracefully).
        ExprKind::Tuple(elems) => {
            elems.iter().any(|e| e.is_some_and(|inner| receiver_resolves_to_gpo(hir, inner)))
        }

        _ => false,
    }
}
