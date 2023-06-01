use super::{ContractId, CoverageItem, CoverageItemKind, SourceLocation};
use ethers::solc::artifacts::ast::{self, Ast, Node, NodeType};
use foundry_common::TestFunctionExt;
use semver::Version;
use std::collections::{HashMap, HashSet};

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
    pub items: Vec<CoverageItem>,

    /// Node IDs of this contract's base contracts, as well as IDs for referenced contracts such as
    /// libraries
    pub base_contract_node_ids: HashSet<usize>,
}

impl<'a> ContractVisitor<'a> {
    pub fn new(source_id: usize, source: &'a str, contract_name: String) -> Self {
        Self {
            source_id,
            source,
            contract_name,
            branch_id: 0,
            last_line: 0,
            items: Vec::new(),
            base_contract_node_ids: HashSet::new(),
        }
    }

    pub fn visit(mut self, contract_ast: Node) -> eyre::Result<Self> {
        let linearized_base_contracts: Vec<usize> =
            contract_ast.attribute("linearizedBaseContracts").ok_or_else(|| {
                eyre::eyre!(
                    "The contract's AST node is missing a list of linearized base contracts"
                )
            })?;

        // We skip the first ID because that's the ID of the contract itself
        self.base_contract_node_ids.extend(&linearized_base_contracts[1..]);

        // Find all functions and walk their AST
        for node in contract_ast.nodes {
            if node.node_type == NodeType::FunctionDefinition {
                self.visit_function_definition(node.clone())?;
            }
        }

        Ok(self)
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

                // The relevant source range for the branch is the `if(...)` statement itself and
                // the true body of the if statement. The false body of the statement (if any) is
                // processed as its own thing. If this source range is not processed like this, it
                // is virtually impossible to correctly map instructions back to branches that
                // include more complex logic like conditional logic.
                self.push_branches(
                    &ethers::solc::artifacts::ast::LowFidelitySourceLocation {
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

                let expr: Option<Node> = node.attribute("expression");
                match expr.as_ref().map(|expr| &expr.node_type) {
                    // Might be a require/assert call
                    Some(NodeType::Identifier) => {
                        let name: Option<String> = expr.and_then(|expr| expr.attribute("name"));
                        if let Some("assert" | "require") = name.as_deref() {
                            self.push_branches(&node.src, self.branch_id);
                            self.branch_id += 1;
                        }
                    }
                    // Might be a call to a library
                    Some(NodeType::MemberAccess) => {
                        let referenced_declaration_id = expr
                            .and_then(|expr| expr.attribute("expression"))
                            .and_then(|subexpr: Node| subexpr.attribute("referencedDeclaration"));
                        if let Some(id) = referenced_declaration_id {
                            self.base_contract_node_ids.insert(id);
                        }
                    }
                    _ => (),
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

    fn source_location_for(&self, loc: &ast::LowFidelitySourceLocation) -> SourceLocation {
        SourceLocation {
            source_id: self.source_id,
            contract_name: self.contract_name.clone(),
            start: loc.start,
            length: loc.length,
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

#[derive(Debug)]
pub struct SourceAnalysis {
    /// A collection of coverage items.
    pub items: Vec<CoverageItem>,
    /// A mapping of contract IDs to item IDs relevant to the contract (including items in base
    /// contracts).
    pub contract_items: HashMap<ContractId, Vec<usize>>,
}

/// Analyzes a set of sources to find coverage items.
#[derive(Default, Clone, Debug)]
pub struct SourceAnalyzer {
    /// A map of source IDs to their source code
    sources: HashMap<usize, String>,
    /// A map of AST node IDs of contracts to their contract IDs.
    contract_ids: HashMap<usize, ContractId>,
    /// A map of contract IDs to their AST nodes.
    contracts: HashMap<ContractId, Node>,
    /// A collection of coverage items.
    items: Vec<CoverageItem>,
    /// A map of contract IDs to item IDs.
    contract_items: HashMap<ContractId, Vec<usize>>,
    /// A map of contracts to their base contracts
    contract_bases: HashMap<ContractId, Vec<ContractId>>,
}

impl SourceAnalyzer {
    /// Creates a new source analyzer.
    ///
    /// The source analyzer expects all given sources to belong to the same compilation job
    /// (defined by `version`).
    pub fn new(
        version: Version,
        asts: HashMap<usize, Ast>,
        sources: HashMap<usize, String>,
    ) -> eyre::Result<Self> {
        let mut analyzer = SourceAnalyzer { sources, ..Default::default() };

        // TODO: Skip interfaces
        for (source_id, ast) in asts.into_iter() {
            for child in ast.nodes {
                if !matches!(child.node_type, NodeType::ContractDefinition) {
                    continue
                }

                let node_id =
                    child.id.ok_or_else(|| eyre::eyre!("The contract's AST node has no ID"))?;
                let contract_id = ContractId {
                    version: version.clone(),
                    source_id,
                    contract_name: child
                        .attribute("name")
                        .ok_or_else(|| eyre::eyre!("Contract has no name"))?,
                };
                analyzer.contract_ids.insert(node_id, contract_id.clone());
                analyzer.contracts.insert(contract_id, child);
            }
        }

        Ok(analyzer)
    }

    /// Analyzes contracts in the sources held by the source analyzer.
    ///
    /// Coverage items are found by:
    /// - Walking the AST of each contract (except interfaces)
    /// - Recording the items and base contracts of each contract
    ///
    /// Finally, the item IDs of each contract and its base contracts are flattened, and the return
    /// value represents all coverage items in the project, along with a mapping of contract IDs to
    /// item IDs.
    ///
    /// Each coverage item contains relevant information to find opcodes corresponding to them: the
    /// source ID the item is in, the source code range of the item, and the contract name the item
    /// is in.
    ///
    /// Note: Source IDs are only unique per compilation job; that is, a code base compiled with
    /// two different solc versions will produce overlapping source IDs if the compiler version is
    /// not taken into account.
    pub fn analyze(mut self) -> eyre::Result<SourceAnalysis> {
        // Analyze the contracts
        self.analyze_contracts()?;

        // Flatten the data
        let mut flattened: HashMap<ContractId, Vec<usize>> = HashMap::new();
        for (contract_id, own_item_ids) in &self.contract_items {
            let mut item_ids = own_item_ids.clone();
            if let Some(base_contract_ids) = self.contract_bases.get(contract_id) {
                for item_id in
                    base_contract_ids.iter().filter_map(|id| self.contract_items.get(id)).flatten()
                {
                    item_ids.push(*item_id);
                }
            }

            // If there are no items for this contract, then it was most likely filtered
            if !item_ids.is_empty() {
                flattened.insert(contract_id.clone(), item_ids);
            }
        }

        Ok(SourceAnalysis { items: self.items.clone(), contract_items: flattened })
    }

    fn analyze_contracts(&mut self) -> eyre::Result<()> {
        for contract_id in self.contracts.keys() {
            // Find this contract's coverage items if we haven't already
            if self.contract_items.get(contract_id).is_none() {
                let ContractVisitor { items, base_contract_node_ids, .. } = ContractVisitor::new(
                    contract_id.source_id,
                    self.sources.get(&contract_id.source_id).unwrap_or_else(|| {
                        panic!(
                            "We should have the source code for source ID {}",
                            contract_id.source_id
                        )
                    }),
                    contract_id.contract_name.clone(),
                )
                .visit(
                    self.contracts
                        .get(contract_id)
                        .unwrap_or_else(|| {
                            panic!("We should have the AST of contract: {contract_id:?}")
                        })
                        .clone(),
                )?;

                let is_test = items.iter().any(|item| {
                    if let CoverageItemKind::Function { name } = &item.kind {
                        name.is_test()
                    } else {
                        false
                    }
                });

                // Record this contract's base contracts
                // We don't do this for test contracts because we don't care about them
                if !is_test {
                    self.contract_bases.insert(
                        contract_id.clone(),
                        base_contract_node_ids
                            .iter()
                            .filter_map(|base_contract_node_id| {
                                self.contract_ids.get(base_contract_node_id).cloned()
                            })
                            .collect(),
                    );
                }

                // For tests and contracts with no items we still record an empty Vec so we don't
                // end up here again
                if items.is_empty() || is_test {
                    self.contract_items.insert(contract_id.clone(), Vec::new());
                } else {
                    let item_ids: Vec<usize> =
                        (self.items.len()..self.items.len() + items.len()).collect();
                    self.items.extend(items);
                    self.contract_items.insert(contract_id.clone(), item_ids.clone());
                }
            }
        }

        Ok(())
    }
}
