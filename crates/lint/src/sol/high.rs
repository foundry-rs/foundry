use solar_ast::{ast::Expr, visit::Visit};

use super::{ArbitraryTransferFrom, IncorrectShift};

impl<'ast> Visit<'ast> for IncorrectShift {
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) {
        todo!()
    }
}

impl<'ast> Visit<'ast> for ArbitraryTransferFrom {
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) {
        todo!()
    }
}
