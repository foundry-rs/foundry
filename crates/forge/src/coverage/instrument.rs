use foundry_evm::coverage::{CoverageItem, CoverageItemKind, SourceLocation};
use solar::{
    ast::{self, visit::Visit},
    interface::{BytePos, Session, Span},
};
use std::{ops::{ControlFlow, Range}, sync::Arc};

pub struct Instrumenter<'ast> {
    pub source_id: u32,
    pub session: &'ast Session,
    pub items: Vec<CoverageItem>,
    pub updates: Vec<(ast::Span, String)>,
    pub branch_counter: u32,
    pub item_counter: u32,
    pub skip_instrumentation: bool,
    pub contract_name: Arc<str>,
    _marker: std::marker::PhantomData<&'ast ()>,
}

impl<'ast> Instrumenter<'ast> {
    pub fn new(session: &'ast Session, source_id: u32) -> Self {
        Self {
            source_id,
            session,
            items: Vec::new(),
            updates: Vec::new(),
            branch_counter: 0,
            item_counter: 0,
            skip_instrumentation: false,
            contract_name: Arc::from(""),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn instrument(&mut self, content: &mut String) {
        if self.updates.is_empty() {
            return;
        }

        // Safety: updates MUST be sorted by lo() descending to avoid index shifting issues?
        // Actually, if we sort by lo() ascending, we need to track shift.
        // My implementation uses shift.

        let sf = self.session.source_map().lookup_source_file(self.updates[0].0.lo());
        let base = sf.start_pos.0;

        self.updates.sort_by_key(|(span, _)| span.lo());
        let mut shift = 0_i64;
        for (span, new) in std::mem::take(&mut self.updates) {
            let lo = span.lo() - base;
            let hi = span.hi() - base;
            let start = ((lo.0 as i64) - shift) as usize;
            let end = ((hi.0 as i64) - shift) as usize;

            if start > content.len() || end > content.len() {
                continue; // Safety check
            }

            content.replace_range(start..end, &new);
            shift += (end as i64 - start as i64) - (new.len() as i64);
        }
    }

    fn next_item_id(&mut self) -> u32 {
        let id = self.item_counter;
        self.item_counter += 1;
        id
    }

    fn next_branch_id(&mut self) -> u32 {
        let id = self.branch_counter;
        self.branch_counter += 1;
        id
    }

    fn inject_hit(&mut self, span: ast::Span, item_id: u32) {
        let hit_call = format!("VmCoverage_{}(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D).coverageHit({});", self.source_id, item_id);
        let injection_span = span.with_hi(span.lo());
        self.updates.push((injection_span, hit_call));
    }

    fn inject_into_block_or_wrap(&mut self, stmt: &'ast ast::Stmt<'ast>, item_id: u32) {
        self.wrap_with_hit(stmt.span, item_id);
    }

    fn wrap_with_hit(&mut self, span: ast::Span, item_id: u32) {
        self.updates.push((span.with_hi(span.lo()), format!("{{ VmCoverage_{}(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D).coverageHit({}); ", self.source_id, item_id)));
        self.updates.push((span.with_lo(span.hi()), " }".to_string()));
    }

    fn push_item(&mut self, kind: CoverageItemKind, span: Span, hits: u32) {
        let item = CoverageItem { kind, loc: self.source_location_for(span), hits };
        self.items.push(item);
    }

    fn source_location_for(&self, mut span: Span) -> SourceLocation {
        // Statements' ranges in the solc source map do not include the semicolon.
        if let Ok(snippet) = self.session.source_map().span_to_snippet(span)
            && let Some(stripped) = snippet.strip_suffix(';')
        {
            let stripped = stripped.trim_end();
            let skipped = snippet.len() - stripped.len();
            span = span.with_hi(span.hi() - BytePos::from_usize(skipped));
        }

        SourceLocation {
            source_id: self.source_id as usize,
            contract_name: self.contract_name.clone(),
            bytes: self.byte_range(span),
            lines: self.line_range(span),
        }
    }

    fn byte_range(&self, span: Span) -> Range<u32> {
        let bytes_usize = self.session.source_map().span_to_source(span).unwrap().data;
        bytes_usize.start as u32..bytes_usize.end as u32
    }

    fn line_range(&self, span: Span) -> Range<u32> {
        let lines = self.session.source_map().span_to_lines(span).unwrap().data;
        assert!(!lines.is_empty());
        let first = lines.first().unwrap();
        let last = lines.last().unwrap();
        first.line_index as u32 + 1..last.line_index as u32 + 2
    }
}

impl<'ast> Visit<'ast> for Instrumenter<'ast> {
    type BreakValue = ();

    fn visit_item_contract(&mut self, contract: &'ast ast::ItemContract<'ast>) -> ControlFlow<Self::BreakValue> {
        self.contract_name = contract.name.as_str().into();
        self.walk_item_contract(contract)
    }

    fn visit_item_function(&mut self, func: &'ast ast::ItemFunction<'ast>) -> ControlFlow<Self::BreakValue> {
        if let Some(sm) = &func.header.state_mutability {
            let s = sm.to_string();
            if s == "pure" || s == "view" {
                self.updates.push((sm.span, "".to_string()));
            }
        }

        let name = func.header.name.as_ref().map(|n| n.as_str()).unwrap_or("function");
        let item_id = self.next_item_id();
        self.push_item(CoverageItemKind::Function { name: name.into() }, func.header.span, 0);

        if let Some(block) = &func.body {
             // Inject hit at the start of function/modifier body
            let injection_span = block.first().map(|s| s.span).unwrap_or_else(|| {
                let lo = block.span.lo() + BytePos(1);
                Span::new(lo, lo)
            });
            self.inject_hit(injection_span, item_id);
        }
        self.walk_item_function(func)
    }

    fn visit_stmt(&mut self, stmt: &'ast ast::Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        if self.skip_instrumentation {
            return self.walk_stmt(stmt);
        }

        match &stmt.kind {
            ast::StmtKind::If(cond, then, els_opt) => {
                let branch_id = self.next_branch_id();
                
                // Hit for the 'if' check itself
                let item_id = self.next_item_id();
                self.push_item(CoverageItemKind::Statement, stmt.span.with_hi(cond.span.hi()), 0);
                self.inject_hit(stmt.span.with_hi(cond.span.hi()), item_id);

                // True path
                let true_item_id = self.next_item_id();
                self.push_item(
                    CoverageItemKind::Branch { branch_id, path_id: 0, is_first_opcode: true },
                    then.span,
                    0
                );
                self.inject_into_block_or_wrap(then, true_item_id);

                // False path
                let false_item_id = self.next_item_id();
                let else_span = els_opt.as_ref().map(|e| e.span).unwrap_or(stmt.span);
                self.push_item(
                    CoverageItemKind::Branch { branch_id, path_id: 1, is_first_opcode: false },
                    else_span,
                    0
                );
                
                if let Some(els) = els_opt {
                    self.inject_into_block_or_wrap(els, false_item_id);
                } else {
                    self.updates.push((stmt.span.with_lo(stmt.span.hi()), format!(" else {{ VmCoverage_{}(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D).coverageHit({}); }}", self.source_id, false_item_id)));
                }
            }
            ast::StmtKind::For { init, cond, next, body } => {
                let item_id = self.next_item_id();
                self.push_item(CoverageItemKind::Statement, stmt.span.with_hi(body.span.lo()), 0);
                self.inject_hit(stmt.span.with_hi(body.span.lo()), item_id);
                
                let body_item_id = self.next_item_id();
                self.push_item(CoverageItemKind::Statement, body.span, 0);
                self.inject_into_block_or_wrap(body, body_item_id);

                self.skip_instrumentation = true;
                if let Some(init) = init { let _ = self.visit_stmt(init); }
                if let Some(cond) = cond { let _ = self.visit_expr(cond); }
                if let Some(next) = next { let _ = self.visit_expr(next); }
                self.skip_instrumentation = false;

                let _ = self.visit_stmt(body);
                return ControlFlow::Continue(());
            }
            ast::StmtKind::While(cond, body) => {
                let item_id = self.next_item_id();
                self.push_item(CoverageItemKind::Statement, stmt.span.with_hi(cond.span.hi()), 0);
                self.inject_hit(stmt.span.with_hi(cond.span.hi()), item_id);
                
                let body_item_id = self.next_item_id();
                self.push_item(CoverageItemKind::Statement, body.span, 0);
                self.inject_into_block_or_wrap(body, body_item_id);

                self.skip_instrumentation = true;
                let _ = self.visit_expr(cond);
                self.skip_instrumentation = false;

                let _ = self.visit_stmt(body);
                return ControlFlow::Continue(());
            }
            ast::StmtKind::DoWhile(body, cond) => {
                let item_id = self.next_item_id();
                self.push_item(CoverageItemKind::Statement, stmt.span.with_hi(body.span.lo()), 0);
                self.inject_hit(stmt.span.with_hi(body.span.lo()), item_id);
                
                let body_item_id = self.next_item_id();
                self.push_item(CoverageItemKind::Statement, body.span, 0);
                self.inject_into_block_or_wrap(body, body_item_id);
                
                let cond_item_id = self.next_item_id();
                self.push_item(CoverageItemKind::Statement, cond.span, 0);
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
                self.push_item(CoverageItemKind::Statement, stmt.span, 0);
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

            let mut instrumenter = Instrumenter::new(&sess, 0);
            let _ = instrumenter.visit_source_unit(&ast);
            instrumenter.instrument(&mut content);

            println!("Instrumented code:\n{}", content);
            
            // Basic assertions
            assert!(content.contains("VmCoverage_0"));
            assert!(!content.contains("pure")); // pure should be removed

            // Check items were collected
            assert!(!instrumenter.items.is_empty());
            
            solar::interface::Result::Ok(())
        });
    }
}
