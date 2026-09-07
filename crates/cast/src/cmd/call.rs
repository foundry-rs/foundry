use super::{
    auth::{confirm_auth_rpc_disclosure, confirm_auth_rpc_disclosure_before_network_resolution},
    call_overrides::CallOverrideOpts,
    fetch_code_via_rpc, print_raw_line,
    run::{
        block_num_hash, call_tracer_frame, fetch_contracts_bytecode_from_trace, trace_addresses,
    },
};
use crate::{
    debug::{ensure_remote_trace_context_unchanged, handle_traces, resolve_remote_trace_hardfork},
    rpc_trace::call_frame_to_arena,
    traces::TraceKind,
    tx::{CastTxBuilder, SenderKind, read_only_sender},
};
use alloy_consensus::BlockHeader;
use alloy_dyn_abi::FunctionExt;
use alloy_eips::BlockNumHash;
use alloy_ens::NameOrAddress;
use alloy_network::{
    BlockResponse, NetworkTransactionBuilder, TransactionBuilder, primitives::HeaderResponse,
};
use alloy_primitives::{B256, Bytes, TxKind, U256, hex, map::AddressHashMap};
use alloy_provider::{Provider, ext::DebugApi};
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag,
    trace::geth::{
        CallConfig, GethDebugBuiltInTracerType, GethDebugTracerType, GethDebugTracingCallOptions,
        GethDebugTracingOptions,
    },
};
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::{
    opts::{ChainValueParser, RpcOpts, TracingArgs, TransactionOpts},
    utils::{LoadConfig, TraceResult, parse_ether_value},
};
use foundry_common::{
    FoundryTransactionBuilder,
    abi::{encode_function_args, get_func},
    fmt::{format_token, serialize_value_as_json},
    provider::{ProviderBuilder, curl_transport::generate_curl_command},
    sh_println, shell,
};
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{
    Chain, Config, TracingConfig,
    figment::{
        self, Metadata, Profile,
        value::{Dict, Map},
    },
};
#[cfg(feature = "monad")]
use foundry_evm::core::evm::MonadEvmNetwork;
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
use foundry_evm::{
    core::{
        FoundryBlock, FoundryTransaction,
        decode::RevertDecoder,
        evm::{EthEvmNetwork, FoundryEvmNetwork, TempoEvmNetwork},
    },
    executors::{ExecutorBuilder, TracingExecutor},
    opts::EvmOpts,
    traces::{InternalTraceMode, SparsedTraceArena, TraceContext, TraceRequirements},
};
use foundry_evm_networks::NetworkConfigs;
use foundry_wallets::{BrowserWalletOpts, WalletOpts};
use std::str::FromStr;

/// CLI arguments for `cast call`.
///
/// ## State Override Flags
///
/// The following flags can be used to override the state for the call:
///
/// * `--override-balance <address>:<balance>` - Override the balance of an account
/// * `--override-nonce <address>:<nonce>` - Override the nonce of an account
/// * `--override-code <address>:<code>` - Override the code of an account
/// * `--override-state <address>:<slot>:<value>` - Override a storage slot of an account
///
/// Multiple overrides can be specified for the same account. For example:
///
/// ```bash
/// cast call 0x... "transfer(address,uint256)" 0x... 100 \
///   --override-balance 0x123:0x1234 \
///   --override-nonce 0x123:1 \
///   --override-code 0x123:0x1234 \
///   --override-state 0x123:0x1:0x1234
///   --override-state-diff 0x123:0x1:0x1234
/// ```
///
/// `--delegate` builds on the same mechanism: it overrides the code of the `--from` address with
/// the destination's code so the call runs as a `delegatecall`.
#[derive(Debug, Parser)]
pub struct CallArgs {
    /// The destination of the transaction.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    #[arg(allow_negative_numbers = true)]
    args: Vec<String>,

    /// Raw hex-encoded data for the transaction. Used instead of `SIG` and `ARGS`.
    #[arg(
        long,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    /// Forks the remote rpc, executes the transaction locally and prints a trace
    #[arg(long, default_value_t = false)]
    trace: bool,

