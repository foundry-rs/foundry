use super::{BranchKind, CoverageItem, SourceLocation};
use ethers::{
    prelude::sourcemap::SourceMap,
    solc::artifacts::ast::{self, Ast, Node, NodeType},
};
use std::collections::HashMap;
use tracing::warn;

#[derive(Debug, Default, Clone)]
pub struct Visitor {
    /// The source ID containing the AST being walked.
    source_id: u32,
    /// The source code that contains the AST being walked.
    source: String,
    /// Source maps for this specific source file, keyed by the contract name.
    source_maps: HashMap<String, SourceMap>,

    /// The contract whose AST we are currently walking
    context: String,
    /// The current branch ID
    // TODO: Does this need to be unique across files?
    branch_id: usize,
    /// Stores the last line we put in the items collection to ensure we don't push duplicate lines
    last_line: usize,

    /// Coverage items
    items: Vec<CoverageItem>,
}

impl Visitor {
    pub fn new(source_id: u32, source: String, source_maps: HashMap<String, SourceMap>) -> Self {
        Self { source_id, source, source_maps, ..Default::default() }
    }

    pub fn visit_ast(mut self, ast: Ast) -> eyre::Result<Vec<CoverageItem>> {
        // Walk AST
        for node in ast.nodes.into_iter() {
            if !matches!(node.node_type, NodeType::ContractDefinition) {
                continue
            }

            self.visit_contract(node)?;
        }

        Ok(self.items)
    }

    fn visit_contract(&mut self, node: Node) -> eyre::Result<()> {
        let is_contract =
            node.attribute("contractKind").map_or(false, |kind: String| kind == "contract");
        let is_abstract: bool = node.attribute("abstract").unwrap_or_default();

        // Skip interfaces, libraries and abstract contracts
        if !is_contract || is_abstract {
            return Ok(())
        }

        // Set the current context
        let contract_name: String =
            node.attribute("name").ok_or_else(|| eyre::eyre!("contract has no name"))?;
        self.context = contract_name;

        // Find all functions and walk their AST
        for node in node.nodes {
            if node.node_type == NodeType::FunctionDefinition {
                self.visit_function_definition(node)?;
            }
        }

        Ok(())
    }

