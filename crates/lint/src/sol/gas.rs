use solar_ast::{Expr, ExprKind};
use std::ops::ControlFlow;

use super::{AsmKeccak256, ASM_KECCACK256};
use crate::linter::EarlyLintPass;

impl<'ast> EarlyLintPass<'ast> for AsmKeccak256 {
    fn check_expr(
        &mut self,
        ctx: &crate::linter::LintContext<'_>,
        expr: &'ast Expr<'ast>,
    ) -> ControlFlow<()> {
        if let ExprKind::Call(expr, _) = &expr.kind {
            if let ExprKind::Ident(ident) = &expr.kind {
                if ident.name.as_str() == "keccak256" {
                    ctx.emit(&ASM_KECCACK256, expr.span);
                }
            }
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use crate::{linter::Lint, sol::SolidityLinter};

    #[test]
    fn test_keccak256() -> eyre::Result<()> {
        let linter =
            SolidityLinter::new().with_lints(Some(vec![ASM_KECCACK256])).with_buffer_emitter(true);

        let emitted = linter.lint_file(Path::new("testdata/Keccak256.sol")).unwrap().to_string();
        let warnings = emitted.matches(&format!("warning: {}", ASM_KECCACK256.id())).count();
        let notes = emitted.matches(&format!("note: {}", ASM_KECCACK256.id())).count();

        assert_eq!(warnings, 0, "Expected 0 warnings");
        assert_eq!(notes, 2, "Expected 2 notes");

        Ok(())
    }
}
