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
use foundry_config::{
    Config, EtherscanConfigs, FoundryHardfork, RpcEndpoints, SolcReq, cache::CachedEndpoints,
};
use foundry_evm::{
    backend::Backend,
    core::{bytecode::InstIter, evm::FoundryEvmNetwork},
    executors::ExecutorBuilder,
    fork::ResolvedFork,
    opts::EvmOpts,
};
use foundry_evm_networks::NetworkConfigs;
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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

/// In-memory backend and the exact fork identity from which it was constructed.
#[derive(Clone, Debug)]
pub(crate) struct CachedBackend<FEN: FoundryEvmNetwork> {
    pub(crate) backend: Backend<FEN>,
    pub(crate) resolved_fork: Option<ResolvedFork>,
}

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
        // Walk the AST of the REPL source to find a top-level `return(...)` call
        // inside any `assembly { ... }` block in `run()`. This lets us pick the
        // meaningful Yul return span even when HIR represents the block coarsely.
        let last_yul_return_span: Option<Span> = self.first_yul_return_span();

        // Find the last statement within the "run()" method and get the program
        // counter via the source map.
        let Some(last_stmt) = run_body.last() else { return Ok(None) };

        // If the final statement is some type of block (unchecked or regular),
        // we need to find the final statement within that block. Otherwise, default to
        // the source loc of the final statement of the `run()` function's block.
        //
        // Inline assembly blocks are handled separately via
        // `trailing_assembly_last_stmt_span`, which walks the AST to recover the last
        // meaningful Yul statement.
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
        // `trailing_assembly_last_stmt_span` verifies via the AST that the HIR node
        // corresponds to an assembly block and supplies the concrete Yul span to use.
        let mut source_span =
            if matches!(last_stmt.kind, HirStmtKind::AssemblyBlock(_) | HirStmtKind::Err(_))
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
    /// Returns the AST `run()` body so inline assembly blocks can be inspected at
    /// Yul-statement granularity.
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
///
/// Serialization is derived, but credential-bearing fields are always written through sanitizing
/// getters, so every serialization path omits RPC and explorer credentials.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(remote = "Self", bound = "")]
pub struct SessionSourceConfig<FEN: FoundryEvmNetwork> {
    /// Foundry configuration
    #[serde(getter = "Self::persisted_foundry_config")]
    pub foundry_config: Config,
    /// EVM Options
    #[serde(getter = "Self::persisted_evm_opts")]
    pub evm_opts: EvmOpts,
    /// Executor tooling selected by the concrete network dispatch.
    #[serde(skip)]
    pub executor_builder: ExecutorBuilder<FEN>,
    /// Network family to restore when leaving fork mode.
    #[serde(default)]
    pub local_networks: Option<NetworkConfigs>,
    /// Chain ID to restore when leaving fork mode.
    #[serde(default)]
    pub local_chain_id: Option<u64>,
    /// Whether the saved fork network was inferred from its endpoint.
    #[serde(default)]
    pub fork_network_is_inferred: bool,
    /// Whether the saved chain ID was inferred from its endpoint.
    #[serde(default)]
    pub fork_chain_id_is_inferred: bool,
    /// Exact network hardfork selected for the latest execution.
    #[serde(skip)]
    pub resolved_hardfork: Option<FoundryHardfork>,
    /// Source chain used for trace decoding and external identifiers.
    #[serde(skip)]
    pub source_chain_id: Option<u64>,
    /// Disable the default `Vm` import.
    pub no_vm: bool,
    /// Cached execution backend and its fork identity.
    #[serde(skip)]
    pub(crate) cached_backend: Option<CachedBackend<FEN>>,
    /// Optionally enable traces for the REPL contract execution
    pub traces: bool,
    /// Optionally set calldata for the REPL contract execution
    pub calldata: Option<Vec<u8>>,
    /// Enable viaIR with minimum optimization
    ///
    /// This can fix most of the "stack too deep" errors while resulting a
    /// relatively accurate source map.
    pub ir_minimum: bool,
    /// Whether a cached session needs the current invocation's fork endpoint.
    #[serde(default, getter = "Self::persisted_fork_url_required")]
    pub(crate) fork_url_required: bool,
}

