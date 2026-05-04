use super::InlineAssembly;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{Stmt, StmtKind},
    interface::{BytePos, Span},
};

declare_forge_lint!(
    INLINE_ASSEMBLY,
    Severity::Info,
    "inline-assembly",
    "usage of inline assembly; assembly bypasses Solidity safety features and should be reviewed"
);

// `assembly` keyword length, used to narrow the diagnostic span so multi-line blocks don't
// flood the output.
const ASSEMBLY_KW_LEN: u32 = 8;

impl<'ast> EarlyLintPass<'ast> for InlineAssembly {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        let StmtKind::Assembly(asm) = &stmt.kind else { return };

        // Underline only the `assembly` keyword to keep diagnostics readable for large blocks.
        let kw_span = assembly_keyword_span(stmt.span);

        // `assembly ("memory-safe") { ... }` is a self-attestation by the developer that the
        // block does not violate Solidity's memory model. We still flag it (the lint's job is
        // to surface every block for review), but tailor the message so reviewers know they
        // can focus on business logic rather than memory layout.
        let memory_safe = asm.flags.iter().any(|f| f.value.as_str() == "memory-safe");
        let msg = if memory_safe {
            "inline assembly (declared memory-safe); review business logic and side effects"
        } else {
            "inline assembly used; review for memory safety and side effects"
        };

        ctx.emit_with_msg(&INLINE_ASSEMBLY, kw_span, msg);
    }
}

fn assembly_keyword_span(span: Span) -> Span {
    span.with_hi(span.lo() + BytePos(ASSEMBLY_KW_LEN))
}