    /// Simulate the call as a `delegatecall` from the `--from` address.
    ///
    /// The destination's runtime code is applied as a code override on the `--from` address and
    /// the call is then made to that address, so the destination's code runs in the caller's
    /// storage context, like an on-chain `delegatecall`.
    ///
    /// Note that the executed code observes `msg.sender` (and `tx.origin`) equal to the `--from`
    /// address itself, whereas in an on-chain `delegatecall` `msg.sender` is preserved from the
    /// delegating contract's own caller.
    #[arg(long, requires = "from", conflicts_with = "browser")]
    delegate: bool,

    /// Fetch the call trace from the node via `debug_traceCall` (callTracer) and render it,
    /// instead of re-executing the call locally like `--trace`.
    ///
    /// This is a call-tree view: nested calls, value, gas, emitted logs and revert data. It does
    /// not provide the opcode / struct-log level detail of a local `--trace` / `--debug` run.
    ///
    /// The local-execution-only trace flags (`--debug`, `--decode-internal`, `--evm-version`) do
    /// not apply, since the trace comes from the node rather than a local run.
    #[arg(
        long = "debug-trace-call",
        default_value_t = false,
        conflicts_with_all = ["trace", "debug", "decode_internal", "evm_version"]
    )]
    debug_trace_call: bool,

    /// Opens an interactive debugger.
    /// Can only be used with `--trace`.
    #[arg(long, requires = "trace")]
    debug: bool,

    #[command(flatten)]
    tracing: TracingArgs,

    /// The EVM Version to use.
    /// Can only be used with `--trace`.
    #[arg(long, requires = "trace")]
    evm_version: Option<EvmVersion>,

    /// The block height to query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[arg(long, short)]
    block: Option<BlockId>,

    #[command(subcommand)]
    command: Option<CallSubcommands>,

    #[command(flatten)]
    tx: TransactionOpts,

    /// Skip the EIP-7702 authorization disclosure confirmation.
    #[arg(long)]
    force: bool,

    #[command(flatten)]
    rpc: RpcOpts,

    #[command(flatten)]
    wallet: WalletOpts,

    #[command(flatten)]
    browser: BrowserWalletOpts,

    #[arg(
        short,
        long,
        alias = "chain-id",
        env = "CHAIN",
        value_parser = ChainValueParser::default(),
    )]
    pub chain: Option<Chain>,

    /// Use current project artifacts for trace decoding.
    #[arg(long, visible_alias = "la")]
    pub with_local_artifacts: bool,

    #[command(flatten)]
    pub overrides: CallOverrideOpts,
}

#[derive(Debug, Parser)]
pub enum CallSubcommands {
    /// ignores the address field and simulates creating a contract
    #[command(name = "--create")]
    Create {
        /// Bytecode of contract.
        code: String,

        /// The signature of the constructor.
        sig: Option<String>,

        /// The arguments of the constructor.
        #[arg(allow_negative_numbers = true)]
        args: Vec<String>,

        /// Ether to send in the transaction.
        ///
        /// Either specified in wei, or as a string with a unit type.
        ///
        /// Examples: 1ether, 10gwei, 0.01ether
        #[arg(long, value_parser = parse_ether_value)]
        value: Option<U256>,
    },
}

/// The `callTracer` options shared by `--debug-trace-call` and its `--curl` rendering.
fn call_tracer_options() -> GethDebugTracingCallOptions {
    GethDebugTracingCallOptions::default().with_tracing_options(
        GethDebugTracingOptions::default()
            .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
            .with_call_config(CallConfig::default().with_log()),
    )
}

impl CallArgs {
    fn resolve_tracing(&self, config: &TracingConfig, verbosity: u8) -> TracingConfig {
        if self.debug_trace_call {
            self.tracing.resolve_call_tracer(config, verbosity)
        } else {
            self.tracing.resolve(config, verbosity)
        }
    }

