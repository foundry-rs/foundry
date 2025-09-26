//! Session Source
//!
//! This module contains the `SessionSource` struct, which is a minimal wrapper around
//! the REPL contract's source code. It provides simple compilation, parsing, and
//! execution helpers.

use eyre::Result;
use forge_doc::solang_ext::{CodeLocationExt, SafeUnwrap};
use foundry_common::fs;
use foundry_compilers::{
    Artifact, ProjectCompileOutput,
    artifacts::{ConfigurableContractArtifact, Source, Sources},
    project::ProjectCompiler,
    solc::Solc,
};
use foundry_config::{Config, SolcReq};
use foundry_evm::{backend::Backend, opts::EvmOpts};
use semver::Version;
use serde::{Deserialize, Serialize};
use solang_parser::pt;
use solar::interface::diagnostics::EmittedDiagnostics;
use std::{cell::OnceCell, collections::HashMap, fmt, path::PathBuf};
use walkdir::WalkDir;

/// The minimum Solidity version of the `Vm` interface.
pub const MIN_VM_VERSION: Version = Version::new(0, 6, 2);

/// Solidity source for the `Vm` interface in [forge-std](https://github.com/foundry-rs/forge-std)
static VM_SOURCE: &str = include_str!("../../../testdata/cheats/Vm.sol");

/// [`SessionSource`] build output.
pub struct GeneratedOutput {
    output: ProjectCompileOutput,
    pub(crate) intermediate: IntermediateOutput,
}

pub struct GeneratedOutputRef<'a> {
    output: &'a ProjectCompileOutput,
    // compiler: &'b solar::sema::CompilerRef<'c>,
    pub(crate) intermediate: &'a IntermediateOutput,
}

/// Intermediate output for the compiled [SessionSource]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntermediateOutput {
    /// All expressions within the REPL contract's run function and top level scope.
    pub repl_contract_expressions: HashMap<String, pt::Expression>,
    /// Intermediate contracts
    pub intermediate_contracts: IntermediateContracts,
}

/// A refined intermediate parse tree for a contract that enables easy lookups
/// of definitions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntermediateContract {
    /// All function definitions within the contract
    pub function_definitions: HashMap<String, Box<pt::FunctionDefinition>>,
    /// All event definitions within the contract
    pub event_definitions: HashMap<String, Box<pt::EventDefinition>>,
    /// All struct definitions within the contract
    pub struct_definitions: HashMap<String, Box<pt::StructDefinition>>,
    /// All variable definitions within the top level scope of the contract
    pub variable_definitions: HashMap<String, Box<pt::VariableDefinition>>,
}

/// A defined type for a map of contract names to [IntermediateContract]s
type IntermediateContracts = HashMap<String, IntermediateContract>;

impl fmt::Debug for GeneratedOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedOutput").finish_non_exhaustive()
    }
}

impl GeneratedOutput {
    pub fn enter<T: Send>(&self, f: impl FnOnce(GeneratedOutputRef<'_>) -> T + Send) -> T {
        // TODO(dani): once intermediate is removed
        // self.output
        //     .parser()
        //     .solc()
        //     .compiler()
        //     .enter(|compiler| f(GeneratedOutputRef { output: &self.output, compiler }))
        f(GeneratedOutputRef { output: &self.output, intermediate: &self.intermediate })
    }
}

impl GeneratedOutputRef<'_> {
    pub fn repl_contract(&self) -> Option<&ConfigurableContractArtifact> {
        self.output.find_first("REPL")
    }
}

impl std::ops::Deref for GeneratedOutput {
    type Target = IntermediateOutput;
    fn deref(&self) -> &Self::Target {
        &self.intermediate
    }
}
impl std::ops::Deref for GeneratedOutputRef<'_> {
    type Target = IntermediateOutput;
    fn deref(&self) -> &Self::Target {
        self.intermediate
    }
}