impl<FEN: FoundryEvmNetwork> SessionSourceConfig<FEN> {
    /// Captures the local execution context for sessions saved before it was persisted explicitly.
    pub fn initialize_local_context(&mut self) {
        self.evm_opts.fork_network_is_inferred = self.fork_network_is_inferred;
        self.evm_opts.fork_chain_id_is_inferred = self.fork_chain_id_is_inferred;
        if self.local_networks.is_none() {
            self.local_networks = Some(self.evm_opts.networks);
            self.local_chain_id =
                self.evm_opts.env.chain_id.or(self.foundry_config.chain.map(|chain| chain.id()));
        }
    }

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

    /// Removes credentials from legacy caches while retaining whether the session was forked.
    pub(crate) fn clear_credentials(&mut self) {
        self.fork_url_required |= self.evm_opts.fork_url.is_some();
        let none = InvocationRpc::default();
        none.apply_config_credentials(&mut self.foundry_config);
        none.apply_evm_credentials(&mut self.evm_opts);
    }

    /// Uses credentials from this invocation, never values persisted by an older Chisel version.
    pub(crate) fn restore_credentials(&mut self, current: &InvocationRpc) -> Result<()> {
        let forked = self.fork_url_required || self.evm_opts.fork_url.is_some();
        if forked && current.fork_url.is_none() {
            eyre::bail!(
                "this saved Chisel session requires a fork endpoint; use !fork <url> or restart Chisel with --fork-url to load it"
            );
        }
        current.apply_config_credentials(&mut self.foundry_config);
        current.apply_evm_credentials(&mut self.evm_opts);
        current.apply_transport(&mut self.foundry_config, &mut self.evm_opts);
        if !forked {
            self.evm_opts.fork_url = None;
        }
        self.fork_url_required = false;
        self.evm_opts.fork_endpoint = None;
        self.evm_opts.expected_fork_endpoint = None;
        self.resolved_hardfork = None;
        self.source_chain_id = None;
        self.cached_backend = None;
        Ok(())
    }

    fn persisted_foundry_config(&self) -> Config {
        let mut config = self.foundry_config.clone();
        InvocationRpc::default().apply_config_credentials(&mut config);
        config
    }

    fn persisted_evm_opts(&self) -> EvmOpts {
        let mut evm_opts = self.evm_opts.clone();
        InvocationRpc::default().apply_evm_credentials(&mut evm_opts);
        evm_opts
    }

    /// Records that the session was forked even though its endpoint is not persisted.
    const fn persisted_fork_url_required(&self) -> bool {
        self.fork_url_required || self.evm_opts.fork_url.is_some()
    }
}

impl<FEN: FoundryEvmNetwork> Serialize for SessionSourceConfig<FEN> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Self::serialize(self, serializer)
    }
}

impl<'de, FEN: FoundryEvmNetwork> Deserialize<'de> for SessionSourceConfig<FEN> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::deserialize(deserializer)
    }
}

/// RPC and explorer settings owned by the current Chisel invocation.
///
/// Loading a session restores credentials and transport settings from this invocation.
/// The default value holds no credentials and is used to strip them from persisted sessions.
#[derive(Clone, Debug, Default)]
pub(crate) struct InvocationRpc {
    eth_rpc_url: Option<String>,
    eth_rpc_jwt: Option<String>,
    eth_rpc_headers: Option<Vec<String>>,
    etherscan_api_key: Option<String>,
    etherscan: EtherscanConfigs,
    rpc_endpoints: RpcEndpoints,
    cached_endpoints: CachedEndpoints,
    eth_rpc_timeout: Option<u64>,
    eth_rpc_accept_invalid_certs: bool,
    eth_rpc_no_proxy: bool,
    fork_url: Option<String>,
    fork_headers: Option<Vec<String>>,
    rpc_jwt: Option<String>,
    rpc_headers: Option<Vec<String>>,
    rpc_timeout: Option<u64>,
    rpc_accept_invalid_certs: bool,
    rpc_no_proxy: bool,
}

