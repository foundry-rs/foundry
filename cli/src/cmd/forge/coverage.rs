//! Coverage command
use crate::{
    cmd::{
        forge::{build::CoreBuildArgs, test::Filter},
        Cmd,
    },
    compile::ProjectCompiler,
    utils,
};
use cast::coverage::{BranchKind, CoverageItem, CoverageMap, CoverageSummary};
use clap::{AppSettings, ArgEnum, Parser};
use comfy_table::{Cell, Color, Row, Table};
use ethers::{
    prelude::{artifacts::Node, Artifact, Project, ProjectCompileOutput},
    solc::{
        artifacts::{
            ast::{Ast, NodeType},
            contract::CompactContractBytecode,
        },
        sourcemap::{self, SourceMap},
        ArtifactId,
    },
};
use forge::{
    executor::opts::EvmOpts,
    result::SuiteResult,
    trace::{identifier::LocalTraceIdentifier, CallTraceDecoder, CallTraceDecoderBuilder},
    MultiContractRunnerBuilder,
};
use foundry_common::evm::EvmArgs;
use foundry_config::{figment::Figment, Config};
use std::{collections::BTreeMap, io::Write, sync::mpsc::channel, thread};
use tracing::warn;

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(CoverageArgs, opts, evm_opts);

/// Generate coverage reports for your tests.
#[derive(Debug, Clone, Parser)]
#[clap(global_setting = AppSettings::DeriveDisplayOrder)]
pub struct CoverageArgs {
    #[clap(
        long,
        arg_enum,
        default_value = "summary",
        help = "The report type to use for coverage."
    )]
    report: CoverageReportKind,

    #[clap(flatten, next_help_heading = "TEST FILTERING")]
    filter: Filter,

    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    evm_opts: EvmArgs,

    #[clap(flatten, next_help_heading = "BUILD OPTIONS")]
    opts: CoreBuildArgs,
}

impl CoverageArgs {
    /// Returns the flattened [`CoreBuildArgs`]
    pub fn build_args(&self) -> &CoreBuildArgs {
        &self.opts
    }

    /// Returns the currently configured [Config] and the extracted [EvmOpts] from that config
    pub fn config_and_evm_opts(&self) -> eyre::Result<(Config, EvmOpts)> {
        // merge all configs
        let figment: Figment = self.into();
        let evm_opts = figment.extract()?;
        let config = Config::from_provider(figment).sanitized();

        Ok((config, evm_opts))
    }
}

impl Cmd for CoverageArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let (config, evm_opts) = self.configure()?;
        let (project, output) = self.build(&config)?;
        println!("Analysing contracts...");
        let (map, source_maps) = self.prepare(output.clone())?;

        println!("Running tests...");
        self.collect(project, output, source_maps, map, config, evm_opts)
    }
}

impl CoverageArgs {
    /// Collects and adjusts configuration.
    fn configure(&self) -> eyre::Result<(Config, EvmOpts)> {
        // Merge all configs
        let (config, mut evm_opts) = self.config_and_evm_opts()?;

        // We always want traces
        evm_opts.verbosity = 3;

        Ok((config, evm_opts))
    }

    /// Builds the project.
    fn build(&self, config: &Config) -> eyre::Result<(Project, ProjectCompileOutput)> {
        // Set up the project
        let project = {
            let mut project = config.ephemeral_no_artifacts_project()?;

            // Disable the optimizer for more accurate source maps
            project.solc_config.settings.optimizer.disable();

            project
        };

        // TODO: This does not strip file prefixes for `SourceFiles`...
        let output = ProjectCompiler::default()
            .compile(&project)?
            .with_stripped_file_prefixes(project.root());

        Ok((project, output))
    }

    /// Builds the coverage map.
    fn prepare(
        &self,
        output: ProjectCompileOutput,
    ) -> eyre::Result<(CoverageMap, BTreeMap<ArtifactId, SourceMap>)> {
        // Get sources and source maps
        let (artifacts, sources) = output.into_artifacts_with_sources();

        let source_maps: BTreeMap<ArtifactId, SourceMap> = artifacts
            .into_iter()
            .filter(|(_, artifact)| {
                // TODO: Filter out dependencies
                // Filter out tests
                !artifact
                    .get_abi()
                    .map(|abi| abi.functions().any(|f| f.name.starts_with("test")))
                    .unwrap_or_default()
            })
            .map(|(id, artifact)| (id, CompactContractBytecode::from(artifact)))
            .filter_map(|(id, artifact): (ArtifactId, CompactContractBytecode)| {
                let source_map = artifact
                    .deployed_bytecode
                    .as_ref()
                    .and_then(|bytecode| bytecode.bytecode.as_ref())
                    .and_then(|bytecode| bytecode.source_map.as_ref())
                    .and_then(|source_map| sourcemap::parse(source_map).ok())?;

                Some((id, source_map))
            })
            .collect();

        let mut map = CoverageMap::new();
        for (path, versioned_sources) in sources.0.into_iter() {
            for mut versioned_source in versioned_sources {
                let source = &mut versioned_source.source_file;
                if let Some(ast) = source.ast.take() {
                    let mut visitor = Visitor::new();
                    visitor.visit_ast(ast)?;

                    if visitor.items.is_empty() {
                        continue
                    }

                    map.add_source(path.clone(), versioned_source, visitor.items);
                }
            }
        }

        Ok((map, source_maps))
    }