impl IntermediateOutput {
    pub fn get_event(&self, input: &str) -> Option<&pt::EventDefinition> {
        self.intermediate_contracts
            .get("REPL")
            .and_then(|contract| contract.event_definitions.get(input).map(std::ops::Deref::deref))
    }

    pub fn final_pc(&self, contract: &ConfigurableContractArtifact) -> Result<Option<usize>> {
        let deployed_bytecode = contract
            .get_deployed_bytecode()
            .ok_or_else(|| eyre::eyre!("No deployed bytecode found for `REPL` contract"))?;
        let deployed_bytecode_bytes = deployed_bytecode
            .bytes()
            .ok_or_else(|| eyre::eyre!("No deployed bytecode found for `REPL` contract"))?;

        let run_func_statements = self.run_func_body()?;

        // Record loc of first yul block return statement (if any).
        // This is used to decide which is the final statement within the `run()` method.
        // see <https://github.com/foundry-rs/foundry/issues/4617>.
        let last_yul_return = run_func_statements.iter().find_map(|statement| {
            if let pt::Statement::Assembly { loc: _, dialect: _, flags: _, block } = statement
                && let Some(statement) = block.statements.last()
                && let pt::YulStatement::FunctionCall(yul_call) = statement
                && yul_call.id.name == "return"
            {
                return Some(statement.loc());
            }
            None
        });

        // Find the last statement within the "run()" method and get the program
        // counter via the source map.
        let Some(final_statement) = run_func_statements.last() else { return Ok(None) };

        // If the final statement is some type of block (assembly, unchecked, or regular),
        // we need to find the final statement within that block. Otherwise, default to
        // the source loc of the final statement of the `run()` function's block.
        //
        // There is some code duplication within the arms due to the difference between
        // the [pt::Statement] type and the [pt::YulStatement] types.
        let mut source_loc = match final_statement {
            pt::Statement::Assembly { loc: _, dialect: _, flags: _, block } => {
                // Select last non variable declaration statement, see <https://github.com/foundry-rs/foundry/issues/4938>.
                let last_statement = block.statements.iter().rev().find(|statement| {
                    !matches!(statement, pt::YulStatement::VariableDeclaration(_, _, _))
                });
                if let Some(statement) = last_statement {
                    statement.loc()
                } else {
                    // In the case where the block is empty, attempt to grab the statement
                    // before the asm block. Because we use saturating sub to get the second
                    // to last index, this can always be safely unwrapped.
                    run_func_statements
                        .get(run_func_statements.len().saturating_sub(2))
                        .unwrap()
                        .loc()
                }
            }
            pt::Statement::Block { loc: _, unchecked: _, statements } => {
                if let Some(statement) = statements.last() {
                    statement.loc()
                } else {
                    // In the case where the block is empty, attempt to grab the statement
                    // before the block. Because we use saturating sub to get the second to
                    // last index, this can always be safely unwrapped.
                    run_func_statements
                        .get(run_func_statements.len().saturating_sub(2))
                        .unwrap()
                        .loc()
                }
            }
            _ => final_statement.loc(),
        };

        // Consider yul return statement as final statement (if it's loc is lower) .
        if let Some(yul_return) = last_yul_return
            && yul_return.end() < source_loc.start()
        {
            source_loc = yul_return;
        }

        // Map the source location of the final statement of the `run()` function to its
        // corresponding runtime program counter
        let final_pc = {
            let offset = source_loc.start() as u32;
            let length = (source_loc.end() - source_loc.start()) as u32;
            contract
                .get_source_map_deployed()
                .unwrap()
                .unwrap()
                .into_iter()
                .zip(InstructionIter::new(deployed_bytecode_bytes))
                .filter(|(s, _)| s.offset() == offset && s.length() == length)
                .map(|(_, i)| i.pc)
                .max()
                .unwrap_or_default()
        };
        Ok(Some(final_pc))
    }

