use super::{CoverageItem, CoverageItemKind, SourceLocation};
use alloy_primitives::map::HashMap;
use foundry_common::TestFunctionExt;
use foundry_compilers::ProjectCompileOutput;
use rayon::prelude::*;
use solar::{
    ast::{self, ExprKind, ItemKind, StmtKind, visit::Visit, yul},
    data_structures::{Never, map::FxHashSet},
    interface::{BytePos, Session, Span},
};
use std::{
    ops::{ControlFlow, Range},
    path::PathBuf,
    sync::Arc,
};

/// A visitor that walks the AST of a single contract and finds coverage items.
#[derive(Clone)]
struct ContractVisitor<'a> {
    /// The source ID of the contract.
    source_id: u32,
    /// The solar session for span resolution.
    sess: &'a Session,

    /// The name of the contract being walked.
    contract_name: Arc<str>,

    /// The current branch ID
    branch_id: u32,
    /// Set of all lines to ensure we don't push duplicate lines
    all_lines: FxHashSet<u32>,

    /// Coverage items
    items: Vec<CoverageItem>,
}

impl<'a> ContractVisitor<'a> {
    fn new(source_id: u32, sess: &'a Session, contract_name: Arc<str>) -> Self {
        Self {
            source_id,
            sess,
            contract_name,
            branch_id: 0,
            all_lines: Default::default(),
            items: Vec::new(),
        }
    }

    /// Filter out all items if the contract has any test functions.
    fn clear_if_test(&mut self) {
        let has_tests = self.items.iter().any(|item| {
            if let CoverageItemKind::Function { name } = &item.kind {
                name.is_any_test()
            } else {
                false
            }
        });
        if has_tests {
            self.items = Vec::new();
        }
    }

    /// Disambiguate functions with the same name in the same contract.
    fn disambiguate_functions(&mut self) {
        let mut dups = HashMap::<_, Vec<usize>>::default();
        for (i, item) in self.items.iter().enumerate() {
            if let CoverageItemKind::Function { name } = &item.kind {
                dups.entry(name.clone()).or_default().push(i);
            }
        }
        for dups in dups.values() {
            if dups.len() > 1 {
                for (i, &dup) in dups.iter().enumerate() {
                    let item = &mut self.items[dup];
                    if let CoverageItemKind::Function { name } = &item.kind {
                        item.kind =
                            CoverageItemKind::Function { name: format!("{name}.{i}").into() };
                    }
                }
            }
        }
    }

    fn sort(&mut self) {
        self.items.sort();
    }

    fn push_lines(&mut self) {
        let mut lines = Vec::new();
        for &line in &self.all_lines {
            let reference_item = self
                .items
                .iter()
                .find(|item| item.loc.lines.start == line)
                .unwrap_or_else(|| panic!("no associated item for line {line}"));
            lines.push(CoverageItem {
                kind: CoverageItemKind::Line,
                loc: reference_item.loc.clone(),
                hits: 0,
            });
        }
        self.items.extend(lines);
    }

    fn push_stmt(&mut self, span: Span) {
        self.push_item_kind(CoverageItemKind::Statement, span);
    }

    /// Creates a coverage item for a given kind and source location. Pushes item to the internal
    /// collection (plus additional coverage line if item is a statement).
    fn push_item_kind(&mut self, kind: CoverageItemKind, span: Span) {
        let item = CoverageItem { kind, loc: self.source_location_for(span), hits: 0 };

        // Register a line item if we haven't already.
        debug_assert!(!matches!(item.kind, CoverageItemKind::Line));
        self.all_lines.insert(item.loc.lines.start);

        self.items.push(item);
    }

    fn source_location_for(&self, mut span: Span) -> SourceLocation {
        // Statements' ranges in the solc source map do not include the semicolon.
        if let Ok(snippet) = self.sess.source_map().span_to_snippet(span)
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
        let bytes_usize = self.sess.source_map().span_to_source(span).unwrap().data;
        bytes_usize.start as u32..bytes_usize.end as u32
    }

