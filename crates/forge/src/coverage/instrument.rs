use solar::{
    ast::{self, visit::Visit},
    interface::Session,
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

pub struct Instrumenter<'ast> {
    pub points: Vec<CoveragePoint>,
    pub updates: Vec<(ast::Span, String)>,
    pub branch_counter: usize,
    pub item_counter: usize,
    pub skip_instrumentation: bool,
    _marker: std::marker::PhantomData<&'ast ()>,
}

impl<'ast> Instrumenter<'ast> {
    pub fn new() -> Self {
        Self {
            points: Vec::new(),
            updates: Vec::new(),
            branch_counter: 0,
            item_counter: 0,
            skip_instrumentation: false,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn instrument(&mut self, sess: &Session, content: &mut String) {
        if self.updates.is_empty() {
            return;
        }

        let sf = sess.source_map().lookup_source_file(self.updates[0].0.lo());
        let base = sf.start_pos.0;

        self.updates.sort_by_key(|(span, _)| span.lo());
        let mut shift = 0_i64;
        for (span, new) in std::mem::take(&mut self.updates) {
            let lo = span.lo() - base;
            let hi = span.hi() - base;
            let start = ((lo.0 as i64) - shift) as usize;
            let end = ((hi.0 as i64) - shift) as usize;

            content.replace_range(start..end, &new);
            shift += (end as i64 - start as i64) - (new.len() as i64);
        }
    }

    fn next_item_id(&mut self) -> usize {
        let id = self.item_counter;
        self.item_counter += 1;
        id
    }

    fn inject_hit(&mut self, span: ast::Span, item_id: usize) {
        let hit_call = format!("vm.coverageHit({});", item_id);
        let injection_span = span.with_hi(span.lo());
        self.updates.push((injection_span, hit_call));
    }

    fn inject_into_block_or_wrap(&mut self, stmt: &'ast ast::Stmt<'ast>, item_id: usize) {
        self.wrap_with_hit(stmt.span, item_id);
    }

    fn wrap_with_hit(&mut self, span: ast::Span, item_id: usize) {
        self.updates.push((span.with_hi(span.lo()), format!("{{ vm.coverageHit({}); ", item_id)));
        self.updates.push((span.with_lo(span.hi()), " }".to_string()));
    }
}

impl<'ast> Visit<'ast> for Instrumenter<'ast> {
    type BreakValue = ();

    fn visit_item_function(&mut self, func: &'ast ast::ItemFunction<'ast>) -> ControlFlow<Self::BreakValue> {
        // Strip pure/view modifiers to allow vm.coverageHit (which is state-changing)
        if let Some(sm) = &func.header.state_mutability {
            let s = sm.to_string();
            if s == "pure" || s == "view" {
                self.updates.push((sm.span, "".to_string()));
            }
        }

        if let Some(block) = &func.body {
            let item_id = self.next_item_id();
            // Inject hit at the start of function/modifier body
            let start_span = block.first().map(|s| s.span).unwrap_or(func.header.span);
            self.inject_hit(start_span, item_id);
        }
        self.walk_item_function(func)
    }

    fn visit_stmt(&mut self, stmt: &'ast ast::Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        if self.skip_instrumentation {
            return self.walk_stmt(stmt);
        }

        match &stmt.kind {
            ast::StmtKind::If(cond, then, els_opt) => {
                let branch_id = self.branch_counter;
                self.branch_counter += 1;

                // Hit for the 'if' check itself
                let item_id = self.next_item_id();
                self.inject_hit(stmt.span.with_hi(cond.span.hi()), item_id);

                // True path
                let true_item_id = self.next_item_id();
                self.points.push(CoveragePoint {
                    kind: CoveragePointKind::Branch { branch_id, path_id: 0 },
                    span: then.span,
                });
                self.inject_into_block_or_wrap(then, true_item_id);

                // False path
                let false_item_id = self.next_item_id();
                self.points.push(CoveragePoint {
                    kind: CoveragePointKind::Branch { branch_id, path_id: 1 },
                    span: els_opt.as_ref().map(|e| e.span).unwrap_or(stmt.span),
                });
                if let Some(els) = els_opt {
                    self.inject_into_block_or_wrap(els, false_item_id);
                } else {
                    // Implicit false branch. Add an 'else' block.
                    self.updates.push((stmt.span.with_lo(stmt.span.hi()), format!(" else {{ vm.coverageHit({}); }}", false_item_id)));
                }
            }
            ast::StmtKind::For { init, cond, next, body } => {
                let item_id = self.next_item_id();
                self.inject_hit(stmt.span.with_hi(body.span.lo()), item_id);
                
                let body_item_id = self.next_item_id();
                self.inject_into_block_or_wrap(body, body_item_id);

                // Visit header components but skip their instrumentation
                self.skip_instrumentation = true;
                if let Some(init) = init {
                    let _ = self.visit_stmt(init);
                }
                if let Some(cond) = cond {
                    let _ = self.visit_expr(cond);
                }
                if let Some(next) = next {
                    let _ = self.visit_expr(next);
                }
                self.skip_instrumentation = false;

                let _ = self.visit_stmt(body);
                return ControlFlow::Continue(());
            }
            ast::StmtKind::While(cond, body) => {
                let item_id = self.next_item_id();
                self.inject_hit(stmt.span.with_hi(cond.span.hi()), item_id);
                
                let body_item_id = self.next_item_id();
                self.inject_into_block_or_wrap(body, body_item_id);

                self.skip_instrumentation = true;
                let _ = self.visit_expr(cond);
                self.skip_instrumentation = false;

                let _ = self.visit_stmt(body);
                return ControlFlow::Continue(());
            }
            ast::StmtKind::DoWhile(body, cond) => {
                let item_id = self.next_item_id();
                self.inject_hit(stmt.span.with_hi(body.span.lo()), item_id);
                
                let body_item_id = self.next_item_id();
                self.inject_into_block_or_wrap(body, body_item_id);
                
                let cond_item_id = self.next_item_id();
                self.inject_hit(cond.span, cond_item_id);

                let _ = self.visit_stmt(body);
                self.skip_instrumentation = true;
                let _ = self.visit_expr(cond);
                self.skip_instrumentation = false;
                return ControlFlow::Continue(());
            }
            ast::StmtKind::Block(_) | ast::StmtKind::UncheckedBlock(_) => {
                // Walk into blocks
            }
            _ => {
                let item_id = self.next_item_id();
                self.points.push(CoveragePoint {
                    kind: CoveragePointKind::Statement,
                    span: stmt.span,
                });
                self.inject_hit(stmt.span, item_id);
            }
        }

        self.walk_stmt(stmt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solar::interface::source_map::FileName;
    use solar::parse::Parser;

    #[test]
    fn test_instrumentation() {
        let src = "contract C {
    modifier m() {
        uint a = 1;
        _;
    }
    function foo(uint x) public m {
        uint y = x + 1;
        if (y > 10) {
            y = 0;
        }
        while (y < 5) {
            y++;
        }
        for (uint i = 0; i < 10; i++) {
            y += i;
        }
    }
    function add(uint a, uint b) public pure returns (uint) {
        return a + b;
    }
}";
        let mut content = src.to_string();
        let sess = Session::builder().with_buffer_emitter(Default::default()).build();
        let _: solar::interface::Result<()> = sess.enter_sequential(|| {
            let arena = ast::Arena::new();
            let mut parser = Parser::from_source_code(&sess, &arena, FileName::Custom("test.sol".to_string()), src.to_string()).unwrap();
            let ast = match parser.parse_file() {
                Ok(ast) => ast,
                Err(diag) => {
                    panic!("Parse error: {:?}", diag);
                }
            };

            let mut instrumenter = Instrumenter::new();
            let _ = instrumenter.visit_source_unit(&ast);
            instrumenter.instrument(&sess, &mut content);

            println!("Instrumented code:\n{}", content);
            
            // Basic assertions
            assert!(content.contains("vm.coverageHit"));
            assert!(!content.contains("pure")); // pure should be removed
            
            solar::interface::Result::Ok(())
        });
    }
}