    pub fn run_func_body(&self) -> Result<&Vec<pt::Statement>> {
        match self
            .intermediate_contracts
            .get("REPL")
            .ok_or_else(|| eyre::eyre!("Could not find REPL intermediate contract!"))?
            .function_definitions
            .get("run")
            .ok_or_else(|| eyre::eyre!("Could not find run function definition in REPL contract!"))?
            .body
            .as_ref()
            .ok_or_else(|| eyre::eyre!("Could not find run function body!"))?
        {
            pt::Statement::Block { statements, .. } => Ok(statements),
            _ => eyre::bail!("Could not find statements within run function body!"),
        }
    }
}

// TODO(dani): further migration blocked on upstream work
#[cfg(false)]
impl<'gcx> GeneratedOutputRef<'_, '_, 'gcx> {
    pub fn gcx(&self) -> Gcx<'gcx> {
        self.compiler.gcx()
    }

    pub fn repl_contract(&self) -> Option<&ConfigurableContractArtifact> {
        self.output.find_first("REPL")
    }

    pub fn get_event(&self, input: &str) -> Option<hir::EventId> {
        self.gcx().hir.events_enumerated().find(|(_, e)| e.name.as_str() == input).map(|(id, _)| id)
    }

    pub fn final_pc(&self, contract: &ConfigurableContractArtifact) -> Result<Option<usize>> {
        let deployed_bytecode = contract
            .get_deployed_bytecode()
            .ok_or_else(|| eyre::eyre!("No deployed bytecode found for `REPL` contract"))?;
        let deployed_bytecode_bytes = deployed_bytecode
            .bytes()
            .ok_or_else(|| eyre::eyre!("No deployed bytecode found for `REPL` contract"))?;

        // Fetch the run function's body statement
        let run_body = self.run_func_body();

        // Record loc of first yul block return statement (if any).
        // This is used to decide which is the final statement within the `run()` method.
        // see <https://github.com/foundry-rs/foundry/issues/4617>.
        let last_yul_return_span: Option<Span> = run_body.iter().find_map(|stmt| {
            // TODO(dani): Yul is not yet lowered to HIR.
            let _ = stmt;
            /*
            if let hir::StmtKind::Assembly { block, .. } = stmt {
                if let Some(stmt) = block.last() {
                    if let pt::YulStatement::FunctionCall(yul_call) = stmt {
                        if yul_call.id.name == "return" {
                            return Some(stmt.loc())
                        }
                    }
                }
            }
            */
            None
        });

        // Find the last statement within the "run()" method and get the program
        // counter via the source map.
        let Some(last_stmt) = run_body.last() else { return Ok(None) };

        // If the final statement is some type of block (assembly, unchecked, or regular),
        // we need to find the final statement within that block. Otherwise, default to
        // the source loc of the final statement of the `run()` function's block.
        //
        // There is some code duplication within the arms due to the difference between
        // the [pt::Statement] type and the [pt::YulStatement] types.
        let source_stmt = match &last_stmt.kind {
            // TODO(dani): Yul is not yet lowered to HIR.
            /*
            pt::Statement::Assembly { loc: _, dialect: _, flags: _, block } => {
                // Select last non variable declaration statement, see <https://github.com/foundry-rs/foundry/issues/4938>.
                let last_statement = block.statements.iter().rev().find(|statement| {
                    !matches!(statement, pt::YulStatement::VariableDeclaration(_, _, _))
                });
                if let Some(stmt) = last_statement {
                    stmt
                } else {
                    // In the case where the block is empty, attempt to grab the statement
                    // before the block. Because we use saturating sub to get the second to
                    // last index, this can always be safely unwrapped.
                    &run_body[run_body.len().saturating_sub(2)]
                }
            }
            */
            hir::StmtKind::UncheckedBlock(stmts) | hir::StmtKind::Block(stmts) => {
                if let Some(stmt) = stmts.last() {
                    stmt
                } else {
                    // In the case where the block is empty, attempt to grab the statement
                    // before the block. Because we use saturating sub to get the second to
                    // last index, this can always be safely unwrapped.
                    &run_body[run_body.len().saturating_sub(2)]
                }
            }
            _ => last_stmt,
        };
        let mut source_span = self.stmt_span_without_semicolon(source_stmt);

        // Consider yul return statement as final statement (if it's loc is lower) .
        if let Some(yul_return_span) = last_yul_return_span
            && yul_return_span.hi() < source_span.lo()
        {
            source_span = yul_return_span;
        }

        // Map the source location of the final statement of the `run()` function to its
        // corresponding runtime program counter
        let (_sf, range) = self.compiler.sess().source_map().span_to_source(source_span).unwrap();
        dbg!(source_span, &range, &_sf.src[range.clone()]);
        let offset = range.start as u32;
        let length = range.len() as u32;
        let final_pc = deployed_bytecode
            .source_map()
            .ok_or_else(|| eyre::eyre!("No source map found for `REPL` contract"))??
            .into_iter()
            .zip(InstructionIter::new(deployed_bytecode_bytes))
            .filter(|(s, _)| s.offset() == offset && s.length() == length)
            .map(|(_, i)| i.pc)
            .max()
            .unwrap_or_default();
        Ok(Some(final_pc))
    }

    /// Statements' ranges in the solc source map do not include the semicolon.
    fn stmt_span_without_semicolon(&self, stmt: &hir::Stmt<'_>) -> Span {
        match stmt.kind {
            hir::StmtKind::DeclSingle(id) => {
                let decl = self.gcx().hir.variable(id);
                if let Some(expr) = decl.initializer {
                    stmt.span.with_hi(expr.span.hi())
                } else {
                    stmt.span
                }
            }
            hir::StmtKind::DeclMulti(_, expr) => stmt.span.with_hi(expr.span.hi()),
            hir::StmtKind::Expr(expr) => expr.span,
            _ => stmt.span,
        }
    }

    fn run_func_body(&self) -> hir::Block<'_> {
        let c = self.repl_contract_hir().expect("REPL contract not found in HIR");
        let f = c
            .functions()
            .find(|&f| self.gcx().hir.function(f).name.as_ref().map(|n| n.as_str()) == Some("run"))
            .expect("`run()` function not found in REPL contract");
        self.gcx().hir.function(f).body.expect("`run()` function does not have a body")
    }

    fn repl_contract_hir(&self) -> Option<&hir::Contract<'_>> {
        self.gcx().hir.contracts().find(|c| c.name.as_str() == "REPL")
    }
}