    fn line_range(&self, span: Span) -> Range<u32> {
        let lines = self.sess.source_map().span_to_lines(span).unwrap().data;
        assert!(!lines.is_empty());
        let first = lines.first().unwrap();
        let last = lines.last().unwrap();
        first.line_index as u32 + 1..last.line_index as u32 + 2
    }

    fn next_branch_id(&mut self) -> u32 {
        let id = self.branch_id;
        self.branch_id = id + 1;
        id
    }
}

impl<'a, 'ast> Visit<'ast> for ContractVisitor<'a> {
    type BreakValue = Never;

    #[expect(clippy::single_match)]
    fn visit_item(&mut self, item: &'ast ast::Item<'ast>) -> ControlFlow<Self::BreakValue> {
        match &item.kind {
            ItemKind::Function(func) => {
                // TODO: We currently can only detect empty bodies in normal functions, not any of
                // the other kinds: https://github.com/foundry-rs/foundry/issues/9458
                if func.kind != ast::FunctionKind::Function && !has_statements(func.body.as_ref()) {
                    return ControlFlow::Continue(());
                }

                let name = func.header.name.as_ref().map(|n| n.as_str()).unwrap_or_else(|| {
                    match func.kind {
                        ast::FunctionKind::Constructor => "constructor",
                        ast::FunctionKind::Receive => "receive",
                        ast::FunctionKind::Fallback => "fallback",
                        ast::FunctionKind::Function | ast::FunctionKind::Modifier => unreachable!(),
                    }
                });

                self.push_item_kind(CoverageItemKind::Function { name: name.into() }, item.span);
            }
            _ => {}
        }
        self.walk_item(item)
    }

