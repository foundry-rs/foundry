use super::{BranchKind, CoverageItem};
use ethers::solc::artifacts::ast::{Ast, Node, NodeType};
use tracing::warn;

#[derive(Debug, Default, Clone)]
pub struct Visitor {
    /// Coverage items
    pub items: Vec<CoverageItem>,
    /// The current branch ID
    // TODO: Does this need to be unique across files?
    pub branch_id: usize,
}

impl Visitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visit_ast(&mut self, ast: Ast) -> eyre::Result<()> {
        for node in ast.nodes.into_iter() {
            if !matches!(node.node_type, NodeType::ContractDefinition) {
                continue
            }

            self.visit_contract(node)?;
        }

        Ok(())
    }

    pub fn visit_contract(&mut self, node: Node) -> eyre::Result<()> {
        let is_contract =
            node.attribute("contractKind").map_or(false, |kind: String| kind == "contract");
        let is_abstract: bool = node.attribute("abstract").unwrap_or_default();

        // Skip interfaces, libraries and abstract contracts
        if !is_contract || is_abstract {
            return Ok(())
        }

        for node in node.nodes {
            if node.node_type == NodeType::FunctionDefinition {
                self.visit_function_definition(node)?;
            }
        }

        Ok(())
    }

    pub fn visit_function_definition(&mut self, mut node: Node) -> eyre::Result<()> {
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
                self.items.push(CoverageItem::Function { name, offset: node.src.start, hits: 0 });
                self.visit_block(*body)
            }
            _ => Ok(()),
        }
    }

    pub fn visit_block(&mut self, node: Node) -> eyre::Result<()> {
        let statements: Vec<Node> = node.attribute("statements").unwrap_or_default();

        for statement in statements {
            self.visit_statement(statement)?;
        }

        Ok(())
    }
    pub fn visit_statement(&mut self, node: Node) -> eyre::Result<()> {
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
                self.items.push(CoverageItem::Statement { offset: node.src.start, hits: 0 });
                Ok(())
            }
            // Variable declaration
            NodeType::VariableDeclarationStatement => {
                self.items.push(CoverageItem::Statement { offset: node.src.start, hits: 0 });
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

                self.items.push(CoverageItem::Branch {
                    branch_id,
                    path_id: 0,
                    kind: BranchKind::True,
                    offset: true_body.src.start,
                    hits: 0,
                });
                self.visit_block_or_statement(true_body)?;

                let false_body: Option<Node> = node.attribute("falseBody");
                if let Some(false_body) = false_body {
                    self.items.push(CoverageItem::Branch {
                        branch_id,
                        path_id: 1,
                        kind: BranchKind::False,
                        offset: false_body.src.start,
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

                self.items.push(CoverageItem::Branch {
                    branch_id,
                    path_id: 0,
                    kind: BranchKind::True,
                    offset: body.src.start,
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

    pub fn visit_expression(&mut self, node: Node) -> eyre::Result<()> {
        // TODO
        // elementarytypenameexpression
        //  memberaccess
        //  newexpression
        //  tupleexpression
        //  yulfunctioncall
        match node.node_type {
            NodeType::Assignment | NodeType::UnaryOperation | NodeType::BinaryOperation => {
                // TODO: Should we explore the subexpressions?
                self.items.push(CoverageItem::Statement { offset: node.src.start, hits: 0 });
                Ok(())
            }
            NodeType::FunctionCall => {
                // TODO: Handle assert and require
                self.items.push(CoverageItem::Statement { offset: node.src.start, hits: 0 });
                Ok(())
            }
            NodeType::Conditional => {
                // TODO: Do we count these as branches?
                self.items.push(CoverageItem::Statement { offset: node.src.start, hits: 0 });
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

    pub fn visit_block_or_statement(&mut self, node: Node) -> eyre::Result<()> {
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
}