impl InvocationRpc {
    /// Captures the RPC settings resolved for this invocation.
    pub(crate) fn capture(config: &Config, evm_opts: &EvmOpts) -> Self {
        Self {
            eth_rpc_url: config.eth_rpc_url.clone(),
            eth_rpc_jwt: config.eth_rpc_jwt.clone(),
            eth_rpc_headers: config.eth_rpc_headers.clone(),
            etherscan_api_key: config.etherscan_api_key.clone(),
            etherscan: config.etherscan.clone(),
            rpc_endpoints: config.rpc_endpoints.clone(),
            cached_endpoints: config.rpc_storage_caching.endpoints.clone(),
            eth_rpc_timeout: config.eth_rpc_timeout,
            eth_rpc_accept_invalid_certs: config.eth_rpc_accept_invalid_certs,
            eth_rpc_no_proxy: config.eth_rpc_no_proxy,
            fork_url: evm_opts.fork_url.clone(),
            fork_headers: evm_opts.fork_headers.clone(),
            rpc_jwt: evm_opts.rpc_jwt.clone(),
            rpc_headers: evm_opts.rpc_headers.clone(),
            rpc_timeout: evm_opts.rpc_timeout,
            rpc_accept_invalid_certs: evm_opts.rpc_accept_invalid_certs,
            rpc_no_proxy: evm_opts.rpc_no_proxy,
        }
    }

    /// Replaces the fork endpoint and its endpoint-specific headers after `!fork <url>`.
    pub(crate) fn set_fork(&mut self, url: String, headers: Option<Vec<String>>) {
        self.eth_rpc_url = Some(url.clone());
        self.fork_url = Some(url);
        self.fork_headers = headers;
    }

    fn apply_config_credentials(&self, config: &mut Config) {
        config.eth_rpc_url.clone_from(&self.eth_rpc_url);
        config.eth_rpc_jwt.clone_from(&self.eth_rpc_jwt);
        config.eth_rpc_headers.clone_from(&self.eth_rpc_headers);
        config.etherscan_api_key.clone_from(&self.etherscan_api_key);
        config.etherscan.clone_from(&self.etherscan);
        config.rpc_endpoints.clone_from(&self.rpc_endpoints);
        config.rpc_storage_caching.endpoints.clone_from(&self.cached_endpoints);
    }

    fn apply_evm_credentials(&self, evm_opts: &mut EvmOpts) {
        evm_opts.fork_url.clone_from(&self.fork_url);
        evm_opts.fork_headers.clone_from(&self.fork_headers);
        evm_opts.rpc_jwt.clone_from(&self.rpc_jwt);
        evm_opts.rpc_headers.clone_from(&self.rpc_headers);
    }

    const fn apply_transport(&self, config: &mut Config, evm_opts: &mut EvmOpts) {
        config.eth_rpc_timeout = self.eth_rpc_timeout;
        config.eth_rpc_accept_invalid_certs = self.eth_rpc_accept_invalid_certs;
        config.eth_rpc_no_proxy = self.eth_rpc_no_proxy;
        evm_opts.rpc_timeout = self.rpc_timeout;
        evm_opts.rpc_accept_invalid_certs = self.rpc_accept_invalid_certs;
        evm_opts.rpc_no_proxy = self.rpc_no_proxy;
    }
}

