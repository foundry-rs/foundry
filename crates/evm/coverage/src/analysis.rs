use super::{CoverageItem, CoverageItemKind, SourceLocation};
use alloy_primitives::map::HashMap;
use foundry_common::TestFunctionExt;
use foundry_compilers::artifacts::{
    Source,
    ast::{self, Ast, Node, NodeType},
};
use rayon::prelude::*;
use std::sync::Arc;

/// A visitor that walks the AST of a single contract and finds coverage items.
#[derive(Clone, Debug)]
struct ContractVisitor<'a> {
    /// The source ID of the contract.
    source_id: u32,
    /// The source code that contains the AST being walked.
    source: &'a str,

    /// The name of the contract being walked.
    contract_name: &'a Arc<str>,

    /// The current branch ID
    branch_id: u32,
    /// Stores the last line we put in the items collection to ensure we don't push duplicate lines
    last_line: u32,

    /// Coverage items
    items: Vec<CoverageItem>,
}

impl<'a> ContractVisitor<'a> {
    fn new(source_id: usize, source: &'a str, contract_name: &'a Arc<str>) -> Self {
        Self {
            source_id: source_id.try_into().expect("too many sources"),
            source,
            contract_name,
            branch_id: 0,
            last_line: 0,
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

    fn visit_contract(&mut self, node: &Node) -> eyre::Result<()> {
        // Find all functions and walk their AST
        for node in &node.nodes {
            match node.node_type {
                NodeType::FunctionDefinition => {
                    self.visit_function_definition(node)?;
                }
                NodeType::ModifierDefinition => {
                    self.visit_modifier_or_yul_fn_definition(node)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn visit_function_definition(&mut self, node: &Node) -> eyre::Result<()> {
        let Some(body) = &node.body else { return Ok(()) };

        let name: Box<str> =
            node.attribute("name").ok_or_else(|| eyre::eyre!("Function has no name"))?;
        let kind: Box<str> =
            node.attribute("kind").ok_or_else(|| eyre::eyre!("Function has no kind"))?;

        // TODO: We currently can only detect empty bodies in normal functions, not any of the other
        // kinds: https://github.com/foundry-rs/foundry/issues/9458
        if &*kind != "function" && !has_statements(body) {
            return Ok(());
        }

        // `fallback`, `receive`, and `constructor` functions have an empty `name`.
        // Use the `kind` itself as the name.
        let name = if name.is_empty() { kind } else { name };

        self.push_item_kind(CoverageItemKind::Function { name }, &node.src);
        self.visit_block(body)
    }

    fn visit_modifier_or_yul_fn_definition(&mut self, node: &Node) -> eyre::Result<()> {
        let Some(body) = &node.body else { return Ok(()) };

        let name: Box<str> =
            node.attribute("name").ok_or_else(|| eyre::eyre!("Modifier has no name"))?;
        self.push_item_kind(CoverageItemKind::Function { name }, &node.src);
        self.visit_block(body)
    }

    fn visit_block(&mut self, node: &Node) -> eyre::Result<()> {
        let statements: Vec<Node> = node.attribute("statements").unwrap_or_default();

        for statement in &statements {
            self.visit_statement(statement)?;
        }

        Ok(())
    }

    fn visit_statement(&mut self, node: &Node) -> eyre::Result<()> {
        match node.node_type {
            // Blocks
            NodeType::Block | NodeType::UncheckedBlock | NodeType::YulBlock => {
                self.visit_block(node)
            }
            // Inline assembly block
            NodeType::InlineAssembly => self.visit_block(
                &node
                    .attribute("AST")
                    .ok_or_else(|| eyre::eyre!("inline assembly block with no AST attribute"))?,
            ),
            // Simple statements
            NodeType::Break
            | NodeType::Continue
            | NodeType::EmitStatement
            | NodeType::RevertStatement
            | NodeType::YulAssignment
            | NodeType::YulBreak
            | NodeType::YulContinue
            | NodeType::YulLeave
            | NodeType::YulVariableDeclaration => {
                self.push_item_kind(CoverageItemKind::Statement, &node.src);
                Ok(())
            }
            // Skip placeholder statements as they are never referenced in source maps.
            NodeType::PlaceholderStatement => Ok(()),
            // Return with eventual subcall
            NodeType::Return => {
                self.push_item_kind(CoverageItemKind::Statement, &node.src);
                if let Some(expr) = node.attribute("expression") {
                    self.visit_expression(&expr)?;
                }
                Ok(())
            }
            // Variable declaration
            NodeType::VariableDeclarationStatement => {
                self.push_item_kind(CoverageItemKind::Statement, &node.src);
                if let Some(expr) = node.attribute("initialValue") {
                    self.visit_expression(&expr)?;
                }
                Ok(())
            }
            // While loops
            NodeType::DoWhileStatement | NodeType::WhileStatement => {
                self.visit_expression(
                    &node
                        .attribute("condition")
                        .ok_or_else(|| eyre::eyre!("while statement had no condition"))?,
                )?;

                let body = node
                    .body
                    .as_deref()
                    .ok_or_else(|| eyre::eyre!("while statement had no body node"))?;
                self.visit_block_or_statement(body)
            }
            // For loops
            NodeType::ForStatement => {
                if let Some(stmt) = node.attribute("initializationExpression") {
                    self.visit_statement(&stmt)?;
                }
                if let Some(expr) = node.attribute("condition") {
                    self.visit_expression(&expr)?;
                }
                if let Some(stmt) = node.attribute("loopExpression") {
                    self.visit_statement(&stmt)?;
                }

                let body = node
                    .body
                    .as_deref()
                    .ok_or_else(|| eyre::eyre!("for statement had no body node"))?;
                self.visit_block_or_statement(body)
            }
            // Expression statement
            NodeType::ExpressionStatement | NodeType::YulExpressionStatement => self
                .visit_expression(
                    &node
                        .attribute("expression")
                        .ok_or_else(|| eyre::eyre!("expression statement had no expression"))?,
                ),
            // If statement
            NodeType::IfStatement => {
                self.visit_expression(
                    &node
                        .attribute("condition")
                        .ok_or_else(|| eyre::eyre!("if statement had no condition"))?,
                )?;

                let true_body: Node = node
                    .attribute("trueBody")
                    .ok_or_else(|| eyre::eyre!("if statement had no true body"))?;

                // We need to store the current branch ID here since visiting the body of either of
                // the if blocks may increase `self.branch_id` in the case of nested if statements.
                let branch_id = self.branch_id;

                // We increase the branch ID here such that nested branches do not use the same
                // branch ID as we do.
                self.branch_id += 1;

                match node.attribute::<Node>("falseBody") {
                    // Both if/else statements.
                    Some(false_body) => {
                        // Add branch coverage items only if one of true/branch bodies contains
                        // statements.
                        if has_statements(&true_body) || has_statements(&false_body) {
                            // The branch instruction is mapped to the first opcode within the true
                            // body source range.
                            self.push_item_kind(
                                CoverageItemKind::Branch {
                                    branch_id,
                                    path_id: 0,
                                    is_first_opcode: true,
                                },
                                &true_body.src,
                            );
                            // Add the coverage item for branch 1 (false body).
                            // The relevant source range for the false branch is the `else`
                            // statement itself and the false body of the else statement.
                            self.push_item_kind(
                                CoverageItemKind::Branch {
                                    branch_id,
                                    path_id: 1,
                                    is_first_opcode: false,
                                },
                                &ast::LowFidelitySourceLocation {
                                    start: node.src.start,
                                    length: false_body.src.length.map(|length| {
                                        false_body.src.start - true_body.src.start + length
                                    }),
                                    index: node.src.index,
                                },
                            );

                            // Process the true body.
                            self.visit_block_or_statement(&true_body)?;
                            // Process the false body.
                            self.visit_block_or_statement(&false_body)?;
                        }
                    }
                    None => {
                        // Add single branch coverage only if it contains statements.
                        if has_statements(&true_body) {
                            // Add the coverage item for branch 0 (true body).
                            self.push_item_kind(
                                CoverageItemKind::Branch {
                                    branch_id,
                                    path_id: 0,
                                    is_first_opcode: true,
                                },
                                &true_body.src,
                            );
                            // Process the true body.
                            self.visit_block_or_statement(&true_body)?;
                        }
                    }
                }

                Ok(())
            }
            NodeType::YulIf => {
                self.visit_expression(
                    &node
                        .attribute("condition")
                        .ok_or_else(|| eyre::eyre!("yul if statement had no condition"))?,
                )?;
                let body = node
                    .body
                    .as_deref()
                    .ok_or_else(|| eyre::eyre!("yul if statement had no body"))?;

                // We need to store the current branch ID here since visiting the body of either of
                // the if blocks may increase `self.branch_id` in the case of nested if statements.
                let branch_id = self.branch_id;

                // We increase the branch ID here such that nested branches do not use the same
                // branch ID as we do
                self.branch_id += 1;

                self.push_item_kind(
                    CoverageItemKind::Branch { branch_id, path_id: 0, is_first_opcode: false },
                    &node.src,
                );
                self.visit_block(body)?;

                Ok(())
            }
            // Try-catch statement. Coverage is reported as branches for catch clauses with
            // statements.
            NodeType::TryStatement => {
                self.visit_expression(
                    &node
                        .attribute("externalCall")
                        .ok_or_else(|| eyre::eyre!("try statement had no call"))?,
                )?;

                let branch_id = self.branch_id;
                self.branch_id += 1;

                let mut clauses = node
                    .attribute::<Vec<Node>>("clauses")
                    .ok_or_else(|| eyre::eyre!("try statement had no clauses"))?;

                let try_block = clauses
                    .remove(0)
                    .attribute::<Node>("block")
                    .ok_or_else(|| eyre::eyre!("try statement had no block"))?;
                // Add branch with path id 0 for try (first clause).
                self.push_item_kind(
                    CoverageItemKind::Branch { branch_id, path_id: 0, is_first_opcode: true },
                    &ast::LowFidelitySourceLocation {
                        start: node.src.start,
                        length: try_block
                            .src
                            .length
                            .map(|length| try_block.src.start + length - node.src.start),
                        index: node.src.index,
                    },
                );
                self.visit_block(&try_block)?;

                let mut path_id = 1;
                for clause in clauses {
                    if let Some(catch_block) = clause.attribute::<Node>("block") {
                        if has_statements(&catch_block) {
                            // Add catch branch if it has statements.
                            self.push_item_kind(
                                CoverageItemKind::Branch {
                                    branch_id,
                                    path_id,
                                    is_first_opcode: true,
                                },
                                &catch_block.src,
                            );
                            self.visit_block(&catch_block)?;
                            // Increment path id for next branch.
                            path_id += 1;
                        } else if clause.attribute::<Node>("parameters").is_some() {
                            // Add coverage for clause with parameters and empty statements.
                            // (`catch (bytes memory reason) {}`).
                            // Catch all clause without statements is ignored (`catch {}`).
                            self.push_item_kind(CoverageItemKind::Statement, &clause.src);
                            self.visit_statement(&clause)?;
                        }
                    }
                }

                Ok(())
            }
            NodeType::YulSwitch => {
                // Add coverage for each case statement amd their bodies.
                for case in node
                    .attribute::<Vec<Node>>("cases")
                    .ok_or_else(|| eyre::eyre!("yul switch had no case"))?
                {
                    self.push_item_kind(CoverageItemKind::Statement, &case.src);
                    self.visit_statement(&case)?;

                    if let Some(body) = case.body {
                        self.push_item_kind(CoverageItemKind::Statement, &body.src);
                        self.visit_block(&body)?
                    }
                }
                Ok(())
            }
            NodeType::YulForLoop => {
                if let Some(condition) = node.attribute("condition") {
                    self.visit_expression(&condition)?;
                }
                if let Some(pre) = node.attribute::<Node>("pre") {
                    self.visit_block(&pre)?
                }
                if let Some(post) = node.attribute::<Node>("post") {
                    self.visit_block(&post)?
                }

                if let Some(body) = &node.body {
                    self.push_item_kind(CoverageItemKind::Statement, &body.src);
                    self.visit_block(body)?
                }
                Ok(())
            }
            NodeType::YulFunctionDefinition => self.visit_modifier_or_yul_fn_definition(node),
            _ => {
                warn!("unexpected node type, expected a statement: {:?}", node.node_type);
                Ok(())
            }
        }
    }

    fn visit_expression(&mut self, node: &Node) -> eyre::Result<()> {
        match node.node_type {
            NodeType::Assignment
            | NodeType::UnaryOperation
            | NodeType::Conditional
            | NodeType::YulFunctionCall => {
                self.push_item_kind(CoverageItemKind::Statement, &node.src);
                Ok(())
            }
            NodeType::FunctionCall => {
                // Do not count other kinds of calls towards coverage (like `typeConversion`
                // and `structConstructorCall`).
                let kind: Option<String> = node.attribute("kind");
                if let Some("functionCall") = kind.as_deref() {
                    self.push_item_kind(CoverageItemKind::Statement, &node.src);

                    let expr: Option<Node> = node.attribute("expression");
                    if let Some(NodeType::Identifier) = expr.as_ref().map(|expr| &expr.node_type) {
                        // Might be a require call, add branch coverage.
                        // Asserts should not be considered branches: <https://github.com/foundry-rs/foundry/issues/9460>.
                        let name: Option<String> = expr.and_then(|expr| expr.attribute("name"));
                        if let Some("require") = name.as_deref() {
                            let branch_id = self.branch_id;
                            self.branch_id += 1;
                            self.push_item_kind(
                                CoverageItemKind::Branch {
                                    branch_id,
                                    path_id: 0,
                                    is_first_opcode: false,
                                },
                                &node.src,
                            );
                            self.push_item_kind(
                                CoverageItemKind::Branch {
                                    branch_id,
                                    path_id: 1,
                                    is_first_opcode: false,
                                },
                                &node.src,
                            );
                        }
                    }
                }

                Ok(())
            }
            NodeType::BinaryOperation => {
                self.push_item_kind(CoverageItemKind::Statement, &node.src);

                // visit left and right expressions
                // There could possibly a function call in the left or right expression
                // e.g: callFunc(a) + callFunc(b)
                if let Some(expr) = node.attribute("leftExpression") {
                    self.visit_expression(&expr)?;
                }

                if let Some(expr) = node.attribute("rightExpression") {
                    self.visit_expression(&expr)?;
                }

                Ok(())
            }
            // Does not count towards coverage
            NodeType::FunctionCallOptions
            | NodeType::Identifier
            | NodeType::IndexAccess
            | NodeType::IndexRangeAccess
            | NodeType::Literal
            | NodeType::YulLiteralValue
            | NodeType::YulIdentifier => Ok(()),
            _ => {
                warn!("unexpected node type, expected an expression: {:?}", node.node_type);
                Ok(())
            }
        }
    }

    fn visit_block_or_statement(&mut self, node: &Node) -> eyre::Result<()> {
        match node.node_type {
            NodeType::Block => self.visit_block(node),
            NodeType::Break
            | NodeType::Continue
            | NodeType::DoWhileStatement
            | NodeType::EmitStatement
            | NodeType::ExpressionStatement
            | NodeType::ForStatement
            | NodeType::IfStatement
            | NodeType::InlineAssembly
            | NodeType::Return
            | NodeType::RevertStatement
            | NodeType::TryStatement
            | NodeType::VariableDeclarationStatement
            | NodeType::YulVariableDeclaration
            | NodeType::WhileStatement => self.visit_statement(node),
            // Skip placeholder statements as they are never referenced in source maps.
            NodeType::PlaceholderStatement => Ok(()),
            _ => {
                warn!("unexpected node type, expected block or statement: {:?}", node.node_type);
                Ok(())
            }
        }
    }

    /// Creates a coverage item for a given kind and source location. Pushes item to the internal
    /// collection (plus additional coverage line if item is a statement).
    fn push_item_kind(&mut self, kind: CoverageItemKind, src: &ast::LowFidelitySourceLocation) {
        let item = CoverageItem { kind, loc: self.source_location_for(src), hits: 0 };

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

    fn source_location_for(&self, loc: &ast::LowFidelitySourceLocation) -> SourceLocation {
        let bytes_start = loc.start as u32;
        let bytes_end = (loc.start + loc.length.unwrap_or(0)) as u32;
        let bytes = bytes_start..bytes_end;

        let start_line = self.source[..bytes.start as usize].lines().count() as u32;
        let n_lines = self.source[bytes.start as usize..bytes.end as usize].lines().count() as u32;
        let lines = start_line..start_line + n_lines;
        SourceLocation {
            source_id: self.source_id as usize,
            contract_name: self.contract_name.clone(),
            bytes,
            lines,
        }
    }
}

/// Helper function to check if a given node is or contains any statement.
fn has_statements(node: &Node) -> bool {
    match node.node_type {
        NodeType::DoWhileStatement
        | NodeType::EmitStatement
        | NodeType::ExpressionStatement
        | NodeType::ForStatement
        | NodeType::IfStatement
        | NodeType::RevertStatement
        | NodeType::TryStatement
        | NodeType::VariableDeclarationStatement
        | NodeType::WhileStatement => true,
        _ => node.attribute::<Vec<Node>>("statements").is_some_and(|s| !s.is_empty()),
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
    pub fn new(data: &SourceFiles<'_>) -> eyre::Result<Self> {
        let mut sourced_items = data
            .sources
            .par_iter()
            .flat_map_iter(|(&source_id, SourceFile { source, ast })| {
                let items = ast.nodes.iter().map(move |node| {
                    if !matches!(node.node_type, NodeType::ContractDefinition) {
                        return Ok(vec![]);
                    }

                    // Skip interfaces which have no function implementations.
                    let contract_kind: String = node
                        .attribute("contractKind")
                        .ok_or_else(|| eyre::eyre!("Contract has no kind"))?;
                    if contract_kind == "interface" {
                        return Ok(vec![]);
                    }

                    let name = node
                        .attribute("name")
                        .ok_or_else(|| eyre::eyre!("Contract has no name"))?;
                    let _guard = debug_span!("visit_contract", %name).entered();
                    let mut visitor = ContractVisitor::new(source_id, &source.content, &name);
                    visitor.visit_contract(node)?;
                    visitor.clear_if_test();
                    visitor.disambiguate_functions();
                    Ok(visitor.items)
                });
                items.map(move |items| items.map(|items| (source_id, items)))
            })
            .collect::<eyre::Result<Vec<(usize, Vec<CoverageItem>)>>>()?;

        // Create mapping and merge items.
        sourced_items.sort_by_key(|(id, items)| (*id, items.first().map(|i| i.loc.bytes.start)));
        let Some(&(max_idx, _)) = sourced_items.last() else { return Ok(Self::default()) };
        let len = max_idx + 1;
        let mut all_items = Vec::new();
        let mut map = vec![(u32::MAX, 0); len];
        for (idx, items) in sourced_items {
            // Assumes that all `idx` items are consecutive, guaranteed by the sort above.
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
#[derive(Debug, Default)]
pub struct SourceFiles<'a> {
    /// The versioned sources.
    pub sources: HashMap<usize, SourceFile<'a>>,
}

/// The source code and AST of a file.
#[derive(Debug)]
pub struct SourceFile<'a> {
    /// The source code.
    pub source: Source,
    /// The AST of the source code.
    pub ast: &'a Ast,
}
