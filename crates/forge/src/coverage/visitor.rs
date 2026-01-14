use solar::{
    ast::{self, visit::Visit},
    interface::data_structures::Never,
};
use std::ops::ControlFlow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoveragePointKind {
    Statement,
    Branch { branch_id: usize, path_id: usize },
}

#[derive(Debug, Clone)]
pub struct CoveragePoint {
    pub kind: CoveragePointKind,
    pub span: ast::Span,
}

pub struct CoverageVisitor<'ast> {
    pub points: Vec<CoveragePoint>,
    pub branch_counter: usize,
    // We need to keep track of the source code to extract spans properly if needed,
    // but ast::Span should be enough for identification.
    _marker: std::marker::PhantomData<&'ast ()>,
}

impl<'ast> CoverageVisitor<'ast> {
    pub fn new() -> Self {
        Self {
            points: Vec::new(),
            branch_counter: 0,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'ast> Visit<'ast> for CoverageVisitor<'ast> {
    type BreakValue = Never;

    fn visit_stmt(&mut self, stmt: &'ast ast::Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        // Identify the statement as a coverage point
        self.points.push(CoveragePoint {
            kind: CoveragePointKind::Statement,
            span: stmt.span,
        });

        match &stmt.kind {
            ast::StmtKind::If(_, _, _) => {
                let branch_id = self.branch_counter;
                self.branch_counter += 1;
                // True path
                self.points.push(CoveragePoint {
                    kind: CoveragePointKind::Branch { branch_id, path_id: 0 },
                    span: stmt.span, // TODO: Refine span to the 'then' block
                });
                // False path (implicit or explicit)
                self.points.push(CoveragePoint {
                    kind: CoveragePointKind::Branch { branch_id, path_id: 1 },
                    span: stmt.span, // TODO: Refine span to the 'else' block
                });
            }
            // TODO: Handle other branching statements (For, While, etc.)
            _ => {}
        }

        self.walk_stmt(stmt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solar::interface::{Session, source_map::FileName};
    use solar::parse::Parser;
    use std::path::Path;

    #[test]
    fn test_visitor_identifies_statements_and_branches() {
        let src = r#"
        contract C {
            function foo(uint x) public {
                uint y = x + 1;
                if (y > 10) {
                    y = 0;
                }
            }
        }
        "#;

        let sess = Session::builder().with_buffer_emitter(Default::default()).build();
        let _: solar::interface::Result<()> = sess.enter_sequential(|| {
            let arena = ast::Arena::new();
            let mut parser = Parser::from_source_code(&sess, &arena, FileName::Real(Path::new("test.sol").to_path_buf()), src.to_string()).unwrap();
            let ast = parser.parse_file().unwrap();

            let mut visitor = CoverageVisitor::new();
            visitor.visit_source_unit(&ast);

            // Verify points
            // 1. uint y = x + 1; (Statement)
            // 2. if (y > 10) ... (Statement)
            // 3. True branch of if
            // 4. False branch of if
            // 5. y = 0; (Statement inside if)

            let statements = visitor.points.iter().filter(|p| matches!(p.kind, CoveragePointKind::Statement)).count();
            // We expect:
            // 1. Variable definition stmt
            // 2. If stmt
            // 3. Expression stmt (y=0)
            // Note: Function body is a Block, which is also a Stmt in some ASTs, but let's see.
            // Solar StmtKind::Block is a statement.
            // The function body is a Block. The if body is a Block.
            
            // Let's print what we found to debug/verify
            println!("Found {} points", visitor.points.len());
            for (i, p) in visitor.points.iter().enumerate() {
                println!("{}: {:?}", i, p);
            }

            // We expect at least 3 statements (decl, if, assign) + 2 branches
            assert!(statements >= 3);
            
            let branches = visitor.points.iter().filter(|p| matches!(p.kind, CoveragePointKind::Branch { .. })).count();
            assert_eq!(branches, 2);
            
            solar::interface::Result::Ok(())
        });
    }
}
