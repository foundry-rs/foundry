use super::{CoverageItem, CoverageItemKind, SourceLocation};
use foundry_common::TestFunctionExt;
use foundry_compilers::artifacts::ast::{self, Ast, Node, NodeType};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// A visitor that walks the AST of a single contract and finds coverage items.
#[derive(Clone, Debug)]
pub struct ContractVisitor<'a> {
    /// The source ID of the contract.
    source_id: usize,
    /// The source code that contains the AST being walked.
    source: &'a str,

    /// The name of the contract being walked.
    contract_name: &'a Arc<str>,

    /// The current branch ID
    branch_id: usize,
    /// Stores the last line we put in the items collection to ensure we don't push duplicate lines
    last_line: usize,

    /// Coverage items
    pub items: Vec<CoverageItem>,
}

impl<'a> ContractVisitor<'a> {
    pub fn new(source_id: usize, source: &'a str, contract_name: &'a Arc<str>) -> Self {
        Self { source_id, source, contract_name, branch_id: 0, last_line: 0, items: Vec::new() }
    }

    pub fn visit_contract(&mut self, node: &Node) -> eyre::Result<()> {
        // Find all functions and walk their AST
        for node in &node.nodes {
            match node.node_type {
                NodeType::FunctionDefinition => {
                    self.visit_function_definition(node)?;
                }
                NodeType::ModifierDefinition => {
                    self.visit_modifier_definition(node)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn visit_function_definition(&mut self, node: &Node) -> eyre::Result<()> {
        let name: String =
            node.attribute("name").ok_or_else(|| eyre::eyre!("Function has no name"))?;

        // TODO(onbjerg): Figure out why we cannot find anchors for the receive function
        let kind: String =
            node.attribute("kind").ok_or_else(|| eyre::eyre!("Function has no kind"))?;
        if kind == "receive" {
            return Ok(())
        }

        match &node.body {
            Some(body) => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Function { name },
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });
                self.visit_block(body)
            }
            _ => Ok(()),
        }
    }

    fn visit_modifier_definition(&mut self, node: &Node) -> eyre::Result<()> {
        let name: String =
            node.attribute("name").ok_or_else(|| eyre::eyre!("Modifier has no name"))?;

        match &node.body {
            Some(body) => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Function { name },
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });
                self.visit_block(body)
            }
            _ => Ok(()),
        }
    }

    fn visit_block(&mut self, node: &Node) -> eyre::Result<()> {
        let statements: Vec<Node> = node.attribute("statements").unwrap_or_default();

        for statement in &statements {
            self.visit_statement(statement)?;
        }

        Ok(())
    }

    fn visit_statement(&mut self, node: &Node) -> eyre::Result<()> {
        // TODO: YulSwitch, YulForLoop, YulFunctionDefinition, YulVariableDeclaration
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
            NodeType::Break |
            NodeType::Continue |
            NodeType::EmitStatement |
            NodeType::RevertStatement |
            NodeType::YulAssignment |
            NodeType::YulBreak |
            NodeType::YulContinue |
            NodeType::YulLeave => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Statement,
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });
                Ok(())
            }

            // Skip placeholder statements as they are never referenced in source maps.
            NodeType::PlaceholderStatement => Ok(()),

            // Return with eventual subcall
            NodeType::Return => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Statement,
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });
                if let Some(expr) = node.attribute("expression") {
                    self.visit_expression(&expr)?;
                }
                Ok(())
            }

            // Variable declaration
            NodeType::VariableDeclarationStatement => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Statement,
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });
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
                        .ok_or_else(|| eyre::eyre!("while statement had no condition"))?,
                )?;

                let true_body: Node = node
                    .attribute("trueBody")
                    .ok_or_else(|| eyre::eyre!("if statement had no true body"))?;

                // We need to store the current branch ID here since visiting the body of either of
                // the if blocks may increase `self.branch_id` in the case of nested if statements.
                let branch_id = self.branch_id;

                // We increase the branch ID here such that nested branches do not use the same
                // branch ID as we do
                self.branch_id += 1;

                // The relevant source range for the branch is the `if(...)` statement itself and
                // the true body of the if statement. The false body of the statement (if any) is
                // processed as its own thing. If this source range is not processed like this, it
                // is virtually impossible to correctly map instructions back to branches that
                // include more complex logic like conditional logic.
                self.push_branches(
                    &foundry_compilers::artifacts::ast::LowFidelitySourceLocation {
                        start: node.src.start,
                        length: true_body
                            .src
                            .length
                            .map(|length| true_body.src.start - node.src.start + length),
                        index: node.src.index,
                    },
                    branch_id,
                );

                // Process the true branch
                self.visit_block_or_statement(&true_body)?;

                // Process the false branch
                if let Some(false_body) = node.attribute("falseBody") {
                    self.visit_block_or_statement(&false_body)?;
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

                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Branch { branch_id, path_id: 0 },
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });
                self.visit_block(body)?;

                Ok(())
            }
            // Try-catch statement
            NodeType::TryStatement => {
                // TODO: Clauses
                // TODO: This is branching, right?
                self.visit_expression(
                    &node
                        .attribute("externalCall")
                        .ok_or_else(|| eyre::eyre!("try statement had no call"))?,
                )
            }
            _ => {
                warn!("unexpected node type, expected a statement: {:?}", node.node_type);
                Ok(())
            }
        }
    }

    fn visit_expression(&mut self, node: &Node) -> eyre::Result<()> {
        // TODO
        // elementarytypenameexpression
        //  memberaccess
        //  newexpression
        //  tupleexpression
        //  yulfunctioncall
        match node.node_type {
            NodeType::Assignment | NodeType::UnaryOperation => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Statement,
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });
                Ok(())
            }
            NodeType::BinaryOperation => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Statement,
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });

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
            NodeType::FunctionCall => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Statement,
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });

                let expr: Option<Node> = node.attribute("expression");
                if let Some(NodeType::Identifier) = expr.as_ref().map(|expr| &expr.node_type) {
                    // Might be a require/assert call
                    let name: Option<String> = expr.and_then(|expr| expr.attribute("name"));
                    if let Some("assert" | "require") = name.as_deref() {
                        self.push_branches(&node.src, self.branch_id);
                        self.branch_id += 1;
                    }
                }

                Ok(())
            }
            NodeType::Conditional => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Statement,
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });
                Ok(())
            }
            // Does not count towards coverage
            NodeType::FunctionCallOptions |
            NodeType::Identifier |
            NodeType::IndexAccess |
            NodeType::IndexRangeAccess |
            NodeType::Literal |
            NodeType::YulLiteralValue |
            NodeType::YulIdentifier => Ok(()),
            _ => {
                warn!("unexpected node type, expected an expression: {:?}", node.node_type);
                Ok(())
            }
        }
    }

    fn visit_block_or_statement(&mut self, node: &Node) -> eyre::Result<()> {
        match node.node_type {
            NodeType::Block => self.visit_block(node),
            NodeType::Break |
            NodeType::Continue |
            NodeType::DoWhileStatement |
            NodeType::EmitStatement |
            NodeType::ExpressionStatement |
            NodeType::ForStatement |
            NodeType::IfStatement |
            NodeType::InlineAssembly |
            NodeType::Return |
            NodeType::RevertStatement |
            NodeType::TryStatement |
            NodeType::VariableDeclarationStatement |
            NodeType::WhileStatement => self.visit_statement(node),
            // Skip placeholder statements as they are never referenced in source maps.
            NodeType::PlaceholderStatement => Ok(()),
            _ => {
                warn!("unexpected node type, expected block or statement: {:?}", node.node_type);
                Ok(())
            }
        }
    }

    /// Pushes a coverage item to the internal collection, and might push a line item as well.
    fn push_item(&mut self, item: CoverageItem) {
        let source_location = &item.loc;

        // Push a line item if we haven't already
        if matches!(item.kind, CoverageItemKind::Statement | CoverageItemKind::Branch { .. }) &&
            self.last_line < source_location.line
        {
            self.items.push(CoverageItem {
                kind: CoverageItemKind::Line,
                loc: source_location.clone(),
                hits: 0,
            });
            self.last_line = source_location.line;
        }

        self.items.push(item);
    }

    fn source_location_for(&self, loc: &ast::LowFidelitySourceLocation) -> SourceLocation {
        SourceLocation {
            source_id: self.source_id,
            contract_name: self.contract_name.clone(),
            start: loc.start as u32,
            length: loc.length.map(|x| x as u32),
            line: self.source[..loc.start].lines().count(),
        }
    }

    fn push_branches(&mut self, loc: &ast::LowFidelitySourceLocation, branch_id: usize) {
        self.push_item(CoverageItem {
            kind: CoverageItemKind::Branch { branch_id, path_id: 0 },
            loc: self.source_location_for(loc),
            hits: 0,
        });
        self.push_item(CoverageItem {
            kind: CoverageItemKind::Branch { branch_id, path_id: 1 },
            loc: self.source_location_for(loc),
            hits: 0,
        });
    }
}