    /// Runs tests, collects coverage data and generates the final report.
    fn collect(
        self,
        project: Project,
        output: ProjectCompileOutput,
        source_maps: BTreeMap<ArtifactId, SourceMap>,
        map: CoverageMap,
        config: Config,
        evm_opts: EvmOpts,
    ) -> eyre::Result<()> {
        // Setup the fuzzer
        // TODO: Add CLI Options to modify the persistence
        let cfg = proptest::test_runner::Config {
            failure_persistence: None,
            cases: config.fuzz_runs,
            max_local_rejects: config.fuzz_max_local_rejects,
            max_global_rejects: config.fuzz_max_global_rejects,
            ..Default::default()
        };
        let fuzzer = proptest::test_runner::TestRunner::new(cfg);
        let root = project.paths.root;

        // Build the contract runner
        let evm_spec = crate::utils::evm_spec(&config.evm_version);
        let mut runner = MultiContractRunnerBuilder::default()
            .fuzzer(fuzzer)
            .initial_balance(evm_opts.initial_balance)
            .evm_spec(evm_spec)
            .sender(evm_opts.sender)
            .with_fork(utils::get_fork(&evm_opts, &config.rpc_storage_caching))
            .with_coverage()
            .build(root.clone(), output, evm_opts)?;
        let (tx, rx) = channel::<(String, SuiteResult)>();

        // Set up identifier
        let local_identifier = LocalTraceIdentifier::new(&runner.known_contracts);

        // TODO: Coverage for fuzz tests
        let handle = thread::spawn(move || runner.test(&self.filter, Some(tx), false).unwrap());
        for mut result in rx.into_iter().flat_map(|(_, suite)| suite.test_results.into_values()) {
            if let Some(hit_data) = result.coverage.take() {
                let mut decoder =
                    CallTraceDecoderBuilder::new().with_events(local_identifier.events()).build();
                for (_, trace) in &mut result.traces {
                    decoder.identify(trace, &local_identifier);
                }
                // TODO: We need an ArtifactId here for the addresses
                let CallTraceDecoder { contracts, .. } = decoder;

                // ..
            }
        }

        // Reattach the thread
        let _ = handle.join();

        match self.report {
            CoverageReportKind::Summary => {
                let mut reporter = SummaryReporter::new();
                reporter.build(map);
                reporter.finalize()
            }
            // TODO: Sensible place to put the LCOV file
            CoverageReportKind::Lcov => {
                let mut reporter =
                    LcovReporter::new(std::fs::File::create(root.join("lcov.info"))?);
                reporter.build(map);
                reporter.finalize()
            }
        }
    }
}

// TODO: HTML
#[derive(Debug, Clone, ArgEnum)]
pub enum CoverageReportKind {
    Summary,
    Lcov,
}