/// Configuration for the [SessionSource]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionSourceConfig {
    /// Foundry configuration
    pub foundry_config: Config,
    /// EVM Options
    pub evm_opts: EvmOpts,
    /// Disable the default `Vm` import.
    pub no_vm: bool,
    /// In-memory REVM db for the session's runner.
    #[serde(skip)]
    pub backend: Option<Backend>,
    /// Optionally enable traces for the REPL contract execution
    pub traces: bool,
    /// Optionally set calldata for the REPL contract execution
    pub calldata: Option<Vec<u8>>,
}

impl SessionSourceConfig {
    /// Detect the solc version to know if VM can be injected.
    pub fn detect_solc(&mut self) -> Result<()> {
        if self.foundry_config.solc.is_none() {
            let version = Solc::ensure_installed(&"*".parse().unwrap())?;
            self.foundry_config.solc = Some(SolcReq::Version(version));
        }
        if !self.no_vm
            && let Some(version) = self.foundry_config.solc_version()
            && version < MIN_VM_VERSION
        {
            tracing::info!(%version, minimum=%MIN_VM_VERSION, "Disabling VM injection");
            self.no_vm = true;
        }
        Ok(())
    }
}

/// REPL Session Source wrapper
///
/// Heavily based on soli's [`ConstructedSource`](https://github.com/jpopesculian/soli/blob/master/src/main.rs#L166)
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionSource {
    /// The file name
    pub file_name: String,
    /// The contract name
    pub contract_name: String,

    /// Session Source configuration
    pub config: SessionSourceConfig,

    /// Global level Solidity code.
    ///
    /// Above and outside all contract declarations, in the global context.
    pub global_code: String,
    /// Top level Solidity code.
    ///
    /// Within the contract declaration, but outside of the `run()` function.
    pub contract_code: String,
    /// The code to be executed in the `run()` function.
    pub run_code: String,

    /// Cached VM source code.
    #[serde(skip, default = "vm_source")]
    vm_source: Source,
    /// The generated output
    #[serde(skip)]
    output: OnceCell<GeneratedOutput>,
}

