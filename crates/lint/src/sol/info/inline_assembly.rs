use super::InlineAssembly;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{Stmt, StmtKind},
    interface::BytePos,
};

declare_forge_lint!(
    INLINE_ASSEMBLY,
    Severity::Info,
    "inline-assembly",
    "usage of inline assembly; assembly bypasses Solidity safety features and should be reviewed"
);

impl<'ast> EarlyLintPass<'ast> for InlineAssembly {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        let StmtKind::Assembly(asm) = &stmt.kind else { return };
        // Keep the diagnostic highlight on the leading `assembly` keyword.
        let kw_span = stmt.span.with_hi(stmt.span.lo() + BytePos("assembly".len() as u32));
        let memory_safe = asm.flags.iter().any(|f| f.value.as_str() == "memory-safe")
            || has_memory_safe_natspec(ctx, stmt.span.lo());
        let msg = if memory_safe {
            "inline assembly (declared memory-safe); review business logic and side effects"
        } else {
            "inline assembly used; review for memory safety and side effects"
        };
        ctx.emit_with_msg(&INLINE_ASSEMBLY, kw_span, msg);
    }
}

/// Returns `true` when the lines immediately preceding `stmt_lo` form a `///` NatSpec block
/// containing `@solidity memory-safe-assembly`.
fn has_memory_safe_natspec(ctx: &LintContext, stmt_lo: BytePos) -> bool {
    let Some(file) = ctx.source_file() else { return false };
    let Some(before) = stmt_lo
        .to_u32()
        .checked_sub(file.start_pos.to_u32())
        .and_then(|offset| file.src.get(..offset as usize))
    else {
        return false;
    };
    before
        .lines()
        .rev()
        .map(str::trim_start)
        .filter(|line| !line.is_empty())
        .map_while(|line| line.strip_prefix("///"))
        .any(|rest| rest.trim_start().starts_with("@solidity memory-safe-assembly"))
}
