use super::{CoverageItem, ItemAnchor, SourceLocation};
use ethers::{
    prelude::{sourcemap::SourceMap, Bytes},
    solc::artifacts::ast::{self, Ast, Node, NodeType},
};
use revm::{opcode, spec_opcode_gas, SpecId};
use std::collections::{BTreeMap, HashMap};
use tracing::warn;

#[derive(Debug, Default, Clone)]
pub struct Visitor {
    /// The source code that contains the AST being walked.
    source: String,
    /// Source maps for this specific source file, keyed by the contract name.
    source_maps: HashMap<String, SourceMap>,
    /// Bytecodes for this specific source file, keyed by the contract name.
    bytecodes: HashMap<String, Bytes>,

    /// The contract whose AST we are currently walking
    context: String,
    /// The current branch ID
    branch_id: usize,
    /// Stores the last line we put in the items collection to ensure we don't push duplicate lines
    last_line: usize,

    /// Coverage items
    items: Vec<CoverageItem>,
}

impl Visitor {
    pub fn new(
        source: String,
        source_maps: HashMap<String, SourceMap>,
        bytecodes: HashMap<String, Bytes>,
    ) -> Self {
        Self { source, source_maps, bytecodes, ..Default::default() }
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

        // TODO(onbjerg): Re-enable constructor parsing when we walk both the deployment and runtime
        // sourcemaps. Currently this fails because we are trying to look for anchors in the runtime
        // sourcemap.
        // TODO(onbjerg): Figure out why we cannot find anchors for the receive function
        let kind: String =
            node.attribute("kind").ok_or_else(|| eyre::eyre!("function has no kind"))?;
        if kind == "constructor" || kind == "receive" {
            return Ok(())
        }

        match node.body.take() {
            // Skip virtual functions
            Some(body) if !is_virtual => {
                self.push_item(CoverageItem::Function {
                    name: format!("{}.{}", self.context, name),
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&body.src)?,
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
                    anchor: self.anchor_for(&node.src)?,
                    hits: 0,
                });
                Ok(())
            }
            // Variable declaration
            NodeType::VariableDeclarationStatement => {
                self.push_item(CoverageItem::Statement {
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src)?,
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

                let (true_branch, false_branch) = self.find_branches(&node.src, branch_id)?;

                // Process the true branch
                self.push_item(true_branch);
                self.visit_block_or_statement(true_body)?;

                // Process the false branch
                self.push_item(false_branch);
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

                self.push_item(CoverageItem::Branch {
                    branch_id,
                    path_id: 0,
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src)?,
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
                self.push_item(CoverageItem::Statement {
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src)?,
                    hits: 0,
                });
                Ok(())
            }
            NodeType::FunctionCall => {
                self.push_item(CoverageItem::Statement {
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src)?,
                    hits: 0,
                });

                let name = node
                    .other
                    .get("expression")
                    .and_then(|v| v.get("name"))
                    .and_then(|v| v.as_str());
                if let Some("assert" | "require") = name {
                    let (false_branch, true_branch) =
                        self.find_branches(&node.src, self.branch_id)?;
                    self.push_item(true_branch);
                    self.push_item(false_branch);
                    self.branch_id += 1;
                }
                Ok(())
            }
            NodeType::Conditional => {
                self.push_item(CoverageItem::Statement {
                    loc: self.source_location_for(&node.src),
                    anchor: self.anchor_for(&node.src)?,
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
                anchor: item.anchor().clone(),
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

    /// Maps a source range to a single instruction, called an anchor.
    ///
    /// If the anchor is executed at any point, then the source range defined by `loc` is considered
    /// to be covered.
    fn anchor_for(&self, loc: &ast::SourceLocation) -> eyre::Result<ItemAnchor> {
        let source_map = self.source_maps.get(&self.context).ok_or_else(|| {
            eyre::eyre!(
                "could not find anchor for node: we do not have a source map for {}",
                self.context
            )
        })?;

        let instruction = source_map
            .iter()
            .enumerate()
            .find_map(|(ic, element)| {
                if element.index? as usize == loc.index? &&
                    element.offset >= loc.start &&
                    element.offset + element.length <= loc.start + loc.length?
                {
                    return Some(ic)
                }

                None
            })
            .ok_or_else(|| {
                eyre::eyre!(
                    "could not find anchor: no matching instruction in range {}:{}",
                    self.context,
                    loc
                )
            })?;

        Ok(ItemAnchor { instruction, contract: self.context.clone() })
    }

    /// Finds the true and false branches for a node.
    ///
    /// This finds the relevant anchors and creates coverage items for both of them. These anchors
    /// are found using the bytecode of the contract in the range of the branching node.
    ///
    /// For `IfStatement` nodes, the template is generally:
    /// ```text
    /// <condition>
    /// PUSH <ic if false>
    /// JUMPI
    /// <true branch>
    /// <...>
    /// <false branch>
    /// ```
    ///
    /// For `assert` and `require`, the template is generally:
    ///
    /// ```text
    /// PUSH <ic if true>
    /// JUMPI
    /// <revert>
    /// <...>
    /// <true branch>
    /// ```
    ///
    /// This function will look for the last JUMPI instruction, backtrack to find the instruction
    /// counter of the first branch, and return an item for that instruction counter, and the
    /// instruction counter immediately after the JUMPI instruction.
    fn find_branches(
        &self,
        loc: &ast::SourceLocation,
        branch_id: usize,
    ) -> eyre::Result<(CoverageItem, CoverageItem)> {
        let source_map = self.source_maps.get(&self.context).ok_or_else(|| {
            eyre::eyre!(
                "could not find anchors for branches: we do not have a source map for {}",
                self.context
            )
        })?;
        let bytecode = self
            .bytecodes
            .get(&self.context)
            .ok_or_else(|| {
                eyre::eyre!(
                    "could not find anchors for branches: we do not have the bytecode for {}",
                    self.context
                )
            })?
            .clone();

        // NOTE(onbjerg): We use `SpecId::LATEST` here since it does not matter; the only difference
        // is the gas cost.
        let opcode_infos = spec_opcode_gas(SpecId::LATEST);

        let mut ic_map: BTreeMap<usize, usize> = BTreeMap::new();
        let mut first_branch_ic = None;
        let mut second_branch_pc = None;
        let mut pc = 0;
        let mut cumulative_push_size = 0;
        while pc < bytecode.0.len() {
            let op = bytecode.0[pc];
            ic_map.insert(pc, pc - cumulative_push_size);

            // We found a push, so we do some PC -> IC translation accounting, but we also check if
            // this push is coupled with the JUMPI we are interested in.
            if opcode_infos[op as usize].is_push {
                let element = if let Some(element) = source_map.get(pc - cumulative_push_size) {
                    element
                } else {
                    // NOTE(onbjerg): For some reason the last few bytes of the bytecode do not have
                    // a source map associated, so at that point we just stop searching
                    break
                };

                // Do push byte accounting
                let push_size = (op - opcode::PUSH1 + 1) as usize;
                pc += push_size;
                cumulative_push_size += push_size;

                // Check if we are in the source range we are interested in, and if the next opcode
                // is a JUMPI
                let source_ids_match =
                    element.index.zip(loc.index).map_or(false, |(a, b)| a as usize == b);
                let is_in_source_range = element.offset >= loc.start &&
                    element.offset + element.length <=
                        loc.start + loc.length.unwrap_or_default();
                if source_ids_match && is_in_source_range && bytecode.0[pc + 1] == opcode::JUMPI {
                    // We do not support program counters bigger than usize. This is also an
                    // assumption in REVM, so this is just a sanity check.
                    if push_size > 8 {
                        eyre::bail!("we found a branch, but it refers to a program counter bigger than 64 bits.");
                    }

                    // The first branch is the opcode directly after JUMPI
                    first_branch_ic = Some(pc + 2 - cumulative_push_size);

                    // Convert the push bytes for the second branch's PC to a usize
                    let push_bytes_start = pc - push_size + 1;
                    let mut pc_bytes: [u8; 8] = [0; 8];
                    for (i, push_byte) in bytecode.0[push_bytes_start..push_bytes_start + push_size]
                        .iter()
                        .enumerate()
                    {
                        pc_bytes[8 - push_size + i] = *push_byte;
                    }
                    second_branch_pc = Some(usize::from_be_bytes(pc_bytes));
                }
            }
            pc += 1;
        }

        match (first_branch_ic, second_branch_pc) {
            (Some(first_branch_ic), Some(second_branch_pc)) => Ok((
                CoverageItem::Branch {
                    branch_id,
                    path_id: 0,
                    loc: self.source_location_for(loc),
                    anchor: ItemAnchor {
                        instruction: first_branch_ic,
                        contract: self.context.clone()
                    },
                    hits: 0,
                },
                CoverageItem::Branch {
                    branch_id,
                    path_id: 1,
                    loc: self.source_location_for(loc),
                    anchor: ItemAnchor {
                        instruction: *ic_map.get(&second_branch_pc).expect("we cannot translate the program counter of the second branch to an instruction counter"),
                        contract: self.context.clone()
                    },
                    hits: 0
                }
            )),
            _ => eyre::bail!("could not detect branches in range {}", loc)
        }
    }
}
