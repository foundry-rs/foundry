use solar_ast::{
    ast::{Expr, ExprKind},
    visit::Visit,
};

use super::{
    AsmKeccak256, AvoidUsingThis, PackStorageVariables, PackStructs, UseConstantVariable,
    UseExternalVisibility, UseImmutableVariable,
};

impl<'ast> Visit<'ast> for AsmKeccak256 {
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) {
        if let ExprKind::Call(expr, _) = &expr.kind {
            if let ExprKind::Ident(ident) = &expr.kind {
                if ident.name.as_str() == "keccak256" {
                    self.results.push(expr.span);
                }
            }
        }
        self.walk_expr(expr);
    }
}

impl<'ast> Visit<'ast> for PackStorageVariables {
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) {
        // TODO:
        self.walk_expr(expr);
    }
}

impl<'ast> Visit<'ast> for PackStructs {
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) {
        // TODO:
        self.walk_expr(expr);
    }
}

impl<'ast> Visit<'ast> for UseConstantVariable {
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) {
        // TODO:
        self.walk_expr(expr);
    }
}

impl<'ast> Visit<'ast> for UseImmutableVariable {
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) {
        // TODO:
        self.walk_expr(expr);
    }
}

impl<'ast> Visit<'ast> for UseExternalVisibility {
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) {
        // TODO:
        self.walk_expr(expr);
    }
}

impl<'ast> Visit<'ast> for AvoidUsingThis {
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) {
        // TODO:
        self.walk_expr(expr);
    }
}

// TODO: avoid using `this` to read public variables

#[cfg(test)]
mod test {
    use solar_ast::{ast, visit::Visit};
    use solar_interface::{ColorChoice, Session};
    use std::path::Path;

    use super::AsmKeccak256;

    #[test]
    fn test_keccak256() -> eyre::Result<()> {
        let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();

        let _ = sess.enter(|| -> solar_interface::Result<()> {
            let arena = ast::Arena::new();

            let mut parser =
                solar_parse::Parser::from_file(&sess, &arena, Path::new("testdata/Keccak256.sol"))?;

            // Parse the file.
            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let mut pattern = AsmKeccak256::default();
            pattern.visit_source_unit(&ast);

            assert_eq!(pattern.results.len(), 2);

            Ok(())
        });

        Ok(())
    }
}