// TODO: Move to other module
#[derive(Debug, Default, Clone)]
struct Visitor {
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
        // TODO: inlineassembly
        match node.node_type {
            // Blocks
            NodeType::Block | NodeType::UncheckedBlock => self.visit_block(node),
            // Simple statements
            NodeType::Break |
            NodeType::Continue |
            NodeType::EmitStatement |
            NodeType::PlaceholderStatement |
            NodeType::Return |
            NodeType::RevertStatement => {
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
            NodeType::ExpressionStatement => self.visit_expression(
                node.attribute("expression")
                    .ok_or_else(|| eyre::eyre!("expression statement had no expression"))?,
            ),
            // If statement
            NodeType::IfStatement => {
                // TODO: create branch
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
                    id: branch_id,
                    kind: BranchKind::True,
                    offset: true_body.src.start,
                    hits: 0,
                });
                self.visit_block_or_statement(true_body)?;

                let false_body: Option<Node> = node.attribute("falseBody");
                if let Some(false_body) = false_body {
                    self.items.push(CoverageItem::Branch {
                        id: branch_id,
                        kind: BranchKind::False,
                        offset: false_body.src.start,
                        hits: 0,
                    });
                    self.visit_block_or_statement(false_body)?;
                }

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
            NodeType::Literal => Ok(()),
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

// TODO: Move reporters to own module
/// A coverage reporter.
pub trait CoverageReporter {
    fn build(&mut self, map: CoverageMap);
    fn finalize(self) -> eyre::Result<()>;
}

/// A simple summary reporter that prints the coverage results in a table.
struct SummaryReporter {
    /// The summary table.
    table: Table,
    /// The total coverage of the entire project.
    total: CoverageSummary,
}

impl SummaryReporter {
    pub fn new() -> Self {
        let mut table = Table::new();
        table.set_header(&["File", "% Lines", "% Statements", "% Branches", "% Funcs"]);

        Self { table, total: CoverageSummary::default() }
    }

    fn add_row(&mut self, name: impl Into<Cell>, summary: CoverageSummary) {
        let mut row = Row::new();
        row.add_cell(name.into())
            .add_cell(format_cell(summary.line_hits, summary.line_count))
            .add_cell(format_cell(summary.statement_hits, summary.statement_count))
            .add_cell(format_cell(summary.branch_hits, summary.branch_count))
            .add_cell(format_cell(summary.function_hits, summary.function_count));
        self.table.add_row(row);
    }
}

impl CoverageReporter for SummaryReporter {
    fn build(&mut self, map: CoverageMap) {
        for file in map {
            let summary = file.summary();

            self.total.add(&summary);
            self.add_row(file.path.to_string_lossy(), summary);
        }
    }

    fn finalize(mut self) -> eyre::Result<()> {
        self.add_row("Total", self.total.clone());
        println!("{}", self.table);
        Ok(())
    }
}

fn format_cell(hits: usize, total: usize) -> Cell {
    let percentage = if total == 0 { 1. } else { hits as f64 / total as f64 };

    Cell::new(format!("{}% ({hits}/{total})", percentage * 100.)).fg(match percentage {
        _ if percentage < 0.5 => Color::Red,
        _ if percentage < 0.75 => Color::Yellow,
        _ => Color::Green,
    })
}

struct LcovReporter<W> {
    /// Destination buffer
    destination: W,
    /// The coverage map to write
    map: Option<CoverageMap>,
}

impl<W> LcovReporter<W> {
    pub fn new(destination: W) -> Self {
        Self { destination, map: None }
    }
}

impl<W> CoverageReporter for LcovReporter<W>
where
    W: Write,
{
    fn build(&mut self, map: CoverageMap) {
        self.map = Some(map);
    }

    fn finalize(mut self) -> eyre::Result<()> {
        let map = self.map.ok_or_else(|| eyre::eyre!("no coverage map given to reporter"))?;

        for file in map {
            let summary = file.summary();

            writeln!(self.destination, "TN:")?;
            writeln!(self.destination, "SF:{}", file.path.to_string_lossy())?;

            // TODO: Line numbers instead of byte offsets
            for item in file.items {
                match item {
                    CoverageItem::Function { name, offset, hits } => {
                        writeln!(self.destination, "FN:{offset},{name}")?;
                        writeln!(self.destination, "FNDA:{hits},{name}")?;
                    }
                    CoverageItem::Line { offset, hits } => {
                        writeln!(self.destination, "DA:{offset},{hits}")?;
                    }
                    CoverageItem::Branch { id, offset, hits, .. } => {
                        // TODO: Block ID
                        writeln!(
                            self.destination,
                            "BRDA:{offset},{id},{id},{}",
                            if hits == 0 { "-".to_string() } else { hits.to_string() }
                        )?;
                    }
                    // Statements are not in the LCOV format
                    CoverageItem::Statement { .. } => (),
                }
            }

            // Function summary
            writeln!(self.destination, "FNF:{}", summary.function_count)?;
            writeln!(self.destination, "FNH:{}", summary.function_hits)?;

            // Line summary
            writeln!(self.destination, "LF:{}", summary.line_count)?;
            writeln!(self.destination, "LH:{}", summary.line_hits)?;

            // Branch summary
            writeln!(self.destination, "BRF:{}", summary.branch_count)?;
            writeln!(self.destination, "BRH:{}", summary.branch_hits)?;

            writeln!(self.destination, "end_of_record")?;
        }

        println!("Wrote LCOV report.");

        Ok(())
    }
}