/// [`SourceAnalyzer`] result type.
#[derive(Debug)]
pub struct SourceAnalysis {
    /// A collection of coverage items.
    pub items: Vec<CoverageItem>,
}

/// Analyzes a set of sources to find coverage items.
#[derive(Debug)]
pub struct SourceAnalyzer<'a> {
    sources: &'a SourceFiles<'a>,
}

impl<'a> SourceAnalyzer<'a> {
    /// Creates a new source analyzer.
    pub fn new(data: &'a SourceFiles<'a>) -> Self {
        Self { sources: data }
    }

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
    pub fn analyze(&self) -> eyre::Result<SourceAnalysis> {
        let items = self
            .sources
            .sources
            .par_iter()
            .flat_map_iter(|(&source_id, SourceFile { source, ast })| {
                ast.nodes.iter().map(move |node| {
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

                    let mut visitor = ContractVisitor::new(source_id, source, &name);
                    visitor.visit_contract(node)?;
                    let mut items = visitor.items;

                    let is_test = items.iter().any(|item| {
                        if let CoverageItemKind::Function { name } = &item.kind {
                            name.is_any_test()
                        } else {
                            false
                        }
                    });
                    if is_test {
                        items.clear();
                    }

                    Ok(items)
                })
            })
            .collect::<eyre::Result<Vec<Vec<_>>>>()?;
        Ok(SourceAnalysis { items: items.concat() })
    }
}

/// A list of versioned sources and their ASTs.
#[derive(Debug, Default)]
pub struct SourceFiles<'a> {
    /// The versioned sources.
    pub sources: FxHashMap<usize, SourceFile<'a>>,
}

/// The source code and AST of a file.
#[derive(Debug)]
pub struct SourceFile<'a> {
    /// The source code.
    pub source: String,
    /// The AST of the source code.
    pub ast: &'a Ast,
}
