//! Session Source
//!
//! This module contains the `SessionSource` struct, which is a minimal wrapper around
//! the REPL contract's source code. It provides simple compilation, parsing, and
//! execution helpers.

use eyre::Result;
use foundry_compilers::{
    Artifact, ProjectCompileOutput,
    artifacts::{ConfigurableContractArtifact, Source, Sources},
    project::ProjectCompiler,
    solc::Solc,
};
use foundry_config::{Config, SolcReq};
use foundry_evm::{backend::Backend, core::bytecode::InstIter, opts::EvmOpts};
use semver::Version;
use serde::{Deserialize, Serialize};
use solar::{
    ast::{ItemKind, StmtKind as AstStmtKind, yul},
    interface::{Span, diagnostics::EmittedDiagnostics},
    sema::{
        CompilerRef,
        hir::{Block, Contract, EventId, ItemId, Stmt, StmtKind as HirStmtKind},
        ty::Gcx,
    },
};
use std::{cell::OnceCell, fmt};
use walkdir::WalkDir;

/// The minimum Solidity version of the `Vm` interface.
pub const MIN_VM_VERSION: Version = Version::new(0, 6, 2);

/// Solidity source for the `Vm` interface in [forge-std](https://github.com/foundry-rs/forge-std)
static VM_SOURCE: &str = include_str!("../../../testdata/utils/Vm.sol");

/// [`SessionSource`] build output.
pub struct GeneratedOutput {
    output: ProjectCompileOutput,
}

impl fmt::Debug for GeneratedOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedOutput").finish_non_exhaustive()
    }
}

impl GeneratedOutput {
    /// Enters the solar compiler context, providing access to the HIR and `Gcx`.
    pub fn enter<R: Send>(
        &self,
        f: impl for<'a, 'b, 'gcx> FnOnce(GeneratedOutputRef<'a, 'b, 'gcx>) -> R + Send,
    ) -> R {
        self.output
            .parser()
            .solc()
            .compiler()
            .enter(|c| f(GeneratedOutputRef { output: &self.output, compiler: c }))
    }
}

/// A scoped reference to a [`GeneratedOutput`] together with an entered solar compiler.
pub struct GeneratedOutputRef<'a, 'b, 'gcx> {
    output: &'a ProjectCompileOutput,
    pub(crate) compiler: &'b CompilerRef<'gcx>,
}

