use super::RedundantBaseConstructorCall;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::hir::{self, ItemId};

declare_forge_lint!(
    REDUNDANT_BASE_CONSTRUCTOR_CALL,
    Severity::Info,
    "redundant-base-constructor-call",
    "explicit empty base-constructor arguments are redundant"
);

impl<'hir> LateLintPass<'hir> for RedundantBaseConstructorCall {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        contract: &'hir hir::Contract<'hir>,
    ) {
        // `contract X is A(...), B(...)` clauses.
        for m in contract.bases_args {
            try_emit(ctx, hir, m);
        }
    }

    fn check_function(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        // `constructor() A(...) {}` modifier-style base calls.
        if !matches!(func.kind, hir::FunctionKind::Constructor) {
            return;
        }
        for m in func.modifiers {
            // Base-constructor invocations resolve to a contract; real modifiers resolve to
            // functions.
            if matches!(m.id, ItemId::Contract(_)) {
                try_emit(ctx, hir, m);
            }
        }
    }
}

fn try_emit<'hir>(ctx: &LintContext, hir: &'hir hir::Hir<'hir>, m: &'hir hir::Modifier<'hir>) {
    let ItemId::Contract(base_id) = m.id else { return };

    // `is A` (no parens written) — nothing to flag.
    if m.args.is_dummy() {
        return;
    }
    // `A(args...)` with real arguments — not redundant.
    if !m.args.is_empty() {
        return;
    }

    // Empty `()`. Redundant only when the base ctor takes no parameters
    // (or the base declares no constructor at all).
    let base = hir.contract(base_id);
    let redundant = match base.ctor {
        None => true,
        Some(c) => hir.function(c).parameters.is_empty(),
    };
    if redundant {
        ctx.emit(&REDUNDANT_BASE_CONSTRUCTOR_CALL, m.args.span);
    }
}
