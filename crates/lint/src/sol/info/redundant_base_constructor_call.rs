use super::RedundantBaseConstructorCall;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::{BytePos, Span, diagnostics::Applicability},
    sema::{Gcx, hir},
};

declare_forge_lint!(
    REDUNDANT_BASE_CONSTRUCTOR_CALL,
    Severity::Info,
    "redundant-base-constructor-call"
);

impl<'gcx> LateLintPass<'gcx> for RedundantBaseConstructorCall {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract: &'gcx hir::Contract<'gcx>,
    ) {
        // `contract X is A(), B()` clauses: removing only the `()` is enough, `is A` is valid.
        for m in contract.bases_args {
            try_emit(ctx, &gcx.hir, m, m.args.span);
        }
    }

    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        func: &'gcx hir::Function<'gcx>,
    ) {
        // `constructor() A() {}` modifier-style base calls. The bare base name `A` is not valid
        // in a constructor's modifier list, so the whole `A()` must be removed, along with one
        // leading whitespace char to avoid leaving a double space.
        if func.kind == hir::FunctionKind::Constructor {
            for m in func.modifiers {
                try_emit(ctx, &gcx.hir, m, expand_to_leading_ws(ctx, m.span));
            }
        }
    }
}

fn try_emit(ctx: &LintContext, hir: &hir::Hir<'_>, m: &hir::Modifier<'_>, fix_span: Span) {
    // Base-constructor invocations resolve to a contract; real modifiers resolve to functions.
    // `is A` (no parens written) and `A(args...)` with real arguments are not redundant.
    let hir::ItemId::Contract(base_id) = m.id else { return };
    if m.args.is_dummy() || !m.args.is_empty() {
        return;
    }
    // Empty `()` is redundant only when the base constructor takes no parameters (or the base
    // declares no constructor at all).
    if hir.contract(base_id).ctor.is_some_and(|c| !hir.function(c).parameters.is_empty()) {
        return;
    }
    // Only emit a machine-applicable fix if the args span really is just `()` (no comments,
    // whitespace, etc. that would silently be dropped).
    if ctx.span_to_snippet(m.args.span).is_some_and(|s| s.trim() == "()")
        && ctx.span_to_snippet(fix_span).is_some_and(|s| !s.contains("//") && !s.contains("/*"))
    {
        ctx.span_lint(&REDUNDANT_BASE_CONSTRUCTOR_CALL, m.args.span, |diag| {
            diag.primary_message("explicit empty base-constructor arguments are redundant");
            diag.span_suggestion(
                fix_span,
                "remove redundant base-constructor call",
                String::new(),
                Applicability::MachineApplicable,
            );
        });
    } else {
        ctx.span_lint(&REDUNDANT_BASE_CONSTRUCTOR_CALL, m.args.span, |diag| {
            diag.primary_message("explicit empty base-constructor arguments are redundant");
        });
    }
}

/// Extends `span` to start one byte earlier when that byte is an ASCII space or tab.
fn expand_to_leading_ws(ctx: &LintContext, span: Span) -> Span {
    if span.is_dummy() || span.lo() == BytePos(0) {
        return span;
    }
    let lo = span.lo() - BytePos(1);
    match ctx.span_to_snippet(Span::new(lo, span.lo())).as_deref() {
        Some(" " | "\t") => span.with_lo(lo),
        _ => span,
    }
}
