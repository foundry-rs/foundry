use super::{CoverageItem, CoverageItemKind, SourceLocation};
use ethers::solc::artifacts::ast::{self, Node, NodeType};
use tracing::warn;

/// A visitor that walks the AST of a single contract and finds coverage items.
#[derive(Debug, Clone)]
pub struct ContractVisitor<'a> {
    /// The source ID of the contract.
    source_id: usize,
    /// The source code that contains the AST being walked.
    source: &'a str,

    /// The name of the contract being walked.
    contract_name: String,

    /// The current branch ID
    branch_id: usize,
    /// Stores the last line we put in the items collection to ensure we don't push duplicate lines
    last_line: usize,

    /// Coverage items
    items: Vec<CoverageItem>,
}

impl<'a> ContractVisitor<'a> {
    pub fn new(source_id: usize, source: &'a str, contract_name: String) -> Self {
        Self { source_id, source, contract_name, branch_id: 0, last_line: 0, items: Vec::new() }
    }

    pub fn visit(mut self, contract_ast: Node) -> eyre::Result<Vec<CoverageItem>> {
        // Find all functions and walk their AST
        for node in contract_ast.nodes {
            if node.node_type == NodeType::FunctionDefinition {
                self.visit_function_definition(node.clone())?;
            }
        }

        Ok(self.items)
    }

    fn visit_function_definition(&mut self, mut node: Node) -> eyre::Result<()> {
        let name: String =
            node.attribute("name").ok_or_else(|| eyre::eyre!("Function has no name"))?;

        // TODO(onbjerg): Re-enable constructor parsing when we walk both the deployment and runtime
        // sourcemaps. Currently this fails because we are trying to look for anchors in the runtime
        // sourcemap.
        // TODO(onbjerg): Figure out why we cannot find anchors for the receive function
        let kind: String =
            node.attribute("kind").ok_or_else(|| eyre::eyre!("Function has no kind"))?;
        if kind == "constructor" || kind == "receive" {
            return Ok(())
        }

        match node.body.take() {
            Some(body) => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Function { name },
                    loc: self.source_location_for(&node.src),
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
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Statement,
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });
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

                self.push_branches(&node.src, branch_id);

                // Process the true branch
                self.visit_block_or_statement(true_body)?;

                // Process the false branch
                let false_body: Option<Node> = node.attribute("falseBody");
                if let Some(false_body) = false_body {
                    self.visit_block_or_statement(false_body)?;
                }

                Ok(())
            }
            NodeType::YulIf => {
                self.visit_expression(
                    node.attribute("condition")
                        .ok_or_else(|| eyre::eyre!("yul if statement had no condition"))?,
                )?;
                let body = node.body.ok_or_else(|| eyre::eyre!("yul if statement had no body"))?;

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
                self.visit_block(*body)?;

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
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Statement,
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });
                Ok(())
            }
            NodeType::FunctionCall => {
                self.push_item(CoverageItem {
                    kind: CoverageItemKind::Statement,
                    loc: self.source_location_for(&node.src),
                    hits: 0,
                });

                let name = node
                    .other
                    .get("expression")
                    .and_then(|v| v.get("name"))
                    .and_then(|v| v.as_str());
                if let Some("assert" | "require") = name {
                    self.push_branches(&node.src, self.branch_id);
                    self.branch_id += 1;
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

    fn source_location_for(&self, loc: &ast::SourceLocation) -> SourceLocation {
        SourceLocation {
            source_id: self.source_id,
            contract_name: self.contract_name.clone(),
            start: loc.start,
            length: loc.length,
            line: self.source[..loc.start].lines().count(),
        }
    }

    fn push_branches(&mut self, loc: &ast::SourceLocation, branch_id: usize) {
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