    fn visit_stmt(&mut self, stmt: &'ast ast::Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        match &stmt.kind {
            StmtKind::Break | StmtKind::Continue | StmtKind::Emit(..) | StmtKind::Revert(..) => {
                self.push_stmt(stmt.span);
                // TODO(dani): these probably shouldn't be excluded.
                return ControlFlow::Continue(());
            }
            StmtKind::Return(_) | StmtKind::DeclSingle(_) | StmtKind::DeclMulti(..) => {
                self.push_stmt(stmt.span);
            }

            StmtKind::If(_cond, then_stmt, else_stmt) => {
                let branch_id = self.next_branch_id();

                // Add branch coverage items only if one of true/branch bodies contains statements.
                if stmt_has_statements(then_stmt)
                    || else_stmt.as_ref().is_some_and(|s| stmt_has_statements(s))
                {
                    // The branch instruction is mapped to the first opcode within the true
                    // body source range.
                    self.push_item_kind(
                        CoverageItemKind::Branch { branch_id, path_id: 0, is_first_opcode: true },
                        then_stmt.span,
                    );
                    if else_stmt.is_some() {
                        // We use `stmt.span`, which includes `else_stmt.span`, since we need to
                        // include the condition so that this can be marked as covered.
                        // Initially implemented in https://github.com/foundry-rs/foundry/pull/3094.
                        self.push_item_kind(
                            CoverageItemKind::Branch {
                                branch_id,
                                path_id: 1,
                                is_first_opcode: false,
                            },
                            stmt.span,
                        );
                    }
                }
            }

            StmtKind::Try(ast::StmtTry { expr: _, clauses }) => {
                let branch_id = self.next_branch_id();

                let mut path_id = 0;
                for catch in clauses.iter() {
                    let ast::TryCatchClause { span, name: _, args, block } = catch;
                    let span = if path_id == 0 { stmt.span.to(*span) } else { *span };
                    if path_id == 0 || has_statements(Some(block)) {
                        self.push_item_kind(
                            CoverageItemKind::Branch { branch_id, path_id, is_first_opcode: true },
                            span,
                        );
                        path_id += 1;
                    } else if !args.is_empty() {
                        // Add coverage for clause with parameters and empty statements.
                        // (`catch (bytes memory reason) {}`).
                        // Catch all clause without statements is ignored (`catch {}`).
                        self.push_stmt(span);
                    }
                }
            }

            // Skip placeholder statements as they are never referenced in source maps.
            StmtKind::Assembly(_)
            | StmtKind::Block(_)
            | StmtKind::UncheckedBlock(_)
            | StmtKind::Placeholder
            | StmtKind::Expr(_)
            | StmtKind::While(..)
            | StmtKind::DoWhile(..)
            | StmtKind::For { .. } => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'ast ast::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            ExprKind::Assign(..)
            | ExprKind::Unary(..)
            | ExprKind::Binary(..)
            | ExprKind::Ternary(..) => {
                self.push_stmt(expr.span);
                if matches!(expr.kind, ExprKind::Binary(..)) {
                    return self.walk_expr(expr);
                }
            }
            ExprKind::Call(callee, _args) => {
                self.push_stmt(expr.span);

                if let ExprKind::Ident(ident) = &callee.kind {
                    // Might be a require call, add branch coverage.
                    // Asserts should not be considered branches: <https://github.com/foundry-rs/foundry/issues/9460>.
                    if ident.as_str() == "require" {
                        let branch_id = self.next_branch_id();
                        self.push_item_kind(
                            CoverageItemKind::Branch {
                                branch_id,
                                path_id: 0,
                                is_first_opcode: false,
                            },
                            expr.span,
                        );
                        self.push_item_kind(
                            CoverageItemKind::Branch {
                                branch_id,
                                path_id: 1,
                                is_first_opcode: false,
                            },
                            expr.span,
                        );
                    }
                }
            }
            _ => {}
        }
        // Intentionally do not walk all expressions.
        ControlFlow::Continue(())
    }

    fn visit_yul_stmt(&mut self, stmt: &'ast yul::Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        use yul::StmtKind;
        match &stmt.kind {
            StmtKind::VarDecl(..)
            | StmtKind::AssignSingle(..)
            | StmtKind::AssignMulti(..)
            | StmtKind::Leave
            | StmtKind::Break
            | StmtKind::Continue => {
                self.push_stmt(stmt.span);
                // Don't walk assignments.
                return ControlFlow::Continue(());
            }
            StmtKind::If(..) => {
                let branch_id = self.next_branch_id();
                self.push_item_kind(
                    CoverageItemKind::Branch { branch_id, path_id: 0, is_first_opcode: false },
                    stmt.span,
                );
            }
            StmtKind::For { body, .. } => {
                self.push_stmt(body.span);
            }
            StmtKind::Switch(switch) => {
                for case in switch.cases.iter() {
                    self.push_stmt(case.span);
                    self.push_stmt(case.body.span);
                }
            }
            StmtKind::FunctionDef(func) => {
                let name = func.name.as_str();
                self.push_item_kind(CoverageItemKind::Function { name: name.into() }, stmt.span);
            }
            StmtKind::Block(_) | StmtKind::Expr(_) => {}
        }
        self.walk_yul_stmt(stmt)
    }

    fn visit_yul_expr(&mut self, expr: &'ast yul::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        use yul::ExprKind;
        match &expr.kind {
            ExprKind::Path(_) | ExprKind::Lit(_) => {}
            ExprKind::Call(_) => self.push_stmt(expr.span),
        }
        // Intentionally do not walk all expressions.
        ControlFlow::Continue(())
    }
}

fn has_statements(block: Option<&ast::Block<'_>>) -> bool {
    block.is_some_and(|block| !block.is_empty())
}

fn stmt_has_statements(stmt: &ast::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Assembly(a) => !a.block.is_empty(),
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => has_statements(Some(b)),
        _ => true,
    }
}

/// Coverage source analysis.
#[derive(Clone, Debug, Default)]
pub struct SourceAnalysis {
    /// All the coverage items.
    all_items: Vec<CoverageItem>,
    /// Source ID to `(offset, len)` into `all_items`.
    map: Vec<(u32, u32)>,
}

