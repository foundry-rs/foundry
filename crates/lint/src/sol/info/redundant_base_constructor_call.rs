use super::RedundantBaseConstructorCall;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use solar::{
    interface::{BytePos, Span, diagnostics::Applicability},
    sema::hir::{self, ItemId},
};

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
        // Removing only the `()` is enough: `is A` is valid Solidity.
        for m in contract.bases_args {
            try_emit(ctx, hir, m, m.args.span);
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
                // The bare base name `A` (without parens) is not valid in a constructor's
                // modifier list, so the whole `A()` must be removed. Extend the span to also
                // swallow one leading whitespace char to avoid leaving a double space.
                try_emit(ctx, hir, m, expand_to_leading_ws(ctx, m.span));
            }
        }
    }
}

fn try_emit<'hir>(
    ctx: &LintContext,
    hir: &'hir hir::Hir<'hir>,
    m: &'hir hir::Modifier<'hir>,
    fix_span: Span,
) {
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
    if !redundant {
        return;
    }

    // Only emit a machine-applicable fix if the args span really is just `()` (no comments,
    // whitespace, etc. that we'd silently drop). Otherwise fall back to a plain diagnostic.
    let safe_to_fix = ctx.span_to_snippet(m.args.span).map(|s| s.trim() == "()").unwrap_or(false);

    if safe_to_fix {
        ctx.emit_with_suggestion(
            &REDUNDANT_BASE_CONSTRUCTOR_CALL,
            m.args.span,
            Suggestion::fix(String::new(), Applicability::MachineApplicable)
                .with_span(fix_span)
                .with_desc("remove redundant base-constructor call"),
        );
    } else {
        ctx.emit(&REDUNDANT_BASE_CONSTRUCTOR_CALL, m.args.span);
    }
}

/// Extends `span` to start one byte earlier when that byte is an ASCII space or tab.
///
/// Used so that removing a modifier-list base call like `A()` from
/// `constructor() ... A() {}` doesn't leave a stray double space in the source.
fn expand_to_leading_ws(ctx: &LintContext, span: Span) -> Span {
    if span.is_dummy() || span.lo() == BytePos(0) {
        return span;
    }
    let prev = Span::new(span.lo() - BytePos(1), span.lo());
    match ctx.span_to_snippet(prev).as_deref() {
        Some(" " | "\t") => span.with_lo(span.lo() - BytePos(1)),
        _ => span,
    }
}
