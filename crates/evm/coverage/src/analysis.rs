use super::{CoverageItem, CoverageItemKind, SourceLocation};
use alloy_primitives::map::HashMap;
use foundry_common::TestFunctionExt;
use foundry_compilers::ProjectCompileOutput;
use rayon::prelude::*;
use solar::{
    ast::{self, ExprKind, ItemKind, StmtKind, visit::Visit, yul},
    data_structures::Never,
    interface::{Session, Span},
};
use std::{ops::ControlFlow, path::PathBuf, sync::Arc};

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
    /// Stores the last line we put in the items collection to ensure we don't push duplicate lines
    last_line: u32,

    /// Coverage items
    items: Vec<CoverageItem>,
}

impl<'a> ContractVisitor<'a> {
    fn new(source_id: u32, sess: &'a Session, contract_name: Arc<str>) -> Self {
        Self { source_id, sess, contract_name, branch_id: 0, last_line: 0, items: Vec::new() }
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
        if self.items.is_empty() {
            return;
        }

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

    /// Creates a coverage item for a given kind and source location. Pushes item to the internal
    /// collection (plus additional coverage line if item is a statement).
    fn push_item_kind(&mut self, kind: CoverageItemKind, span: Span) {
        let item = CoverageItem { kind, loc: self.source_location_for(span), hits: 0 };

        // Push a line item if we haven't already.
        debug_assert!(!matches!(item.kind, CoverageItemKind::Line));
        if self.last_line < item.loc.lines.start {
            self.items.push(CoverageItem {
                kind: CoverageItemKind::Line,
                loc: item.loc.clone(),
                hits: 0,
            });
            self.last_line = item.loc.lines.start;
        }

        self.items.push(item);
    }

    fn source_location_for(&self, span: Span) -> SourceLocation {
        let bytes_usize = self.sess.source_map().span_to_source(span).unwrap().1;
        let bytes = bytes_usize.start as u32..bytes_usize.end as u32;

        let lines = self.sess.source_map().span_to_lines(span).unwrap().lines;
        assert!(!lines.is_empty());
        let first = lines.first().unwrap();
        let last = lines.last().unwrap();
        let lines = first.line_index as u32 + 1..last.line_index as u32 + 2;

        SourceLocation {
            source_id: self.source_id as usize,
            contract_name: self.contract_name.clone(),
            bytes,
            lines,
        }
    }
}

impl<'a, 'ast> Visit<'ast> for ContractVisitor<'a> {
    type BreakValue = Never;

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
            StmtKind::Assembly(_) | StmtKind::Block(_) | StmtKind::UncheckedBlock(_) => {
                self.walk_stmt(stmt)?
            }
            StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Emit(..)
            | StmtKind::Revert(..)
            | StmtKind::Return(_)
            | StmtKind::DeclSingle(_)
            | StmtKind::DeclMulti(..) => {
                self.push_item_kind(CoverageItemKind::Statement, stmt.span);
            }
            // Skip placeholder statements as they are never referenced in source maps.
            StmtKind::Placeholder => {}
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.visit_expr(cond)?;
                let branch_id = self.branch_id;
                self.branch_id += 1;
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
                    if let Some(else_stmt) = else_stmt {
                        // The relevant source range for the false branch is the `else`
                        // statement itself and the false body of the else statement.
                        self.push_item_kind(
                            CoverageItemKind::Branch {
                                branch_id,
                                path_id: 1,
                                is_first_opcode: false,
                            },
                            stmt.span.to(else_stmt.span),
                        );
                    }
                }

                self.visit_stmt(then_stmt)?;
                if let Some(else_stmt) = else_stmt {
                    self.visit_stmt(else_stmt)?;
                }
            }
            StmtKind::Try(ast::StmtTry { expr, clauses }) => {
                self.visit_expr(expr)?;
                let branch_id = self.branch_id;
                self.branch_id += 1;

                let mut path_id = 0;
                for catch in clauses.iter() {
                    let ast::TryCatchClause { span, name: _, args, block } = catch;
                    let span = if path_id == 0 { stmt.span.to(*span) } else { *span };
                    self.visit_parameter_list(args)?;
                    if path_id == 0 || has_statements(Some(block)) {
                        self.push_item_kind(
                            CoverageItemKind::Branch { branch_id, path_id, is_first_opcode: true },
                            span,
                        );
                        path_id += 1;
                        self.visit_block(block)?;
                    } else if !args.is_empty() {
                        // Add coverage for clause with parameters and empty statements.
                        // (`catch (bytes memory reason) {}`).
                        // Catch all clause without statements is ignored (`catch {}`).
                        self.push_item_kind(CoverageItemKind::Statement, span);
                        self.visit_block(block)?;
                    }
                }
            }
            StmtKind::DoWhile(..)
            | StmtKind::Expr(_)
            | StmtKind::For { .. }
            | StmtKind::While(..) => self.walk_stmt(stmt)?,
        }
        // Intentionally do not walk all statements.
        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &'ast ast::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            ExprKind::Assign(..)
            | ExprKind::Unary(..)
            | ExprKind::Binary(..)
            | ExprKind::Ternary(..) => {
                self.push_item_kind(CoverageItemKind::Statement, expr.span);
                if matches!(expr.kind, ExprKind::Binary(..)) {
                    return self.walk_expr(expr);
                }
            }
            ExprKind::Call(callee, _args) => {
                self.push_item_kind(CoverageItemKind::Statement, expr.span);

                if let ExprKind::Ident(ident) = &callee.kind {
                    // Might be a require call, add branch coverage.
                    // Asserts should not be considered branches: <https://github.com/foundry-rs/foundry/issues/9460>.
                    if ident.as_str() == "require" {
                        let branch_id = self.branch_id;
                        self.branch_id += 1;
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
                self.push_item_kind(CoverageItemKind::Statement, stmt.span);
            }
            StmtKind::If(cond, block) => {
                self.visit_yul_expr(cond)?;
                let branch_id = self.branch_id;
                self.branch_id += 1;
                self.push_item_kind(
                    CoverageItemKind::Branch { branch_id, path_id: 0, is_first_opcode: false },
                    stmt.span,
                );
                self.visit_yul_block(block)?;
            }
            StmtKind::For { init, cond, step, body } => {
                self.visit_yul_block(init)?;
                self.visit_yul_expr(cond)?;
                self.visit_yul_block(step)?;
                self.push_item_kind(CoverageItemKind::Statement, body.span);
                self.visit_yul_block(body)?;
            }
            StmtKind::Switch(switch) => {
                for case in switch.branches.iter() {
                    self.push_item_kind(CoverageItemKind::Statement, case.body.span);
                    self.visit_yul_block(&case.body)?;
                }
                if let Some(default) = &switch.default_case {
                    self.push_item_kind(CoverageItemKind::Statement, default.span);
                    self.visit_yul_block(default)?;
                }
            }
            StmtKind::FunctionDef(func) => {
                let name = func.name.as_str();
                self.push_item_kind(CoverageItemKind::Function { name: name.into() }, stmt.span);
                self.visit_yul_block(&func.body)?;
            }
            StmtKind::Block(_) | StmtKind::Expr(_) => self.walk_yul_stmt(stmt)?,
        }
        // Intentionally do not walk all statements.
        ControlFlow::Continue(())
    }

    fn visit_yul_expr(&mut self, expr: &'ast yul::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        use yul::ExprKind;
        match &expr.kind {
            ExprKind::Path(_) | ExprKind::Lit(_) => {}
            ExprKind::Call(_) => {
                self.push_item_kind(CoverageItemKind::Statement, expr.span);
            }
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
                        visitor.disambiguate_functions();
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
