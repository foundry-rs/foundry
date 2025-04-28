use std::ops::ControlFlow;
use solar_ast::{BinOp, BinOpKind, Expr, ExprKind};

use super::{EarlyLintPass, IncorrectShift, LintContext, INCORRECT_SHIFT};

impl<'ast> EarlyLintPass<'ast> for IncorrectShift {
    fn check_expr(&mut self, ctx: &LintContext<'_>, expr: &'ast Expr<'ast>) -> ControlFlow<()> {
        if let ExprKind::Binary(left_expr, BinOp { kind: BinOpKind::Shl | BinOpKind::Shr, .. }, right_expr) = &expr.kind {
            if contains_incorrect_shift(left_expr, right_expr) {
                ctx.emit(&INCORRECT_SHIFT, expr.span);
            }
        }
        ControlFlow::Continue(())
    }
}

// TODO: come up with a better heuristic. Treat initial impl as a PoC.
// Checks if the left operand is a literal and the right operand is not, indicating a potential reversed shift operation.
fn contains_incorrect_shift<'ast>(left_expr: &'ast Expr<'ast>, right_expr: &'ast Expr<'ast>) -> bool {
    let is_left_literal = matches!(left_expr.kind, ExprKind::Lit(..));
    let is_right_not_literal = !matches!(right_expr.kind, ExprKind::Lit(..));

    is_left_literal && is_right_not_literal
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use crate::{linter::Lint, sol::SolidityLinter};

    #[test]
    fn test_incorrect_shift() -> eyre::Result<()> {
        let linter = SolidityLinter::new()
            .with_lints(Some(vec![INCORRECT_SHIFT]))
            .with_buffer_emitter(true);

        let emitted =
            linter.lint_file(Path::new("testdata/IncorrectShift.sol")).unwrap().to_string();
        let warnings =
            emitted.matches(&format!("warning: {}", INCORRECT_SHIFT.id())).count();
        let notes = emitted.matches(&format!("note: {}", INCORRECT_SHIFT.id())).count();

        assert_eq!(warnings, 5, "Expected 5 warnings");
        assert_eq!(notes, 0, "Expected 0 notes");

        Ok(())
    }
}