    pub async fn run(mut self) -> Result<()> {
        self.validate_trace_args()?;

        // Handle --curl mode early, before any provider interaction
        if self.rpc.curl {
            if self.browser.browser {
                eyre::bail!("--browser cannot be combined with --curl; use --from <ADDRESS>");
            }
            if self.delegate {
                // The code override that makes the call a `delegatecall` is read from the node,
                // which `--curl` deliberately never contacts.
                eyre::bail!("--delegate cannot be combined with --curl");
            }
            return self.run_curl().await;
        }

        let figment = self.rpc.clone().into_figment(self.with_local_artifacts).merge(&self);
        let (mut config, mut evm_opts) = super::load_cast_config_and_evm_opts(figment)?;
        evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());
        if self.tx.tempo.is_tempo() {
            evm_opts.networks = NetworkConfigs::with_tempo();
        } else if let Some(chain) = self.chain {
            evm_opts.networks =
                evm_opts.networks.try_with_chain_id(chain.id()).map_err(eyre::Report::msg)?;
        }
        let Some(auth_preflight) = self.preflight_auth_disclosure().await? else {
            return Ok(());
        };
        evm_opts.infer_network_from_fork().await?;
        if self.chain.is_none()
            && let Some(chain_id) = evm_opts.env.chain_id
        {
            let chain = Chain::from_id(chain_id);
            self.chain = Some(chain);
            config.chain = Some(chain);
        }

        if evm_opts.networks.is_tempo() {
            return self
                .run_with_network_and_opts::<TempoEvmNetwork>(
                    config,
                    evm_opts,
                    auth_preflight,
                    ExecutorBuilder::<TempoEvmNetwork>::new(),
                )
                .await;
        }

        #[cfg(feature = "monad")]
        if evm_opts.networks.is_monad() {
            return self
                .run_with_network_and_opts::<MonadEvmNetwork>(
                    config,
                    evm_opts,
                    auth_preflight,
                    ExecutorBuilder::<MonadEvmNetwork>::new(),
                )
                .await;
        }

        #[cfg(feature = "optimism")]
        if evm_opts.networks.is_optimism() {
            return self
                .run_with_network_and_opts::<OpEvmNetwork>(
                    config,
                    evm_opts,
                    auth_preflight,
                    ExecutorBuilder::<OpEvmNetwork>::new(),
                )
                .await;
        }

