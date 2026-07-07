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

const ASSEMBLY_KW_LEN: u32 = 8;
const NATSPEC_MEMORY_SAFE_MARKER: &str = "@solidity memory-safe-assembly";

impl<'ast> EarlyLintPass<'ast> for InlineAssembly {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        let StmtKind::Assembly(asm) = &stmt.kind else { return };

        let kw_span = assembly_keyword_span(stmt.span);

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

/// Narrows a span to the leading `assembly` keyword to keep diagnostics readable.
fn assembly_keyword_span(span: Span) -> Span {
    span.with_hi(span.lo() + BytePos(ASSEMBLY_KW_LEN))
}

/// Returns `true` when the lines immediately preceding `stmt_lo` form a `///` NatSpec block
/// containing `@solidity memory-safe-assembly`.
fn has_memory_safe_natspec(ctx: &LintContext, stmt_lo: BytePos) -> bool {
    let Some(source_file) = ctx.source_file() else { return false };
    let src = source_file.src.as_str();
    let start_pos = source_file.start_pos.to_u32();
    let lo_abs = stmt_lo.to_u32();
    if lo_abs < start_pos {
        return false;
    }
    let offset = (lo_abs - start_pos) as usize;
    if offset > src.len() {
        return false;
    }

    for line in src[..offset].lines().rev() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("///") else { return false };
        if rest.trim_start().starts_with(NATSPEC_MEMORY_SAFE_MARKER) {
            return true;
        }
    }
    false
}