impl<'gcx> GeneratedOutputRef<'_, '_, 'gcx> {
    pub fn gcx(&self) -> Gcx<'gcx> {
        self.compiler.gcx()
    }

    pub fn repl_contract(&self) -> Option<&ConfigurableContractArtifact> {
        self.output.find_first("REPL")
    }

    /// Looks up the REPL contract in the HIR.
    pub fn repl_contract_hir(&self) -> Option<&'gcx Contract<'gcx>> {
        self.gcx().hir.contracts().find(|c| c.name.as_str() == "REPL")
    }

    /// Returns the body block of the REPL `run()` function.
    pub fn run_func_body(&self) -> Block<'gcx> {
        let hir = &self.gcx().hir;
        let c = self.repl_contract_hir().expect("REPL contract not found in HIR");
        let f = c
            .functions()
            .find(|&f| hir.function(f).name.as_ref().map(|n| n.as_str()) == Some("run"))
            .expect("`run()` function not found in REPL contract");
        hir.function(f).body.expect("`run()` function does not have a body")
    }

    /// Returns the [`EventId`] of an event named `input` in the REPL contract, if any.
    pub fn get_event(&self, input: &str) -> Option<EventId> {
        let hir = &self.gcx().hir;
        let c = self.repl_contract_hir()?;
        c.items.iter().find_map(|id| {
            if let ItemId::Event(eid) = id
                && hir.event(*eid).name.as_str() == input
            {
                Some(*eid)
            } else {
                None
            }
        })
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
        //
        // Yul is not yet lowered to HIR (assembly statements appear as `StmtKind::Err`),
        // so we walk the AST of the REPL source to find a top-level `return(...)` call
        // inside any `assembly { ... }` block in `run()`.
        let last_yul_return_span: Option<Span> = self.first_yul_return_span();

        // Find the last statement within the "run()" method and get the program
        // counter via the source map.
        let Some(last_stmt) = run_body.last() else { return Ok(None) };

        // If the final statement is some type of block (unchecked or regular),
        // we need to find the final statement within that block. Otherwise, default to
        // the source loc of the final statement of the `run()` function's block.
        //
        // Inline assembly blocks (lowered to `StmtKind::Err` in HIR in the pinned solar
        // version) are handled separately via `trailing_assembly_last_stmt_span`, which
        // walks the AST to recover the last meaningful Yul statement.
        let source_stmt = match &last_stmt.kind {
            HirStmtKind::UncheckedBlock(stmts) | HirStmtKind::Block(stmts) => {
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
        // If the trailing statement is an assembly block, prefer the last meaningful
        // (non-`let`) Yul statement's span as the source location for `final_pc`.
        // See <https://github.com/foundry-rs/foundry/issues/4938>.
        //
        // Two guards are required:
        //   1. `StmtKind::Err`, assembly lowers to an error node in the current pinned solar
        //      version; this ensures we don't apply the AST fallback to properly-lowered stmts.
        //   2. `trailing_assembly_last_stmt_span` returning `Some`, verifies via the AST that the
        //      failing HIR node actually corresponds to an assembly block (not some other lowering
        //      failure), and supplies the concrete span to use.
        let mut source_span = if matches!(last_stmt.kind, HirStmtKind::Err(_))
            && let Some(span) = self.trailing_assembly_last_stmt_span()
        {
            span
        } else {
            self.stmt_span_without_semicolon(source_stmt)
        };

        // Consider yul return statement as final statement (if it's loc is lower).
        if let Some(yul_return_span) = last_yul_return_span
            && yul_return_span.hi() < source_span.lo()
        {
            source_span = yul_return_span;
        }

        // Map the source location of the final statement of the `run()` function to its
        // corresponding runtime program counter
        let result = self
            .compiler
            .sess()
            .source_map()
            .span_to_source(source_span)
            .map_err(|e| eyre::eyre!("failed to resolve span: {e:?}"))?;
        let range = result.data;
        let offset = range.start as u32;
        let length = range.len() as u32;
        trace!(%offset, %length, "find pc");
        let final_pc = contract
            .get_source_map_deployed()
            .ok_or_else(|| eyre::eyre!("No source map found for `REPL` contract"))??
            .into_iter()
            .zip(InstIter::new(deployed_bytecode_bytes).with_pc().map(|(pc, _)| pc))
            .filter(|(s, _)| s.offset() == offset && s.length() == length)
            .map(|(_, pc)| pc)
            .max();
        trace!(?final_pc);
        Ok(final_pc)
    }

    /// Statements' ranges in the solc source map do not include the semicolon.
    fn stmt_span_without_semicolon(&self, stmt: &Stmt<'_>) -> Span {
        match stmt.kind {
            HirStmtKind::DeclSingle(id) => {
                let decl = self.gcx().hir.variable(id);
                if let Some(expr) = decl.initializer {
                    stmt.span.with_hi(expr.span.hi())
                } else {
                    stmt.span
                }
            }
            HirStmtKind::DeclMulti(_, expr) => stmt.span.with_hi(expr.span.hi()),
            HirStmtKind::Expr(expr) => expr.span,
            _ => stmt.span,
        }
    }

    /// Returns the AST `run()` body of the REPL contract, if any.
    ///
    /// Yul/assembly is not yet lowered to HIR in the pinned solar version, so we
    /// keep around the AST to be able to inspect inline assembly blocks.
    fn repl_run_ast_body(&self) -> Option<&'gcx solar::ast::Block<'gcx>> {
        let contract = self.repl_contract_hir()?;
        let source = self.gcx().sources.get(contract.source)?;
        let ast = source.ast.as_ref()?;

        let contract_ast = ast.items.iter().find_map(|i| match &i.kind {
            ItemKind::Contract(c) if c.name.as_str() == "REPL" => Some(c),
            _ => None,
        })?;
        contract_ast.body.iter().find_map(|i| match &i.kind {
            ItemKind::Function(f) if f.header.name.is_some_and(|n| n.as_str() == "run") => {
                f.body.as_ref()
            }
            _ => None,
        })
    }

    /// Returns the span of the first top-level `return(...)` call inside any
    /// `assembly { ... }` block in the REPL `run()` function, if any.
    fn first_yul_return_span(&self) -> Option<Span> {
        let run_body = self.repl_run_ast_body()?;
        for stmt in run_body.stmts.iter() {
            let AstStmtKind::Assembly(asm) = &stmt.kind else { continue };
            for ystmt in asm.block.stmts.iter() {
                if let yul::StmtKind::Expr(e) = &ystmt.kind
                    && let yul::ExprKind::Call(call) = &e.kind
                    && call.name.as_str() == "return"
                {
                    return Some(ystmt.span);
                }
            }
        }
        None
    }

    /// If the last statement of the REPL `run()` function is an `assembly { ... }` block,
    /// returns the span of its last non-`let` (i.e. non-VarDecl) Yul statement.
    ///
    /// This mirrors the legacy behavior used to pick a meaningful end-of-function PC when
    /// the trailing statement is inline assembly.
    fn trailing_assembly_last_stmt_span(&self) -> Option<Span> {
        let run_body = self.repl_run_ast_body()?;
        let AstStmtKind::Assembly(asm) = &run_body.stmts.last()?.kind else { return None };
        asm.block
            .stmts
            .iter()
            .rev()
            .find(|s| !matches!(s.kind, yul::StmtKind::VarDecl(_, _)))
            .map(|s| s.span)
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
    /// Enable viaIR with minimum optimization
    ///
    /// This can fix most of the "stack too deep" errors while resulting a
    /// relatively accurate source map.
    pub ir_minimum: bool,
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
            info!(%version, minimum=%MIN_VM_VERSION, "Disabling VM injection");
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
            debug!("{errors}");
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
        let output = GeneratedOutput { output };
        Ok(self.output.get_or_init(|| output))
    }

    /// Compiles the source.
    #[cold]
    fn compile(&self) -> Result<ProjectCompileOutput> {
        let sources = self.get_sources();

        let mut project = self.config.foundry_config.ephemeral_project()?;
        self.config.foundry_config.disable_optimizations(&mut project, self.config.ir_minimum);
        let mut output = ProjectCompiler::with_sources(&project, sources)?.compile()?;

        if output.has_compiler_errors() {
            eyre::bail!("{output}");
        }

        // Drive HIR lowering and analysis so that subsequent `enter` queries can use them.
        output.parser_mut().solc_mut().compiler_mut().enter_mut(|c| {
            let _ = c.lower_asts();
            let _ = c.analysis();
        });

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