        self.run_with_network_and_opts::<EthEvmNetwork>(
            config,
            evm_opts,
            auth_preflight,
            ExecutorBuilder::<EthEvmNetwork>::new(),
        )
        .await
    }

    /// Returns whether resolving this call can disclose an authorization before the transaction
    /// builder exists. This mirrors the builder's disclosure check after applying `.raw()`.
    const fn will_disclose_auth(&self) -> bool {
        !self.tx.auth.is_empty() && (!self.trace || matches!(self.tx.access_list, Some(None)))
    }

    /// Confirms the authorization disclosure before the network is resolved.
    ///
    /// Returns `None` when the user declined, otherwise whether the disclosure was confirmed
    /// along with the sender resolved for it (absent for browser wallets).
    async fn preflight_auth_disclosure(
        &self,
    ) -> Result<Option<(bool, Option<SenderKind<'static>>)>> {
        if !self.will_disclose_auth() {
            return Ok(Some((false, None)));
        }

        let sender = if self.browser.browser {
            None
        } else {
            Some(SenderKind::from_wallet_opts(self.wallet.clone()).await?)
        };
        let browser_sender = SenderKind::from(self.wallet.from.unwrap_or_default());
        let validation_sender = sender.as_ref().unwrap_or(&browser_sender);
        if !confirm_auth_rpc_disclosure_before_network_resolution(
            &self.tx.auth,
            validation_sender,
            self.force,
        )? {
            return Ok(None);
        }

        Ok(Some((true, sender)))
    }

    fn validate_trace_args(&self) -> Result<()> {
        if !self.trace
            && !self.debug_trace_call
            && (self.tracing.disable_labels
                || self.tracing.compact_labels
                || !self.tracing.labels.is_empty()
                || self.tracing.trace_depth.is_some())
        {
            eyre::bail!("trace rendering options require `--trace` or `--debug-trace-call`");
        }

        if self.tracing.decode_internal && !self.trace {
            eyre::bail!("`--decode-internal` requires `--trace`");
        }

        Ok(())
    }

    async fn run_with_network_and_opts<FEN: FoundryEvmNetwork>(
        self,
        mut config: Box<Config>,
        evm_opts: EvmOpts,
        (auth_confirmed, auth_sender): (bool, Option<SenderKind<'static>>),
        executor_builder: ExecutorBuilder<FEN>,
    ) -> Result<()> {
        config.networks = evm_opts.networks;
        let mut state_overrides = self.overrides.get_state_overrides()?;
        let block_overrides = self.overrides.get_block_overrides()?;
        config.tracing = self.resolve_tracing(&config.tracing, shell::verbosity());
        let tracing = config.tracing.clone();

        let Self {
            mut to,
            mut sig,
            mut args,
            mut tx,
            command,
            block,
            trace,
            debug_trace_call,
            evm_version,
            debug,
            data,
            with_local_artifacts,
            wallet,
            browser,
            force,
            delegate,
            ..
        } = self;

        if let Some(data) = data {
            sig = Some(data);
        }

        let provider = ProviderBuilder::<FEN::Network>::from_config(&config)?.build()?;
        let endpoint_identity =
            if debug_trace_call { Some(evm_opts.discover_fork_endpoint().await?) } else { None };
        let sender = match auth_sender {
            Some(sender) => sender,
            None => read_only_sender::<FEN::Network>(&browser, wallet).await?.0,
        };
        let from = sender.address();

        // A `delegatecall` runs the destination's code against the caller's storage, which
        // `eth_call` cannot express. Overriding the caller's code with the destination's and
        // then calling the caller reproduces that context. The retarget to the caller happens
        // after the transaction is built, so calldata encoding and function resolution still
        // see the destination.
        if delegate {
            if command.is_some() {
                eyre::bail!("`--delegate` cannot be combined with `--create`");
            }
            let Some(target) = to else {
                eyre::bail!("`--delegate` requires a destination address");
            };
            let target = target.resolve(&provider).await?;
            let overrides = state_overrides.get_or_insert_with(Default::default);
            if overrides.get(&from).is_some_and(|account| account.code.is_some()) {
                eyre::bail!("`--delegate` conflicts with `--override-code` for the sender {from}");
            }
            // A code override for the destination is the state this call runs against, so it
            // takes precedence over the deployed code.
            let code = match overrides.get(&target).and_then(|account| account.code.clone()) {
                Some(code) => code,
                None => provider.get_code_at(target).block_id(block.unwrap_or_default()).await?,
            };
            if code.is_empty() {
                eyre::bail!("`--delegate` destination {target} has no code to delegate to");
            }
            overrides.entry(from).or_default().code = Some(code);
            to = Some(NameOrAddress::Address(target));
        }

        let code = if let Some(CallSubcommands::Create {
            code,
            sig: create_sig,
            args: create_args,
            value,
        }) = command
        {
            sig = create_sig;
            args = create_args;
            if let Some(value) = value {
                tx.value = Some(value);
            }
            Some(code)
        } else {
            None
        };

        let builder = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .raw();
        let will_disclose =
            (!trace && builder.has_auth()) || builder.will_disclose_auth_during_build();
        if will_disclose
            && !auth_confirmed
            && !confirm_auth_rpc_disclosure(&builder, &sender, force)?
        {
            return Ok(());
        }
        let (mut tx, func) = builder.build(sender).await?;

        // The delegate override put the destination's code on the sender, so the built call is
        // aimed at the sender; the calldata above was still encoded against the destination.
        if delegate {
            tx.set_to(from);
        }

        if debug_trace_call {
            let endpoint_identity = endpoint_identity
                .ok_or_else(|| eyre::eyre!("remote trace endpoint identity was not captured"))?;
            let requested_block = block.unwrap_or(BlockId::latest());
            let fetched_block = provider.get_block(requested_block).await?;
            let resolved_canonical_block =
                if matches!(requested_block, BlockId::Number(_)) && !requested_block.is_pending() {
                    fetched_block.as_ref().map(block_num_hash)
                } else {
                    None
                };
            let block = pin_remote_trace_block(
                requested_block,
                fetched_block.as_ref().map(|block| block.header().hash()),
            )?;
            let block_time_override = block_overrides.as_ref().and_then(|overrides| overrides.time);
            let mut call_options = call_tracer_options();
            // A contract that only exists through a `--override-code` entry has no on-chain
            // code to fetch for local-artifact matching, so remember the override code before
            // handing the overrides to `debug_traceCall`.
            let mut override_bytecode = AddressHashMap::<Bytes>::default();
            if with_local_artifacts && let Some(overrides) = &state_overrides {
                for (address, account) in overrides {
                    if let Some(code) = &account.code {
                        override_bytecode.insert(*address, code.clone());
                    }
                }
            }

            // Honour the same state / block overrides as the local `--trace` path.
            if let Some(state_overrides) = state_overrides {
                call_options = call_options.with_state_overrides(state_overrides);
            }
            if let Some(block_overrides) = block_overrides {
                call_options = call_options.with_block_overrides(block_overrides);
            }

            let frame = call_tracer_frame(
                provider.debug_trace_call(tx, block, call_options).await,
                "debug_traceCall",
                "drop `--debug-trace-call` to run the call locally with `--trace`",
                "the requested block; use an archive endpoint, or target a more recent block with `--block`",
            )?;

            let arena = SparsedTraceArena {
                arena: call_frame_to_arena(&frame, None),
                ignored: Default::default(),
                diagnostics: Default::default(),
            };
            let result = TraceResult {
                success: frame.error.is_none() && frame.revert_reason.is_none(),
                traces: Some(vec![(TraceKind::Execution, arena)]),
                gas_used: frame.gas_used.saturating_to(),
            };

            // Local-artifact labeling matches deployed runtime bytecode against the
            // project artifacts. There is no local executor on this path, so fetch the code
            // over RPC for the addresses in the trace. Skip the extra round-trips unless
            // local artifacts were requested.
            let contracts_bytecode = if with_local_artifacts {
                let mut contracts_bytecode =
                    fetch_code_via_rpc(&provider, trace_addresses(&result), block).await;
                // The trace ran the override code, not the on-chain code, so the override
                // wins for artifact matching.
                contracts_bytecode.extend(override_bytecode);
                contracts_bytecode
            } else {
                Default::default()
            };
            let final_endpoint_identity = evm_opts.discover_fork_endpoint().await?;
            ensure_remote_trace_context_unchanged(&endpoint_identity, &final_endpoint_identity)?;

            // The remote node executed this trace, so its reported family is authoritative for
            // decoding even when the caller selected a compatible local EVM implementation.
            let chain = alloy_chains::Chain::from_id(endpoint_identity.source_chain_id);
            let block_timestamp = block_time_override
                .or_else(|| fetched_block.as_ref().map(|block| block.header().timestamp()));
            let resolved_hardfork =
                resolve_remote_trace_hardfork(config.hardfork, &endpoint_identity, block_timestamp);
            if let Some(resolved_block) = resolved_canonical_block {
                let canonical_block =
                    provider.get_block_by_number(resolved_block.number.into()).await?;
                ensure_remote_trace_block_is_canonical(
                    resolved_block,
                    canonical_block.as_ref().map(block_num_hash),
                )?;
            }
            return handle_traces(
                result,
                &config,
                TraceContext::new(chain, endpoint_identity.network_profile, resolved_hardfork),
                &contracts_bytecode,
                &tracing,
                with_local_artifacts,
                false,
            )
            .await;
        }

        if trace {
            if let Some(BlockId::Number(BlockNumberOrTag::Number(block_number))) = block {
                // Override Config `fork_block_number` (if set) with CLI value.
                config.fork_block_number = Some(block_number);
            }

            let create2_deployer = evm_opts.create2_deployer;
            let mut fork = TracingExecutor::<FEN>::get_fork(&mut config, evm_opts).await?;
            // Modify settings usually set in eth_call while keeping execution gas bounded.
            fork.evm_env.cfg_env.disable_block_gas_limit = true;
            fork.evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);

            if let Some(block_overrides) = block_overrides {
                if let Some(number) = block_overrides.number {
                    fork.evm_env.block_env.set_number(number.to());
                }
                if let Some(time) = block_overrides.time {
                    fork.evm_env.block_env.set_timestamp(U256::from(time));
                }
            }
            fork.resolve_spec(&config, evm_version);
            fork.extend_precompile_labels(&mut config);
            let context = fork.context();

            let trace_requirements = TraceRequirements::none()
                .with_calls(true)
                .with_debug(debug)
                .with_decode_internal(if tracing.decode_internal {
                    InternalTraceMode::Full
                } else {
                    InternalTraceMode::None
                })
                .with_state_changes(tracing.verbosity > 4);
            let mut executor = fork.into_executor(
                executor_builder,
                trace_requirements,
                create2_deployer,
                state_overrides,
            )?;

            let value = tx.value().unwrap_or_default();
            let input = tx.input().cloned().unwrap_or_default();
            let tx_kind = tx.kind().expect("set by builder");

            // Apply a user-provided `--gas-limit` to the executor. `build_test_env` propagates the
            // executor's gas limit to the executed call/deploy, so setting it here is what takes
            // effect; writing it onto the tx env directly would be overwritten.
            if let Some(gas_limit) = tx.gas_limit() {
                executor.set_gas_limit(gas_limit);
            }

            // Set transaction options with --trace
            let env_tx = executor.tx_env_mut();
            if let Some(gas_price) = tx.max_fee_per_gas().or(tx.gas_price()) {
                env_tx.set_gas_price(gas_price);
            }
            if let Some(max_priority_fee_per_gas) = tx.max_priority_fee_per_gas() {
                env_tx.set_gas_priority_fee(Some(max_priority_fee_per_gas));
            }
            if let Some(max_fee_per_blob_gas) = tx.max_fee_per_blob_gas() {
                env_tx.set_max_fee_per_blob_gas(max_fee_per_blob_gas);
            }
            if let Some(nonce) = tx.nonce() {
                env_tx.set_nonce(nonce);
            }
            env_tx.set_tx_type(tx.output_tx_type().into());
            if let Some(access_list) = tx.access_list().cloned() {
                env_tx.set_access_list(access_list);
            }
            if let Some(auth) = tx.authorization_list().cloned() {
                env_tx.set_signed_authorization(auth);
            }

            let trace = match tx_kind {
                TxKind::Create => {
                    let deploy_result = executor.deploy(from, input, value, None);
                    TraceResult::try_from(deploy_result)?
                }
                TxKind::Call(to) => TraceResult::from_raw(
                    executor.transact_raw(from, to, input, value)?,
                    TraceKind::Execution,
                ),
            };

            let contracts_bytecode = fetch_contracts_bytecode_from_trace(&executor, &trace)?;
            return handle_traces(
                trace,
                &config,
                context,
                &contracts_bytecode,
                &tracing,
                with_local_artifacts,
                debug,
            )
            .await;
        }

        let mut call = provider
            .call(tx.clone())
            .block(block.unwrap_or_default())
            .with_block_overrides_opt(block_overrides);
        if let Some(state_override) = state_overrides {
            call = call.overrides(state_override)
        }

        let res = match call.await {
            Ok(res) => res,
            Err(err) => {
                let data = err.as_error_resp().and_then(|payload| payload.as_revert_data());
                if let Some(data) = data {
                    let decoded = match RevertDecoder::new().maybe_decode_known(&data) {
                        Some(decoded) => Some(decoded),
                        None => crate::tx::decode_custom_error(&data).await.ok().flatten(),
                    };
                    if let Some(decoded) = decoded {
                        return Err(err).wrap_err(format!("execution reverted: {decoded}"));
                    }
                }
                return Err(err.into());
            }
        };
        let decoded = match func.as_ref() {
            Some(func) => match func.abi_decode_output(res.as_ref()) {
                Ok(decoded) => decoded,
                Err(err) => {
                    // An empty response usually means the recipient is not a contract.
                    if res.is_empty() {
                        let Some(addr) = tx.to() else {
                            eyre::bail!("tx req is a contract deployment");
                        };
                        if let Ok(code) =
                            provider.get_code_at(addr).block_id(block.unwrap_or_default()).await
                            && code.is_empty()
                        {
                            eyre::bail!("contract {addr:?} does not have any code");
                        }
                    }
                    return Err(err).wrap_err(
                        "could not decode output; did you specify the wrong function return data type?"
                    );
                }
            },
            None => vec![],
        };

        // handle case when return type is not specified
        let response = if decoded.is_empty() {
            res.to_string()
        } else if shell::is_json() {
            let tokens = decoded
                .into_iter()
                .map(|value| serialize_value_as_json(value, None, true))
                .collect::<eyre::Result<Vec<_>>>()?;
            serde_json::to_string_pretty(&tokens).unwrap()
        } else {
            // seth compatible user-friendly return type conversions
            decoded.iter().map(format_token).collect::<Vec<_>>().join("\n")
        };

        // With `--delegate` the call targets the sender, whose code comes from the override and
        // was already checked to be non-empty, so the on-chain code lookup would be misleading.
        if response == "0x"
            && !delegate
            && let Some(contract_address) = tx.to()
            && provider.get_code_at(contract_address).await?.is_empty()
        {
            sh_warn!("Contract code is empty")?;
        }

        print_raw_line(response)
    }

    /// Handle --curl mode by generating curl command without any RPC interaction.
    async fn run_curl(self) -> Result<()> {
        let config = self.rpc.load_config()?;
        let url = config.get_rpc_url_or_localhost_http()?;
        let jwt = config.get_rpc_jwt_secret()?;

        // Get call data - either from --data or from sig + args
        let data = if let Some(data) = &self.data {
            hex::decode(data)?
        } else if let Some(sig) = &self.sig {
            // If sig is already hex data, use it directly
            match hex::decode(sig) {
                Ok(data) => data,
                Err(_) => encode_function_args(&get_func(sig)?, &self.args)?,
            }
        } else {
            Vec::new()
        };

        // Resolve the destination address (must be a raw address for curl mode)
        let to = self.to.as_ref().map(|n| match n {
            NameOrAddress::Address(addr) => Ok(*addr),
            NameOrAddress::Name(name) => {
                eyre::bail!("ENS names are not supported with --curl. Please use a raw address instead of '{}'", name);
            }
        }).transpose()?;

        // Build eth_call params. `--curl` builds the request offline, so the fields the
        // RPC-backed builder would resolve against the node (fee style, blob sidecars,
        // authorization lists) are left to the node's defaults; the scalar fields given on the
        // command line are forwarded as-is so the printed request runs the same call as the
        // non-curl command.
        let mut call_object = serde_json::json!({
            "to": to,
            "data": format!("0x{}", hex::encode(&data)),
        });
        if let Some(from) = self.wallet.from {
            call_object["from"] = serde_json::json!(from);
        }
        if let Some(value) = self.tx.value {
            call_object["value"] = serde_json::json!(value);
        }
        if let Some(gas_limit) = self.tx.gas_limit {
            call_object["gas"] = serde_json::json!(gas_limit);
        }
        if let Some(nonce) = self.tx.nonce {
            call_object["nonce"] = serde_json::json!(nonce);
        }

        let block_param = self
            .block
            .map(|b| serde_json::to_value(b).unwrap_or(serde_json::json!("latest")))
            .unwrap_or(serde_json::json!("latest"));

        // `--debug-trace-call` fetches a callTracer trace of the call instead of executing it,
        // so the curl payload must target `debug_traceCall` with the same third param as the
        // non-curl path: the tracer options plus any state / block overrides, so the printed
        // request traces the same state as the command it represents.
        let (method, params) = if self.debug_trace_call {
            let mut call_options = call_tracer_options();
            if let Some(state_overrides) = self.overrides.get_state_overrides()? {
                call_options = call_options.with_state_overrides(state_overrides);
            }
            if let Some(block_overrides) = self.overrides.get_block_overrides()? {
                call_options = call_options.with_block_overrides(block_overrides);
            }
            ("debug_traceCall", serde_json::json!([call_object, block_param, call_options]))
        } else {
            ("eth_call", serde_json::json!([call_object, block_param]))
        };

        let curl_cmd = generate_curl_command(
            url.as_ref(),
            method,
            params,
            config.eth_rpc_headers.as_deref(),
            jwt.as_deref(),
        )?;

        sh_println!("{}", curl_cmd)?;
        Ok(())
    }
}