/// REPL Session Source wrapper
///
/// Heavily based on soli's [`ConstructedSource`](https://github.com/jpopesculian/soli/blob/master/src/main.rs#L166)
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct SessionSource<FEN: FoundryEvmNetwork> {
    /// The file name
    pub file_name: String,
    /// The contract name
    pub contract_name: String,

    /// Session Source configuration
    pub config: SessionSourceConfig<FEN>,

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

impl<FEN: FoundryEvmNetwork> Clone for SessionSource<FEN> {
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

impl<FEN: FoundryEvmNetwork> SessionSource<FEN> {
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
    pub fn new(mut config: SessionSourceConfig<FEN>) -> Result<Self> {
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
        // Chisel inspects expression values, so enable Solar's expression type table.
        let compiler = output.parser_mut().solc_mut().compiler_mut();
        compiler.enter_mut(|c| {
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
            vm_import = format!(
                "import {{Vm}} from \"{}\";\n",
                vm_path.path().to_string_lossy().replace('\\', "/")
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_compilers::artifacts::remappings::{RelativeRemapping, RelativeRemappingPathBuf};
    use foundry_evm::core::evm::EthEvmNetwork;
    use std::fs;

    #[test]
    fn initialize_local_context_migrates_legacy_session() {
        let mut config = SessionSourceConfig::<EthEvmNetwork>::default();
        config.evm_opts.networks = NetworkConfigs::with_tempo();
        config.evm_opts.env.chain_id = Some(4217);

        config.initialize_local_context();

        assert_eq!(config.local_networks, Some(NetworkConfigs::with_tempo()));
        assert_eq!(config.local_chain_id, Some(4217));

        config.evm_opts.networks = NetworkConfigs::default();
        config.evm_opts.env.chain_id = Some(1);
        config.initialize_local_context();

        assert_eq!(config.local_networks, Some(NetworkConfigs::with_tempo()));
        assert_eq!(config.local_chain_id, Some(4217));
    }

    #[test]
    fn serialized_session_restores_fork_inference_provenance() {
        let config = SessionSourceConfig::<EthEvmNetwork> {
            fork_network_is_inferred: true,
            fork_chain_id_is_inferred: true,
            ..Default::default()
        };
        let encoded = serde_json::to_string(&config).unwrap();
        let mut decoded =
            serde_json::from_str::<SessionSourceConfig<EthEvmNetwork>>(&encoded).unwrap();

        assert!(!decoded.evm_opts.fork_network_is_inferred);
        assert!(!decoded.evm_opts.fork_chain_id_is_inferred);
        decoded.initialize_local_context();
        assert!(decoded.evm_opts.fork_network_is_inferred);
        assert!(decoded.evm_opts.fork_chain_id_is_inferred);
    }

    #[test]
    fn legacy_session_without_rpc_transport_flags_deserializes() {
        let config = SessionSourceConfig::<EthEvmNetwork>::default();
        let mut legacy_session = serde_json::to_value(config).unwrap();
        let evm_opts = legacy_session["evm_opts"].as_object_mut().expect("serialized EVM options");
        assert!(evm_opts.remove("eth_rpc_accept_invalid_certs").is_some());
        assert!(evm_opts.remove("eth_rpc_no_proxy").is_some());

        let decoded =
            serde_json::from_value::<SessionSourceConfig<EthEvmNetwork>>(legacy_session).unwrap();

        assert!(!decoded.evm_opts.rpc_accept_invalid_certs);
        assert!(!decoded.evm_opts.rpc_no_proxy);
    }

    /// Regression test for <https://github.com/foundry-rs/foundry/issues/14711>.
    ///
    /// `to_repl_source()` must use forward slashes in the Vm import path regardless of OS,
    /// because Solidity import statements require `/` as the path separator.
    #[test]
    fn test_vm_import_path_uses_forward_slashes() {
        let tmp = tempfile::tempdir().unwrap();
        let vm_sol = tmp.path().join("Vm.sol");
        fs::write(&vm_sol, "// dummy").unwrap();

        let remapping = RelativeRemapping {
            context: None,
            name: "forge-std/".to_string(),
            path: RelativeRemappingPathBuf { parent: None, path: tmp.path().to_path_buf() },
        };

        let mut config: SessionSourceConfig<EthEvmNetwork> = SessionSourceConfig {
            foundry_config: Config {
                solc: Some(SolcReq::Version(Version::new(0, 8, 29))),
                remappings: vec![remapping],
                ..Default::default()
            },
            ..Default::default()
        };
        // Pre-set solc so detect_solc() skips the ensure_installed I/O.
        config.detect_solc().unwrap();

        let source = SessionSource {
            file_name: "ReplContract.sol".to_string(),
            contract_name: "REPL".to_string(),
            config,
            global_code: Default::default(),
            contract_code: Default::default(),
            run_code: Default::default(),
            vm_source: vm_source(),
            output: Default::default(),
        };

        let repl = source.to_repl_source();
        let import_line = repl.lines().find(|l| l.contains("import {Vm}")).unwrap();
        assert!(
            !import_line.contains('\\'),
            "Vm import path must not contain backslashes, got: {import_line}"
        );
        assert!(import_line.contains('/'), "Vm import path must use forward slashes");
    }

    #[test]
    fn session_serialization_omits_credentials() {
        let mut config = SessionSourceConfig::<EthEvmNetwork>::default();
        config.foundry_config.eth_rpc_url =
            Some("https://user:synthetic-password@rpc.invalid/key".into());
        config.foundry_config.eth_rpc_jwt = Some("synthetic-jwt".into());
        config.foundry_config.eth_rpc_headers =
            Some(vec!["Authorization: synthetic-header".into()]);
        config.foundry_config.etherscan_api_key = Some("synthetic-api-key".into());
        config.foundry_config.etherscan = serde_json::from_value(serde_json::json!({
            "mainnet": { "key": "synthetic-explorer-key", "chain": 1 }
        }))
        .unwrap();
        config.foundry_config.rpc_endpoints = serde_json::from_value(serde_json::json!({
            "mainnet": "https://rpc.invalid/synthetic-endpoint-key"
        }))
        .unwrap();
        config.foundry_config.rpc_storage_caching.endpoints =
            "synthetic-cache-endpoint-key".parse().unwrap();
        config.evm_opts.fork_url = config.foundry_config.eth_rpc_url.clone();
        config.evm_opts.rpc_jwt = config.foundry_config.eth_rpc_jwt.clone();
        config.evm_opts.rpc_headers = config.foundry_config.eth_rpc_headers.clone();
        config.evm_opts.fork_headers = Some(vec!["Authorization: synthetic-fork-header".into()]);
        config.evm_opts.fork_block_number = Some(42);
        config.calldata = Some(vec![0xde, 0xad, 0xbe, 0xef]);
        config.traces = true;

        let encoded = serde_json::to_string(&config).unwrap();
        let decoded: SessionSourceConfig<EthEvmNetwork> = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.foundry_config.eth_rpc_url, None);
        assert_eq!(decoded.foundry_config.eth_rpc_jwt, None);
        assert_eq!(decoded.foundry_config.eth_rpc_headers, None);
        assert_eq!(decoded.foundry_config.etherscan_api_key, None);
        assert!(decoded.foundry_config.etherscan.is_empty());
        assert!(decoded.foundry_config.rpc_endpoints.is_empty());
        assert_eq!(decoded.foundry_config.rpc_storage_caching.endpoints.to_string(), "all");
        assert_eq!(decoded.evm_opts.fork_url, None);
        assert_eq!(decoded.evm_opts.rpc_jwt, None);
        assert_eq!(decoded.evm_opts.rpc_headers, None);
        assert_eq!(decoded.evm_opts.fork_headers, None);
        assert_eq!(decoded.evm_opts.fork_block_number, Some(42));
        assert_eq!(decoded.calldata, Some(vec![0xde, 0xad, 0xbe, 0xef]));
        assert!(decoded.traces);
        assert!(decoded.fork_url_required);
        assert_eq!(config.foundry_config.eth_rpc_jwt.as_deref(), Some("synthetic-jwt"));
        assert_eq!(config.evm_opts.fork_url, config.foundry_config.eth_rpc_url);
    }

    /// Pins the persisted schema so that a new field is a deliberate decision to cache or skip it.
    #[test]
    fn session_serialization_round_trips_persisted_fields() {
        let config = SessionSourceConfig::<EthEvmNetwork> {
            foundry_config: Config { optimizer_runs: Some(500), ..Default::default() },
            evm_opts: EvmOpts { fork_block_number: Some(42), ..Default::default() },
            local_networks: Some(NetworkConfigs::with_tempo()),
            local_chain_id: Some(4217),
            fork_network_is_inferred: true,
            fork_chain_id_is_inferred: true,
            no_vm: true,
            traces: true,
            calldata: Some(vec![1, 2, 3]),
            ir_minimum: true,
            fork_url_required: true,
            source_chain_id: Some(1),
            ..Default::default()
        };

        let encoded = serde_json::to_value(&config).unwrap();
        let mut keys = encoded.as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "calldata",
                "evm_opts",
                "fork_chain_id_is_inferred",
                "fork_network_is_inferred",
                "fork_url_required",
                "foundry_config",
                "ir_minimum",
                "local_chain_id",
                "local_networks",
                "no_vm",
                "traces",
            ]
        );

        let decoded =
            serde_json::from_value::<SessionSourceConfig<EthEvmNetwork>>(encoded).unwrap();
        assert_eq!(decoded.foundry_config.optimizer_runs, Some(500));
        assert_eq!(decoded.evm_opts.fork_block_number, Some(42));
        assert!(decoded.local_networks.is_some_and(|networks| networks.is_tempo()));
        assert_eq!(decoded.local_chain_id, Some(4217));
        assert!(decoded.fork_network_is_inferred);
        assert!(decoded.fork_chain_id_is_inferred);
        assert!(decoded.no_vm);
        assert!(decoded.traces);
        assert_eq!(decoded.calldata, Some(vec![1, 2, 3]));
        assert!(decoded.ir_minimum);
        assert!(decoded.fork_url_required);
        assert_eq!(decoded.source_chain_id, None);
    }

    #[test]
    fn saved_fork_restores_current_credentials_and_execution_settings() {
        let mut saved = SessionSourceConfig::<EthEvmNetwork>::default();
        saved.evm_opts.fork_url = Some("https://rpc.invalid/old-token".into());
        saved.evm_opts.fork_block_number = Some(42);
        saved.evm_opts.env.chain_id = Some(1);
        saved.foundry_config.optimizer_runs = Some(500);
        saved.foundry_config.eth_rpc_accept_invalid_certs = true;
        saved.foundry_config.eth_rpc_no_proxy = true;
        saved.evm_opts.rpc_accept_invalid_certs = true;
        saved.evm_opts.rpc_no_proxy = true;
        saved.calldata = Some(vec![1, 2, 3]);
        saved.ir_minimum = true;
        let encoded = serde_json::to_string(&saved).unwrap();
        let mut saved =
            serde_json::from_str::<SessionSourceConfig<EthEvmNetwork>>(&encoded).unwrap();
        let mut current = SessionSourceConfig::<EthEvmNetwork>::default();
        current.foundry_config.eth_rpc_url = Some("https://rpc.invalid/new-token".into());
        current.foundry_config.eth_rpc_jwt = Some("current-jwt".into());
        current.foundry_config.eth_rpc_headers = Some(vec!["Authorization: current-header".into()]);
        current.foundry_config.etherscan_api_key = Some("current-api-key".into());
        current.foundry_config.rpc_endpoints = serde_json::from_value(serde_json::json!({
            "mainnet": "https://rpc.invalid/current-endpoint-token"
        }))
        .unwrap();
        current.foundry_config.etherscan = serde_json::from_value(serde_json::json!({
            "mainnet": { "key": "current-explorer-key", "chain": 1 }
        }))
        .unwrap();
        current.foundry_config.rpc_storage_caching.endpoints =
            "current-cache-endpoint-token".parse().unwrap();
        current.evm_opts.fork_url = current.foundry_config.eth_rpc_url.clone();
        current.evm_opts.rpc_jwt = current.foundry_config.eth_rpc_jwt.clone();
        current.evm_opts.rpc_headers = current.foundry_config.eth_rpc_headers.clone();
        current.evm_opts.fork_headers = Some(vec!["Authorization: current-fork-header".into()]);
        current.evm_opts.fork_block_number = Some(100);
        current.evm_opts.env.chain_id = Some(10);
        current.foundry_config.eth_rpc_timeout = Some(30);
        current.evm_opts.rpc_timeout = Some(30);

        saved
            .restore_credentials(&InvocationRpc::capture(
                &current.foundry_config,
                &current.evm_opts,
            ))
            .unwrap();

        assert_eq!(saved.foundry_config.eth_rpc_url, current.foundry_config.eth_rpc_url);
        assert_eq!(saved.foundry_config.eth_rpc_jwt, current.foundry_config.eth_rpc_jwt);
        assert_eq!(saved.foundry_config.eth_rpc_headers, current.foundry_config.eth_rpc_headers);
        assert_eq!(
            saved.foundry_config.etherscan_api_key,
            current.foundry_config.etherscan_api_key
        );
        assert_eq!(saved.foundry_config.etherscan, current.foundry_config.etherscan);
        assert_eq!(saved.foundry_config.rpc_endpoints, current.foundry_config.rpc_endpoints);
        assert_eq!(
            saved.foundry_config.rpc_storage_caching.endpoints,
            current.foundry_config.rpc_storage_caching.endpoints
        );
        assert_eq!(saved.evm_opts.fork_url, current.evm_opts.fork_url);
        assert_eq!(saved.evm_opts.rpc_jwt, current.evm_opts.rpc_jwt);
        assert_eq!(saved.evm_opts.rpc_headers, current.evm_opts.rpc_headers);
        assert_eq!(saved.evm_opts.fork_headers, current.evm_opts.fork_headers);
        assert!(!saved.foundry_config.eth_rpc_accept_invalid_certs);
        assert!(!saved.foundry_config.eth_rpc_no_proxy);
        assert!(!saved.evm_opts.rpc_accept_invalid_certs);
        assert!(!saved.evm_opts.rpc_no_proxy);
        assert_eq!(saved.foundry_config.eth_rpc_timeout, Some(30));
        assert_eq!(saved.evm_opts.rpc_timeout, Some(30));
        assert_eq!(saved.evm_opts.fork_block_number, Some(42));
        assert_eq!(saved.evm_opts.env.chain_id, Some(1));
        assert_eq!(saved.foundry_config.optimizer_runs, Some(500));
        assert_eq!(saved.calldata, Some(vec![1, 2, 3]));
        assert!(saved.ir_minimum);
    }

    #[test]
    fn legacy_session_credentials_are_not_reused() {
        let mut legacy = SessionSourceConfig::<EthEvmNetwork>::default();
        legacy.foundry_config.eth_rpc_jwt = Some("legacy-jwt".into());
        legacy.foundry_config.etherscan_api_key = Some("legacy-key".into());
        legacy.evm_opts.rpc_headers = Some(vec!["Authorization: legacy-header".into()]);
        legacy.evm_opts.fork_headers = Some(vec!["Authorization: legacy-fork-header".into()]);
        legacy.restore_credentials(&InvocationRpc::default()).unwrap();

        assert_eq!(legacy.foundry_config.eth_rpc_jwt, None);
        assert_eq!(legacy.foundry_config.etherscan_api_key, None);
        assert_eq!(legacy.evm_opts.rpc_headers, None);
        assert_eq!(legacy.evm_opts.fork_headers, None);
    }

    #[test]
    fn saved_fork_requires_current_endpoint() {
        let mut saved = SessionSourceConfig::<EthEvmNetwork>::default();
        saved.evm_opts.fork_url = Some("https://rpc.invalid/legacy-token".into());
        saved.clear_credentials();
        assert_eq!(saved.evm_opts.fork_url, None);
        assert!(saved.fork_url_required);

        let error = saved.restore_credentials(&InvocationRpc::default()).unwrap_err();

        assert_eq!(
            error.to_string(),
            "this saved Chisel session requires a fork endpoint; use !fork <url> or restart Chisel with --fork-url to load it"
        );
    }

    #[test]
    fn saved_local_session_stays_local_with_current_fork_endpoint() {
        let mut saved = SessionSourceConfig::<EthEvmNetwork>::default();
        let mut current = SessionSourceConfig::<EthEvmNetwork>::default();
        current.evm_opts.fork_url = Some("https://rpc.invalid/current-token".into());

        saved
            .restore_credentials(&InvocationRpc::capture(
                &current.foundry_config,
                &current.evm_opts,
            ))
            .unwrap();

        assert_eq!(saved.evm_opts.fork_url, None);
        assert!(!saved.fork_url_required);
    }
}