    fn visit_function_definition(&mut self, mut node: Node) -> eyre::Result<()> {
        let name: String =
            node.attribute("name").ok_or_else(|| eyre::eyre!("function has no name"))?;
        let is_virtual: bool = node.attribute("virtual").unwrap_or_default();

        // Skip virtual functions
        if is_virtual {
            return Ok(())
        }

        match node.body.take() {
            // Skip virtual functions
            Some(body) if !is_virtual => {
                self.push_item(CoverageItem::Function {
                    name,
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&body.src),
                    hits: 0,
                });
                self.visit_block(*body)
            }
            _ => Ok(()),
        }
    }

    fn visit_block(&mut self, node: Node) -> eyre::Result<()> {
        let statements: Vec<Node> = node.attribute("statements").unwrap_or_default();

        for statement in statements {
            self.visit_statement(statement)?;
        }

        Ok(())
    }
    fn visit_statement(&mut self, node: Node) -> eyre::Result<()> {
        // TODO: YulSwitch, YulForLoop, YulFunctionDefinition, YulVariableDeclaration
        match node.node_type {
            // Blocks
            NodeType::Block | NodeType::UncheckedBlock | NodeType::YulBlock => {
                self.visit_block(node)
            }
            // Inline assembly block
            NodeType::InlineAssembly => self.visit_block(
                node.attribute("AST")
                    .ok_or_else(|| eyre::eyre!("inline assembly block with no AST attribute"))?,
            ),
            // Simple statements
            NodeType::Break |
            NodeType::Continue |
            NodeType::EmitStatement |
            NodeType::PlaceholderStatement |
            NodeType::Return |
            NodeType::RevertStatement |
            NodeType::YulAssignment |
            NodeType::YulBreak |
            NodeType::YulContinue |
            NodeType::YulLeave => {
                self.push_item(CoverageItem::Statement {
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src),
                    hits: 0,
                });
                Ok(())
            }
            // Variable declaration
            NodeType::VariableDeclarationStatement => {
                self.push_item(CoverageItem::Statement {
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src),
                    hits: 0,
                });
                if let Some(expr) = node.attribute("initialValue") {
                    self.visit_expression(expr)?;
                }
                Ok(())
            }
            // While loops
            NodeType::DoWhileStatement | NodeType::WhileStatement => {
                self.visit_expression(
                    node.attribute("condition")
                        .ok_or_else(|| eyre::eyre!("while statement had no condition"))?,
                )?;

                let body =
                    node.body.ok_or_else(|| eyre::eyre!("while statement had no body node"))?;
                self.visit_block_or_statement(*body)
            }
            // For loops
            NodeType::ForStatement => {
                if let Some(stmt) = node.attribute("initializationExpression") {
                    self.visit_statement(stmt)?;
                }
                if let Some(expr) = node.attribute("condition") {
                    self.visit_expression(expr)?;
                }
                if let Some(stmt) = node.attribute("loopExpression") {
                    self.visit_statement(stmt)?;
                }

                let body =
                    node.body.ok_or_else(|| eyre::eyre!("for statement had no body node"))?;
                self.visit_block_or_statement(*body)
            }
            // Expression statement
            NodeType::ExpressionStatement | NodeType::YulExpressionStatement => self
                .visit_expression(
                    node.attribute("expression")
                        .ok_or_else(|| eyre::eyre!("expression statement had no expression"))?,
                ),
            // If statement
            NodeType::IfStatement => {
                self.visit_expression(
                    node.attribute("condition")
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

                self.push_item(CoverageItem::Branch {
                    branch_id,
                    path_id: 0,
                    kind: BranchKind::True,
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&true_body.src),
                    hits: 0,
                });
                self.visit_block_or_statement(true_body)?;

                let false_body: Option<Node> = node.attribute("falseBody");
                if let Some(false_body) = false_body {
                    self.push_item(CoverageItem::Branch {
                        branch_id,
                        path_id: 1,
                        kind: BranchKind::False,
                        loc: self.source_location_for(&node.src),
                        anchor: self.anchor_for(&false_body.src),
                        hits: 0,
                    });
                    self.visit_block_or_statement(false_body)?;
                }

                Ok(())
            }
            NodeType::YulIf => {
                self.visit_expression(
                    node.attribute("condition")
                        .ok_or_else(|| eyre::eyre!("while statement had no condition"))?,
                )?;

                let body: Node = node
                    .attribute("body")
                    .ok_or_else(|| eyre::eyre!("yul if statement had no body"))?;

                // We need to store the current branch ID here since visiting the body of either of
                // the if blocks may increase `self.branch_id` in the case of nested if statements.
                let branch_id = self.branch_id;

                // We increase the branch ID here such that nested branches do not use the same
                // branch ID as we do
                self.branch_id += 1;

                self.push_item(CoverageItem::Branch {
                    branch_id,
                    path_id: 0,
                    kind: BranchKind::True,
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src),
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
                    node.attribute("externalCall")
                        .ok_or_else(|| eyre::eyre!("try statement had no call"))?,
                )
            }
            _ => {
                warn!("unexpected node type, expected a statement: {:?}", node.node_type);
                Ok(())
            }
        }
    }

    fn visit_expression(&mut self, node: Node) -> eyre::Result<()> {
        // TODO
        // elementarytypenameexpression
        //  memberaccess
        //  newexpression
        //  tupleexpression
        //  yulfunctioncall
        match node.node_type {
            NodeType::Assignment | NodeType::UnaryOperation | NodeType::BinaryOperation => {
                // TODO: Should we explore the subexpressions?
                self.push_item(CoverageItem::Statement {
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src),
                    hits: 0,
                });
                Ok(())
            }
            NodeType::FunctionCall => {
                // TODO: Handle assert and require
                self.push_item(CoverageItem::Statement {
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src),
                    hits: 0,
                });
                Ok(())
            }
            NodeType::Conditional => {
                // TODO: Do we count these as branches?
                self.push_item(CoverageItem::Statement {
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src),
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

    fn visit_block_or_statement(&mut self, node: Node) -> eyre::Result<()> {
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
            NodeType::PlaceholderStatement |
            NodeType::Return |
            NodeType::RevertStatement |
            NodeType::TryStatement |
            NodeType::VariableDeclarationStatement |
            NodeType::WhileStatement => self.visit_statement(node),
            _ => {
                warn!("unexpected node type, expected block or statement: {:?}", node.node_type);
                Ok(())
            }
        }
    }

    /// Pushes a coverage item to the internal collection, and might push a line item as well.
    fn push_item(&mut self, item: CoverageItem) {
        let source_location = item.source_location();

        // Push a line item if we haven't already
        if matches!(item, CoverageItem::Statement { .. } | CoverageItem::Branch { .. }) &&
            self.last_line < source_location.line
        {
            self.items.push(CoverageItem::Line {
                loc: source_location.clone(),
                anchor: item.anchor(),
                hits: 0,
            });
            self.last_line = source_location.line;
        }

        self.items.push(item);
    }

    fn source_location_for(&self, loc: &ast::SourceLocation) -> SourceLocation {
        SourceLocation {
            start: loc.start,
            length: loc.length,
            line: self.source[..loc.start].lines().count(),
        }
    }

    fn anchor_for(&self, loc: &ast::SourceLocation) -> usize {
        self.source_maps
            .get(&self.context)
            .and_then(|source_map| {
                source_map
                    .iter()
                    .enumerate()
                    .find(|(_, element)| {
                        element
                            .index
                            .and_then(|source_id| {
                                Some(
                                    source_id == self.source_id &&
                                        element.offset >= loc.start &&
                                        element.offset + element.length <=
                                            loc.start + loc.length?,
                                )
                            })
                            .unwrap_or_default()
                    })
                    .map(|(ic, _)| ic)
            })
            .unwrap_or(loc.start)
    }
}