fn vm_source() -> Source {
    Source::new(VM_SOURCE)
}

impl Clone for SessionSource {
    fn clone(&self) -> Self {
        Self {
            file_name: self.file_name.clone(),
            contract_name: self.contract_name.clone(),
            global_code: self.global_code.clone(),
            contract_code: self.contract_code.clone(),
            run_code: self.run_code.clone(),
            config: self.config.clone(),
            vm_source: self.vm_source.clone(),
            output: Default::default(),
        }
    }
}

impl SessionSource {
    /// Creates a new source given a solidity compiler version
    ///
    /// # Panics
    ///
    /// If no Solc binary is set, cannot be found or the `--version` command fails
    ///
    /// ### Takes
    ///
    /// - An instance of [Solc]
    /// - An instance of [SessionSourceConfig]
    ///
    /// ### Returns
    ///
    /// A new instance of [SessionSource]
    pub fn new(mut config: SessionSourceConfig) -> Result<Self> {
        config.detect_solc()?;
        Ok(Self {
            file_name: "ReplContract.sol".to_string(),
            contract_name: "REPL".to_string(),
            config,
            global_code: Default::default(),
            contract_code: Default::default(),
            run_code: Default::default(),
            vm_source: vm_source(),
            output: Default::default(),
        })
    }

    /// Clones the [SessionSource] and appends a new line of code.
    ///
    /// Returns `true` if the new line was added to `run()`.
    pub fn clone_with_new_line(&self, mut content: String) -> Result<(Self, bool)> {
        if let Some((new_source, fragment)) = self
            .parse_fragment(&content)
            .or_else(|| {
                content.push(';');
                self.parse_fragment(&content)
            })
            .or_else(|| {
                content = content.trim_end().trim_end_matches(';').to_string();
                self.parse_fragment(&content)
            })
        {
            Ok((new_source, matches!(fragment, ParseTreeFragment::Function)))
        } else {
            eyre::bail!("\"{}\"", content.trim());
        }
    }

    /// Parses a fragment of Solidity code in memory and assigns it a scope within the
    /// [`SessionSource`].
    fn parse_fragment(&self, buffer: &str) -> Option<(Self, ParseTreeFragment)> {
        #[track_caller]
        fn debug_errors(errors: &EmittedDiagnostics) {
            tracing::debug!("{errors}");
        }

        let mut this = self.clone();
        match this.add_run_code(buffer).parse() {
            Ok(()) => return Some((this, ParseTreeFragment::Function)),
            Err(e) => debug_errors(&e),
        }
        this = self.clone();
        match this.add_contract_code(buffer).parse() {
            Ok(()) => return Some((this, ParseTreeFragment::Contract)),
            Err(e) => debug_errors(&e),
        }
        this = self.clone();
        match this.add_global_code(buffer).parse() {
            Ok(()) => return Some((this, ParseTreeFragment::Source)),
            Err(e) => debug_errors(&e),
        }
        None
    }

    /// Append global-level code to the source.
    pub fn add_global_code(&mut self, content: &str) -> &mut Self {
        self.global_code.push_str(content.trim());
        self.global_code.push('\n');
        self.clear_output();
        self
    }

