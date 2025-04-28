use solar_ast::{BinOp, BinOpKind, Expr, ExprKind};
use std::ops::ControlFlow;

use super::{DivideBeforeMultiply, DIVIDE_BEFORE_MULTIPLY};
use crate::linter::{EarlyLintPass, LintContext};

impl<'ast> EarlyLintPass<'ast> for DivideBeforeMultiply {
    fn check_expr(&mut self, ctx: &LintContext<'_>, expr: &'ast Expr<'ast>) -> ControlFlow<()> {
        if let ExprKind::Binary(left_expr, BinOp { kind: BinOpKind::Mul, .. }, _) = &expr.kind {
            if contains_division(left_expr) {
                ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
            }
        }
        ControlFlow::Continue(())
    }
}

fn contains_division<'ast>(expr: &'ast Expr<'ast>) -> bool {
    match &expr.kind {
        ExprKind::Binary(_, BinOp { kind: BinOpKind::Div, .. }, _) => true,
        ExprKind::Tuple(inner_exprs) => inner_exprs.iter().any(|opt_expr| {
            if let Some(inner_expr) = opt_expr {
                contains_division(inner_expr)
            } else {
                false
            }
        }),
        _ => false,
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use crate::{linter::Lint, sol::SolidityLinter};

    #[test]
    fn test_divide_before_multiply() -> eyre::Result<()> {
        let linter = SolidityLinter::new()
            .with_lints(Some(vec![DIVIDE_BEFORE_MULTIPLY]))
            .with_buffer_emitter(true);

        let emitted =
            linter.lint_file(Path::new("testdata/DivideBeforeMultiply.sol")).unwrap().to_string();
        let warnings =
            emitted.matches(&format!("warning: {}", DIVIDE_BEFORE_MULTIPLY.id())).count();
        let notes = emitted.matches(&format!("note: {}", DIVIDE_BEFORE_MULTIPLY.id())).count();

        assert_eq!(warnings, 6, "Expected 6 warnings");
        assert_eq!(notes, 0, "Expected 0 notes");

        Ok(())
    }
}