impl SourceAnalysis {
    /// Analyzes contracts in the sources held by the source analyzer.
    ///
    /// Coverage items are found by:
    /// - Walking the AST of each contract (except interfaces)
    /// - Recording the items of each contract
    ///
    /// Each coverage item contains relevant information to find opcodes corresponding to them: the
    /// source ID the item is in, the source code range of the item, and the contract name the item
    /// is in.
    ///
    /// Note: Source IDs are only unique per compilation job; that is, a code base compiled with
    /// two different solc versions will produce overlapping source IDs if the compiler version is
    /// not taken into account.
    #[instrument(name = "SourceAnalysis::new", skip_all)]
    pub fn new(data: &SourceFiles, output: &ProjectCompileOutput) -> eyre::Result<Self> {
        let mut sourced_items = output.parser().solc().compiler().enter(|compiler| {
            data.sources
                .par_iter()
                .flat_map_iter(|(&source_id, path)| {
                    let (_, source) = compiler.gcx().get_ast_source(path).unwrap();
                    let ast = source.ast.as_ref().unwrap();
                    let items = ast.items.iter().map(move |item| {
                        let ItemKind::Contract(contract) = &item.kind else { return Ok(vec![]) };

                        // Skip interfaces which have no function implementations.
                        if contract.kind.is_interface() {
                            return Ok(vec![]);
                        }

                        let name: Arc<str> = contract.name.as_str().into();
                        let _guard = debug_span!("visit_contract", %name).entered();
                        let mut visitor = ContractVisitor::new(source_id, compiler.sess(), name);
                        let _ = visitor.visit_item_contract(contract);
                        visitor.clear_if_test();
                        if !visitor.items.is_empty() {
                            visitor.disambiguate_functions();
                            visitor.sort();
                            visitor.push_lines();
                            visitor.sort();
                        }
                        Ok(visitor.items)
                    });
                    items.map(move |items| items.map(|items| (source_id, items)))
                })
                .collect::<eyre::Result<Vec<(u32, Vec<CoverageItem>)>>>()
        })?;

        // Create mapping and merge items.
        sourced_items.sort_by_key(|(id, items)| (*id, items.first().map(|i| i.loc.bytes.start)));
        let Some(&(max_idx, _)) = sourced_items.last() else { return Ok(Self::default()) };
        let len = max_idx + 1;
        let mut all_items = Vec::new();
        let mut map = vec![(u32::MAX, 0); len as usize];
        for (idx, items) in sourced_items {
            // Assumes that all `idx` items are consecutive, guaranteed by the sort above.
            let idx = idx as usize;
            if map[idx].0 == u32::MAX {
                map[idx].0 = all_items.len() as u32;
            }
            map[idx].1 += items.len() as u32;
            all_items.extend(items);
        }

        Ok(Self { all_items, map })
    }

    /// Returns all the coverage items.
    pub fn all_items(&self) -> &[CoverageItem] {
        &self.all_items
    }

    /// Returns all the mutable coverage items.
    pub fn all_items_mut(&mut self) -> &mut Vec<CoverageItem> {
        &mut self.all_items
    }

    /// Returns an iterator over the coverage items and their IDs for the given source.
    pub fn items_for_source_enumerated(
        &self,
        source_id: u32,
    ) -> impl Iterator<Item = (u32, &CoverageItem)> {
        let (base_id, items) = self.items_for_source(source_id);
        items.iter().enumerate().map(move |(idx, item)| (base_id + idx as u32, item))
    }

    /// Returns the base item ID and all the coverage items for the given source.
    pub fn items_for_source(&self, source_id: u32) -> (u32, &[CoverageItem]) {
        let (mut offset, len) = self.map.get(source_id as usize).copied().unwrap_or_default();
        if offset == u32::MAX {
            offset = 0;
        }
        (offset, &self.all_items[offset as usize..][..len as usize])
    }

    /// Returns the coverage item for the given item ID.
    #[inline]
    pub fn get(&self, item_id: u32) -> Option<&CoverageItem> {
        self.all_items.get(item_id as usize)
    }
}

/// A list of versioned sources and their ASTs.
#[derive(Default)]
pub struct SourceFiles {
    /// The versioned sources.
    pub sources: HashMap<u32, PathBuf>,
}