    /// Append contract-level code to the source.
    pub fn add_contract_code(&mut self, content: &str) -> &mut Self {
        self.contract_code.push_str(content.trim());
        self.contract_code.push('\n');
        self.clear_output();
        self
    }

    /// Append code to the `run()` function of the REPL contract.
    pub fn add_run_code(&mut self, content: &str) -> &mut Self {
        self.run_code.push_str(content.trim());
        self.run_code.push('\n');
        self.clear_output();
        self
    }

    /// Clears all source code.
    pub fn clear(&mut self) {
        String::clear(&mut self.global_code);
        String::clear(&mut self.contract_code);
        String::clear(&mut self.run_code);
        self.clear_output();
    }

    /// Clear the global-level code .
    pub fn clear_global(&mut self) -> &mut Self {
        String::clear(&mut self.global_code);
        self.clear_output();
        self
    }

    /// Clear the contract-level code .
    pub fn clear_contract(&mut self) -> &mut Self {
        String::clear(&mut self.contract_code);
        self.clear_output();
        self
    }

    /// Clear the `run()` function code.
    pub fn clear_run(&mut self) -> &mut Self {
        String::clear(&mut self.run_code);
        self.clear_output();
        self
    }

    fn clear_output(&mut self) {
        self.output.take();
    }

    /// Compiles the source if necessary.
    pub fn build(&self) -> Result<&GeneratedOutput> {
        // TODO: mimics `get_or_try_init`
        if let Some(output) = self.output.get() {
            return Ok(output);
        }
        let output = self.compile()?;
        let intermediate = self.generate_intermediate_output()?;
        let output = GeneratedOutput { output, intermediate };
        Ok(self.output.get_or_init(|| output))
    }

    /// Compiles the source.
    #[cold]
    fn compile(&self) -> Result<ProjectCompileOutput> {
        let sources = self.get_sources();

        let project = self.config.foundry_config.ephemeral_project()?;
        let mut output = ProjectCompiler::with_sources(&project, sources)?.compile()?;

        if output.has_compiler_errors() {
            eyre::bail!("{output}");
        }

        // TODO(dani): re-enable
        if cfg!(false) {
            output.parser_mut().solc_mut().compiler_mut().enter_mut(|c| {
                let _ = c.lower_asts();
            });
        }

        Ok(output)
    }

    fn get_sources(&self) -> Sources {
        let mut sources = Sources::new();

        let src = self.to_repl_source();
        sources.insert(self.file_name.clone().into(), Source::new(src));

        // Include Vm.sol if forge-std remapping is not available.
        if !self.config.no_vm
            && !self
                .config
                .foundry_config
                .get_all_remappings()
                .any(|r| r.name.starts_with("forge-std"))
        {
            sources.insert("forge-std/Vm.sol".into(), self.vm_source.clone());
        }

        sources
    }

    /// Generate intermediate contracts for all contract definitions in the compilation source.
    ///
    /// ### Returns
    ///
    /// Optionally, a map of contract names to a vec of [IntermediateContract]s.
    pub fn generate_intermediate_contracts(&self) -> Result<HashMap<String, IntermediateContract>> {
        let mut res_map = HashMap::default();
        let parsed_map = self.get_sources();
        for source in parsed_map.values() {
            Self::get_intermediate_contract(&source.content, &mut res_map);
        }
        Ok(res_map)
    }

    /// Generate intermediate output for the REPL contract
    pub fn generate_intermediate_output(&self) -> Result<IntermediateOutput> {
        // Parse generate intermediate contracts
        let intermediate_contracts = self.generate_intermediate_contracts()?;

        // Construct variable definitions
        let variable_definitions = intermediate_contracts
            .get("REPL")
            .ok_or_else(|| eyre::eyre!("Could not find intermediate REPL contract!"))?
            .variable_definitions
            .clone()
            .into_iter()
            .map(|(k, v)| (k, v.ty))
            .collect::<HashMap<String, pt::Expression>>();
        // Construct intermediate output
        let mut intermediate_output = IntermediateOutput {
            repl_contract_expressions: variable_definitions,
            intermediate_contracts,
        };

        // Add all statements within the run function to the repl_contract_expressions map
        for (key, val) in intermediate_output
            .run_func_body()?
            .clone()
            .iter()
            .flat_map(Self::get_statement_definitions)
        {
            intermediate_output.repl_contract_expressions.insert(key, val);
        }

        Ok(intermediate_output)
    }

