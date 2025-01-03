use solar_ast::{ast::Expr, visit::Visit};

use super::IncorrectShift;

impl<'ast> Visit<'ast> for IncorrectShift {
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) {
        // TODO:
        self.walk_expr(expr);
    }
}
