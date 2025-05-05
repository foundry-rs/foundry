use solar_ast::{Expr, ExprKind};
use solar_interface::kw::Keccak256;

use super::AsmKeccak256;
use crate::{
    declare_forge_lint,
    linter::EarlyLintPass,
    sol::{Severity, SolLint},
};

declare_forge_lint!(
    ASM_KECCACK256,
    Severity::Gas,
    "asm-keccack256",
    "hash using inline assembly to save gas",
    ""
);

impl<'ast> EarlyLintPass<'ast> for AsmKeccak256 {
    fn check_expr(&mut self, ctx: &crate::linter::LintContext<'_>, expr: &'ast Expr<'ast>) {
        if let ExprKind::Call(expr, _) = &expr.kind {
            if let ExprKind::Ident(ident) = &expr.kind {
                if ident.name == Keccak256 {
                    ctx.emit(&ASM_KECCACK256, expr.span);
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use crate::{linter::Lint, sol::SolidityLinter};

    #[test]
    fn test_keccak256() -> eyre::Result<()> {
        let linter = SolidityLinter::new().with_lints(Some(vec![ASM_KECCACK256]));

        let emitted = linter.lint_test(Path::new("testdata/Keccak256.sol")).unwrap().to_string();
        let warnings = emitted.matches(&format!("warning[{}]", ASM_KECCACK256.id())).count();
        let notes = emitted.matches(&format!("note[{}]", ASM_KECCACK256.id())).count();

        assert_eq!(warnings, 0, "Expected 0 warnings");
        assert_eq!(notes, 2, "Expected 2 notes");

        Ok(())
    }
}