fn pin_remote_trace_block(requested: BlockId, fetched_hash: Option<B256>) -> Result<BlockId> {
    if requested.is_pending() {
        return Ok(requested);
    }

    let fetched_hash = fetched_hash.ok_or_else(|| {
        eyre::eyre!("block {requested:?} was not found while preparing the remote trace")
    })?;
    if let BlockId::Hash(requested_hash) = requested {
        if requested_hash.block_hash != fetched_hash {
            eyre::bail!(
                "the RPC endpoint returned block {fetched_hash} for requested block {}; retry the command",
                requested_hash.block_hash
            );
        }
        // Preserve `requireCanonical` exactly as supplied by the caller.
        return Ok(requested);
    }

    Ok(BlockId::hash(fetched_hash))
}

fn ensure_remote_trace_block_is_canonical(
    expected: BlockNumHash,
    actual: Option<BlockNumHash>,
) -> Result<()> {
    let Some(actual) = actual else {
        eyre::bail!(
            "block {} at {} changed canonicality while collecting its remote trace: the canonical block lookup no longer reports that height; retry the command",
            expected.hash,
            expected.number,
        );
    };
    if actual != expected {
        eyre::bail!(
            "block {} at {} changed canonicality while collecting its remote trace: the canonical block lookup reported block {} at {}; retry the command",
            expected.hash,
            expected.number,
            actual.hash,
            actual.number,
        );
    }

    Ok(())
}