    /// Construct the source as a valid Forge script.
    pub fn to_script_source(&self) -> String {
        let Self {
            contract_name,
            global_code,
            contract_code: top_level_code,
            run_code,
            config,
            ..
        } = self;

        let script_import =
            if !config.no_vm { "import {Script} from \"forge-std/Script.sol\";\n" } else { "" };

        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0;

{script_import}
{global_code}

contract {contract_name} is Script {{
    {top_level_code}

    /// @notice Script entry point
    function run() public {{
        {run_code}
    }}
}}"#,
        )
    }

    /// Construct the REPL source.
    pub fn to_repl_source(&self) -> String {
        let Self {
            contract_name,
            global_code,
            contract_code: top_level_code,
            run_code,
            config,
            ..
        } = self;
        let (mut vm_import, mut vm_constant) = (String::new(), String::new());
        // Check if there's any `forge-std` remapping and determine proper path to it by
        // searching remapping path.
        if !config.no_vm
            && let Some(remapping) = config
                .foundry_config
                .remappings
                .iter()
                .find(|remapping| remapping.name == "forge-std/")
            && let Some(vm_path) = WalkDir::new(&remapping.path.path)
                .into_iter()
                .filter_map(|e| e.ok())
                .find(|e| e.file_name() == "Vm.sol")
        {
            vm_import = format!("import {{Vm}} from \"{}\";\n", vm_path.path().display());
            vm_constant = "Vm internal constant vm = Vm(address(uint160(uint256(keccak256(\"hevm cheat code\")))));\n".to_string();
        }

        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0;

{vm_import}
{global_code}

contract {contract_name} {{
    {vm_constant}
    {top_level_code}

    /// @notice REPL contract entry point
    function run() public {{
        {run_code}
    }}
}}"#,
        )
    }

    /// Parse the current source in memory using Solar.
    pub(crate) fn parse(&self) -> Result<(), EmittedDiagnostics> {
        let sess =
            solar::interface::Session::builder().with_buffer_emitter(Default::default()).build();
        let _ = sess.enter_sequential(|| -> solar::interface::Result<()> {
            let arena = solar::ast::Arena::new();
            let filename = self.file_name.clone().into();
            let src = self.to_repl_source();
            let mut parser = solar::parse::Parser::from_source_code(&sess, &arena, filename, src)?;
            let _ast = parser.parse_file().map_err(|e| e.emit())?;
            Ok(())
        });
        sess.dcx.emitted_errors().unwrap()
    }

    /// Gets the [IntermediateContract] for a Solidity source string and inserts it into the
    /// passed `res_map`. In addition, recurses on any imported files as well.
    ///
    /// ### Takes
    /// - `content` - A Solidity source string
    /// - `res_map` - A mutable reference to a map of contract names to [IntermediateContract]s
    pub fn get_intermediate_contract(
        content: &str,
        res_map: &mut HashMap<String, IntermediateContract>,
    ) {
        if let Ok((pt::SourceUnit(source_unit_parts), _)) = solang_parser::parse(content, 0) {
            let func_defs = source_unit_parts
                .into_iter()
                .filter_map(|sup| match sup {
                    pt::SourceUnitPart::ImportDirective(i) => match i {
                        pt::Import::Plain(s, _)
                        | pt::Import::Rename(s, _, _)
                        | pt::Import::GlobalSymbol(s, _, _) => {
                            let s = match s {
                                pt::ImportPath::Filename(s) => s.string,
                                pt::ImportPath::Path(p) => p.to_string(),
                            };
                            let path = PathBuf::from(s);

                            match fs::read_to_string(path) {
                                Ok(source) => {
                                    Self::get_intermediate_contract(&source, res_map);
                                    None
                                }
                                Err(_) => None,
                            }
                        }
                    },
                    pt::SourceUnitPart::ContractDefinition(cd) => {
                        let mut intermediate = IntermediateContract::default();

                        cd.parts.into_iter().for_each(|part| match part {
                            pt::ContractPart::FunctionDefinition(def) => {
                                // Only match normal function definitions here.
                                if matches!(def.ty, pt::FunctionTy::Function) {
                                    intermediate
                                        .function_definitions
                                        .insert(def.name.clone().unwrap().name, def);
                                }
                            }
                            pt::ContractPart::EventDefinition(def) => {
                                let event_name = def.name.safe_unwrap().name.clone();
                                intermediate.event_definitions.insert(event_name, def);
                            }
                            pt::ContractPart::StructDefinition(def) => {
                                let struct_name = def.name.safe_unwrap().name.clone();
                                intermediate.struct_definitions.insert(struct_name, def);
                            }
                            pt::ContractPart::VariableDefinition(def) => {
                                let var_name = def.name.safe_unwrap().name.clone();
                                intermediate.variable_definitions.insert(var_name, def);
                            }
                            _ => {}
                        });
                        Some((cd.name.safe_unwrap().name.clone(), intermediate))
                    }
                    _ => None,
                })
                .collect::<HashMap<String, IntermediateContract>>();
            res_map.extend(func_defs);
        }
    }

    /// Helper to deconstruct a statement
    ///
    /// ### Takes
    ///
    /// A reference to a [pt::Statement]
    ///
    /// ### Returns
    ///
    /// A vector containing tuples of the inner expressions' names, types, and storage locations.
    pub fn get_statement_definitions(statement: &pt::Statement) -> Vec<(String, pt::Expression)> {
        match statement {
            pt::Statement::VariableDefinition(_, def, _) => {
                vec![(def.name.safe_unwrap().name.clone(), def.ty.clone())]
            }
            pt::Statement::Expression(_, pt::Expression::Assign(_, left, _)) => {
                if let pt::Expression::List(_, list) = left.as_ref() {
                    list.iter()
                        .filter_map(|(_, param)| {
                            param.as_ref().and_then(|param| {
                                param
                                    .name
                                    .as_ref()
                                    .map(|name| (name.name.clone(), param.ty.clone()))
                            })
                        })
                        .collect()
                } else {
                    Vec::default()
                }
            }
            _ => Vec::default(),
        }
    }
}

/// A Parse Tree Fragment
///
/// Used to determine whether an input will go to the "run()" function,
/// the top level of the contract, or in global scope.
#[derive(Debug)]
enum ParseTreeFragment {
    /// Code for the global scope
    Source,
    /// Code for the top level of the contract
    Contract,
    /// Code for the "run()" function
    Function,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Instruction {
    pub pc: usize,
    pub opcode: u8,
    pub data: [u8; 32],
    pub data_len: u8,
}

struct InstructionIter<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> InstructionIter<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl Iterator for InstructionIter<'_> {
    type Item = Instruction;

    fn next(&mut self) -> Option<Self::Item> {
        let pc = self.offset;
        self.offset += 1;
        let opcode = *self.bytes.get(pc)?;
        let (data, data_len) = if matches!(opcode, 0x60..=0x7F) {
            let mut data = [0; 32];
            let data_len = (opcode - 0x60 + 1) as usize;
            data[..data_len].copy_from_slice(&self.bytes[self.offset..self.offset + data_len]);
            self.offset += data_len;
            (data, data_len as u8)
        } else {
            ([0; 32], 0)
        };
        Some(Instruction { pc, opcode, data, data_len })
    }
}