impl figment::Provider for CallArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("CallArgs")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut map = Map::new();

        if let Some(evm_version) = self.evm_version {
            map.insert("evm_version".into(), figment::value::Value::serialize(evm_version)?);
        }
        if let Some(chain) = self.chain {
            map.insert("chain_id".into(), chain.id().into());
        }

        Ok(Map::from([(Config::selected_profile(), map)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_eips::RpcBlockHash;

    #[test]
    fn remote_trace_block_pinning() {
        let hash = B256::repeat_byte(0x11);

        // Pending stays unpinned; every other block is pinned to the fetched hash.
        assert_eq!(pin_remote_trace_block(BlockId::pending(), None).unwrap(), BlockId::pending());
        for requested in [
            BlockId::number(42),
            BlockId::earliest(),
            BlockId::latest(),
            BlockId::safe(),
            BlockId::finalized(),
        ] {
            assert_eq!(pin_remote_trace_block(requested, Some(hash)).unwrap(), BlockId::hash(hash));
        }
        let err = pin_remote_trace_block(BlockId::number(42), None).unwrap_err();
        assert!(err.to_string().contains("was not found while preparing the remote trace"));

        // Hash requests keep `requireCanonical` and must match the response.
        for require_canonical in [None, Some(false), Some(true)] {
            let requested = BlockId::Hash(RpcBlockHash { block_hash: hash, require_canonical });
            assert_eq!(pin_remote_trace_block(requested, Some(hash)).unwrap(), requested);
        }
        let err = pin_remote_trace_block(
            BlockId::hash_canonical(B256::repeat_byte(0x33)),
            Some(B256::repeat_byte(0x44)),
        )
        .unwrap_err();
        assert!(err.to_string().contains("returned block"));
    }

    #[test]
    fn remote_trace_block_must_remain_canonical() {
        let expected = BlockNumHash::new(42, B256::repeat_byte(0x55));
        ensure_remote_trace_block_is_canonical(expected, Some(expected)).unwrap();

        let err = ensure_remote_trace_block_is_canonical(expected, None).unwrap_err();
        assert!(err.to_string().contains("no longer reports that height"), "{err}");

        let reorged = BlockNumHash::new(42, B256::repeat_byte(0x66));
        let err = ensure_remote_trace_block_is_canonical(expected, Some(reorged)).unwrap_err();
        assert!(err.to_string().contains("changed canonicality"), "{err}");
    }

    #[test]
    fn chain_is_merged_into_config() {
        let args = CallArgs::parse_from(["foundry-cli", "--chain", "1"]);
        let config = Config::from_provider(Config::figment().merge(&args)).unwrap();

        assert_eq!(config.chain, Some(Chain::mainnet()));
    }

    #[test]
    fn debug_trace_call_ignores_configured_internal_decoding() {
        let args = CallArgs::parse_from(["foundry-cli", "--debug-trace-call"]);
        let config = TracingConfig { decode_internal: true, ..Default::default() };

        assert!(!args.resolve_tracing(&config, 0).decode_internal);
    }
}
