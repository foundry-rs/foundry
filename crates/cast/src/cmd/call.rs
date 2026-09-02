use super::{
    auth::{confirm_auth_rpc_disclosure, confirm_auth_rpc_disclosure_before_network_resolution},
    call_overrides::CallOverrideOpts,
    run::{fetch_contracts_bytecode_from_trace, fetch_contracts_bytecode_via_rpc},
};
use crate::{
    Cast,
    debug::{ensure_remote_trace_context_unchanged, handle_traces, select_remote_trace_hardfork},
    rpc_trace::{call_frame_to_arena, is_method_not_found_error, is_missing_state_error},
    traces::TraceKind,
    tx::{CastTxBuilder, SenderKind},
};
use alloy_consensus::BlockHeader;
use alloy_eips::BlockNumHash;
use alloy_ens::NameOrAddress;
use alloy_network::{
    BlockResponse, NetworkTransactionBuilder, TransactionBuilder, primitives::HeaderResponse,
};
use alloy_primitives::{B256, Bytes, TxKind, U256, hex, map::AddressHashMap};
use alloy_provider::{Provider, ext::DebugApi};
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag, BlockOverrides,
    state::StateOverride,
    trace::geth::{
        CallConfig, GethDebugBuiltInTracerType, GethDebugTracerType, GethDebugTracingCallOptions,
        GethDebugTracingOptions, GethTrace,
    },
};
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{ChainValueParser, RpcOpts, TracingArgs, TransactionOpts},
    utils::{LoadConfig, TraceResult, parse_ether_value},
};
use foundry_common::{
    FoundryTransactionBuilder,
    abi::{encode_function_args, get_func},
    provider::{ProviderBuilder, curl_transport::generate_curl_command},
    sh_println, shell,
};
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{
    Chain, Config, FoundryHardfork, TracingConfig,
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
        evm::{EthEvmNetwork, FoundryEvmNetwork, TempoEvmNetwork, context_for_child_transaction},
    },
    executors::{ExecutorBuilder, TracingExecutor},
    opts::EvmOpts,
    traces::{InternalTraceMode, SparsedTraceArena, TraceRequirements},
};
use foundry_evm_networks::NetworkConfigs;
use foundry_wallets::{BrowserWalletOpts, WalletOpts};
use revm::context::Block;
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

struct AuthDisclosurePreflight {
    confirmed: bool,
    sender: Option<SenderKind<'static>>,
}

fn infer_network_from_chain_id(networks: NetworkConfigs, chain_id: u64) -> Result<NetworkConfigs> {
    networks.try_with_chain_id(chain_id).map_err(eyre::Report::msg)
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
            evm_opts.networks = infer_network_from_chain_id(evm_opts.networks, chain.id())?;
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

        #[cfg(feature = "base")]
        if evm_opts.networks.is_base() {
            return self
                .run_with_network_and_opts::<foundry_evm::core::evm::BaseEvmNetwork>(
                    config,
                    evm_opts,
                    auth_preflight,
                    ExecutorBuilder::<foundry_evm::core::evm::BaseEvmNetwork>::new(),
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

    async fn preflight_auth_disclosure(&self) -> Result<Option<AuthDisclosurePreflight>> {
        if !self.will_disclose_auth() {
            return Ok(Some(AuthDisclosurePreflight { confirmed: false, sender: None }));
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

        Ok(Some(AuthDisclosurePreflight { confirmed: true, sender }))
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
        auth_preflight: AuthDisclosurePreflight,
        executor_builder: ExecutorBuilder<FEN>,
    ) -> Result<()> {
        config.networks = evm_opts.networks;
        let mut state_overrides = self.get_state_overrides()?;
        let block_overrides = self.get_block_overrides()?;
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
        let sender = if let Some(sender) = auth_preflight.sender {
            sender
        } else if let Some(browser) = browser.run::<FEN::Network>().await? {
            browser.address().into()
        } else {
            SenderKind::from_wallet_opts(wallet).await?
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
            && !auth_preflight.confirmed
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
                .as_ref()
                .ok_or_else(|| eyre::eyre!("remote trace endpoint identity was not captured"))?;
            let requested_block = block.unwrap_or(BlockId::latest());
            let fetched_block = provider.get_block(requested_block).await?;
            let resolved_canonical_block =
                if matches!(requested_block, BlockId::Number(_)) && !requested_block.is_pending() {
                    fetched_block.as_ref().map(|block| {
                        BlockNumHash::new(block.header().number(), block.header().hash())
                    })
                } else {
                    None
                };
            let block = pin_remote_trace_block(
                requested_block,
                fetched_block.as_ref().map(|block| block.header().hash()),
            )?;
            let block_time_override = block_overrides.as_ref().and_then(|overrides| overrides.time);
            let mut call_options = GethDebugTracingCallOptions::default().with_tracing_options(
                GethDebugTracingOptions::default()
                    .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                    .with_call_config(CallConfig::default().with_log()),
            );
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

            let geth_trace = provider
                .debug_trace_call(tx, block, call_options)
                .await
                .map_err(|err| -> eyre::Report {
                    // Two RPC rejections deserve an actionable hint instead of the raw transport
                    // error, and they need different fixes: a disabled `debug` namespace, and
                    // missing historical state, hit whenever `--block` targets a block a full
                    // node has pruned.
                    if is_method_not_found_error(&err) {
                        eyre::eyre!(
                            "the RPC endpoint does not support `debug_traceCall` (method not found); use a node with the `debug` namespace enabled (e.g. a local anvil/reth or an archive endpoint), or drop `--debug-trace-call` to run the call locally with `--trace`"
                        )
                    } else if is_missing_state_error(&err) {
                        eyre::eyre!(
                            "the RPC endpoint does not have the historical state for the requested block; use an archive endpoint, or target a more recent block with `--block`"
                        )
                    } else {
                        err.into()
                    }
                })?;
            let GethTrace::CallTracer(frame) = geth_trace else {
                eyre::bail!(
                    "`debug_traceCall` did not return a callTracer frame; the RPC endpoint may not \
                     support the `callTracer`"
                );
            };

            let success = frame.error.is_none() && frame.revert_reason.is_none();
            let gas_used = frame.gas_used.saturating_to();
            let arena = SparsedTraceArena {
                arena: call_frame_to_arena(&frame),
                ignored: Default::default(),
                diagnostics: Default::default(),
            };
            let result = TraceResult {
                success,
                traces: Some(vec![(TraceKind::Execution, arena)]),
                gas_used,
            };

            // Local-artifact labeling matches deployed runtime bytecode against the
            // project artifacts. There is no local executor on this path, so fetch the code
            // over RPC for the addresses in the trace. Skip the extra round-trips unless
            // local artifacts were requested.
            let contracts_bytecode = if with_local_artifacts {
                let mut contracts_bytecode =
                    fetch_contracts_bytecode_via_rpc(&provider, &result, block).await?;
                // The trace ran the override code, not the on-chain code, so the override
                // wins for artifact matching.
                contracts_bytecode.extend(override_bytecode);
                contracts_bytecode
            } else {
                Default::default()
            };
            let final_endpoint_identity = evm_opts.discover_fork_endpoint().await?;
            ensure_remote_trace_context_unchanged(endpoint_identity, &final_endpoint_identity)?;

            // The remote node executed this trace, so its reported family is authoritative for
            // decoding even when the caller selected a compatible local EVM implementation.
            let execution_network = endpoint_identity.network;
            let chain = alloy_chains::Chain::from_id(endpoint_identity.source_chain_id);
            let block_timestamp = if let Some(timestamp) = block_time_override {
                Some(timestamp)
            } else {
                fetched_block.as_ref().map(|block| block.header().timestamp())
            };
            // A configured hardfork is an explicit trace-decoding override. Otherwise honor an
            // Anvil endpoint's exact execution hardfork before consulting the source schedule.
            let resolved_hardfork = select_remote_trace_hardfork(
                config.hardfork,
                endpoint_identity.hardfork,
                execution_network,
            )
            .or_else(|| {
                block_timestamp.and_then(|timestamp| {
                    FoundryHardfork::from_chain_and_timestamp(chain.id(), timestamp)
                })
            });
            if let Some(resolved_block) = resolved_canonical_block {
                let canonical_block =
                    provider.get_block_by_number(resolved_block.number.into()).await?;
                ensure_remote_trace_block_is_canonical(
                    resolved_block,
                    canonical_block.as_ref().map(|block| {
                        BlockNumHash::new(block.header().number(), block.header().hash())
                    }),
                )?;
            }
            handle_traces(
                result,
                &config,
                chain,
                &contracts_bytecode,
                &tracing,
                with_local_artifacts,
                false,
                resolved_hardfork,
                endpoint_identity.network_profile,
            )
            .await?;

            return Ok(());
        }

        if trace {
            if let Some(BlockId::Number(BlockNumberOrTag::Number(block_number))) = block {
                // Override Config `fork_block_number` (if set) with CLI value.
                config.fork_block_number = Some(block_number);
            }

            let create2_deployer = evm_opts.create2_deployer;
            let verbosity = tracing.verbosity;
            let (mut evm_env, tx_env, fork, chain, networks, endpoint_hardfork) =
                TracingExecutor::<FEN>::get_fork_material(&mut config, evm_opts).await?;
            let context_block_number = evm_env.block_env.number().saturating_to();
            // Modify settings usually set in eth_call while keeping execution gas bounded.
            evm_env.cfg_env.disable_block_gas_limit = true;
            evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);

            // Apply the block overrides.
            if let Some(block_overrides) = block_overrides {
                if let Some(number) = block_overrides.number {
                    evm_env.block_env.set_number(number.to());
                }
                if let Some(time) = block_overrides.time {
                    evm_env.block_env.set_timestamp(U256::from(time));
                }
            }
            let resolved_hardfork = TracingExecutor::<FEN>::resolve_spec_for_chain(
                &config,
                networks,
                chain.id(),
                endpoint_hardfork,
                &mut evm_env,
                evm_version,
            );
            TracingExecutor::<FEN>::extend_precompile_labels(
                &mut config,
                networks,
                resolved_hardfork,
            );

            let trace_requirements = TraceRequirements::none()
                .with_calls(true)
                .with_debug(debug)
                .with_decode_internal(if tracing.decode_internal {
                    InternalTraceMode::Full
                } else {
                    InternalTraceMode::None
                })
                .with_state_changes(verbosity > 4);
            let mut executor = TracingExecutor::<FEN>::new(
                executor_builder,
                (evm_env, tx_env),
                fork,
                None,
                trace_requirements,
                networks,
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

            let env_tx = executor.tx_env_mut();

            // Set transaction options with --trace
            if let Some(gas_price) = tx.gas_price() {
                env_tx.set_gas_price(gas_price);
            }

            if let Some(max_fee_per_gas) = tx.max_fee_per_gas() {
                env_tx.set_gas_price(max_fee_per_gas);
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

            let mut context_tx = executor.tx_env().clone();
            context_tx.set_caller(from);
            context_tx.set_kind(tx_kind);
            context_tx.set_data(input.clone());
            context_tx.set_value(value);
            let chain_context = context_for_child_transaction::<FEN, _>(
                &provider,
                context_block_number,
                &context_tx,
                networks,
            )
            .await?;

            let trace = match tx_kind {
                TxKind::Create => {
                    let deploy_result =
                        executor.deploy_with_context(from, input, value, chain_context, None);
                    TraceResult::try_from(deploy_result)?
                }
                TxKind::Call(to) => TraceResult::from_raw(
                    executor.transact_raw_with_context(from, to, input, value, chain_context)?,
                    TraceKind::Execution,
                ),
            };

            let contracts_bytecode = fetch_contracts_bytecode_from_trace(&executor, &trace)?;
            handle_traces(
                trace,
                &config,
                chain,
                &contracts_bytecode,
                &tracing,
                with_local_artifacts,
                debug,
                resolved_hardfork,
                networks,
            )
            .await?;

            return Ok(());
        }

        let response = Cast::new(&provider)
            .call(&tx, func.as_ref(), block, state_overrides, block_overrides)
            .await?;

        // With `--delegate` the call targets the sender, whose code comes from the override and
        // was already checked to be non-empty, so the on-chain code lookup would be misleading.
        if response == "0x"
            && !delegate
            && let Some(contract_address) = tx.to()
        {
            let code = provider.get_code_at(contract_address).await?;
            if code.is_empty() {
                sh_warn!("Contract code is empty")?;
            }
        }

        // Bypass the shell verbosity layer so `--quiet` does not suppress the primary result.
        let mut shell = shell::Shell::get();
        let out = shell.out();
        writeln!(out, "{response}")?;
        out.flush()?;

        Ok(())
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
            if let Ok(data) = hex::decode(sig) {
                data
            } else {
                // Parse function signature and encode args
                let func = get_func(sig)?;
                encode_function_args(&func, &self.args)?
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
            let mut call_options = GethDebugTracingCallOptions::default().with_tracing_options(
                GethDebugTracingOptions::default()
                    .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                    .with_call_config(CallConfig::default().with_log()),
            );
            if let Some(state_overrides) = self.get_state_overrides()? {
                call_options = call_options.with_state_overrides(state_overrides);
            }
            if let Some(block_overrides) = self.get_block_overrides()? {
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

    /// Parses state overrides from command line arguments.
    pub fn get_state_overrides(&self) -> Result<Option<StateOverride>> {
        self.overrides.get_state_overrides()
    }

    /// Parses block overrides from command line arguments.
    pub fn get_block_overrides(&self) -> Result<Option<BlockOverrides>> {
        self.overrides.get_block_overrides()
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
    use alloy_primitives::U64;

    #[test]
    fn pending_remote_trace_block_remains_unpinned() {
        assert_eq!(pin_remote_trace_block(BlockId::pending(), None).unwrap(), BlockId::pending());
    }

    #[test]
    fn non_pending_remote_trace_blocks_are_pinned() {
        let hash = B256::repeat_byte(0x11);
        for requested in [
            BlockId::number(42),
            BlockId::earliest(),
            BlockId::latest(),
            BlockId::safe(),
            BlockId::finalized(),
        ] {
            assert_eq!(pin_remote_trace_block(requested, Some(hash)).unwrap(), BlockId::hash(hash));
        }
    }

    #[test]
    fn non_pending_remote_trace_block_must_exist() {
        let err = pin_remote_trace_block(BlockId::number(42), None).unwrap_err();
        assert!(err.to_string().contains("was not found while preparing the remote trace"));
    }

    #[test]
    fn hash_remote_trace_block_preserves_canonical_requirement() {
        let hash = B256::repeat_byte(0x22);
        for require_canonical in [None, Some(false), Some(true)] {
            let requested = BlockId::Hash(RpcBlockHash { block_hash: hash, require_canonical });
            assert_eq!(pin_remote_trace_block(requested, Some(hash)).unwrap(), requested);
        }
    }

    #[test]
    fn hash_remote_trace_block_must_match_response() {
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
        assert!(err.to_string().contains("no longer reports that height"));

        let err = ensure_remote_trace_block_is_canonical(
            expected,
            Some(BlockNumHash::new(expected.number, B256::repeat_byte(0x66))),
        )
        .unwrap_err();
        assert!(err.to_string().contains("changed canonicality"));
    }

    #[test]
    fn can_parse_call_data() {
        let data = hex::encode("hello");
        let args = CallArgs::parse_from(["foundry-cli", "--data", data.as_str()]);
        assert_eq!(args.data, Some(data));

        let data = hex::encode_prefixed("hello");
        let args = CallArgs::parse_from(["foundry-cli", "--data", data.as_str()]);
        assert_eq!(args.data, Some(data));
    }

    #[test]
    fn chain_is_merged_into_config() {
        let args = CallArgs::parse_from(["foundry-cli", "--chain", "1"]);
        let config = Config::from_provider(Config::figment().merge(&args)).unwrap();

        assert_eq!(config.chain, Some(Chain::mainnet()));
    }

    /// Base chain IDs resolved to Optimism before Base support existed, so a build without the
    /// `base` feature — which is what release binaries ship — must keep resolving them that way.
    #[test]
    #[cfg(all(not(feature = "base"), feature = "optimism"))]
    fn chain_id_without_base_still_resolves_to_optimism() {
        for chain_id in [8453, 84532] {
            let networks = infer_network_from_chain_id(NetworkConfigs::default(), chain_id)
                .unwrap_or_else(|error| panic!("chain ID {chain_id} must still resolve: {error}"));
            assert!(networks.is_optimism(), "chain ID {chain_id} must resolve to Optimism");
        }
    }

    #[test]
    #[cfg(not(feature = "monad"))]
    fn chain_id_rejects_disabled_monad_network() {
        let error = infer_network_from_chain_id(NetworkConfigs::default(), 143).unwrap_err();

        assert_eq!(
            error.to_string(),
            "cannot infer execution network from chain ID 143: network family `monad` is not \
             enabled in this build"
        );
    }

    #[test]
    fn explicit_ethereum_overrides_chain_id_inference() {
        let ethereum = NetworkConfigs::with_ethereum();
        for chain_id in [8453, 143] {
            assert_eq!(infer_network_from_chain_id(ethereum, chain_id).unwrap(), ethereum);
        }
    }

    #[test]
    fn can_parse_state_overrides() {
        let args = CallArgs::parse_from([
            "foundry-cli",
            "--override-balance",
            "0x123:0x1234",
            "--override-nonce",
            "0x123:1",
            "--override-code",
            "0x123:0x1234",
            "--override-state",
            "0x123:0x1:0x1234",
        ]);

        assert_eq!(args.overrides.balance_overrides, Some(vec!["0x123:0x1234".to_string()]));
        assert_eq!(args.overrides.nonce_overrides, Some(vec!["0x123:1".to_string()]));
        assert_eq!(args.overrides.code_overrides, Some(vec!["0x123:0x1234".to_string()]));
        assert_eq!(args.overrides.state_overrides, Some(vec!["0x123:0x1:0x1234".to_string()]));
    }

    #[test]
    fn can_parse_multiple_state_overrides() {
        let args = CallArgs::parse_from([
            "foundry-cli",
            "--override-balance",
            "0x123:0x1234",
            "--override-balance",
            "0x456:0x5678",
            "--override-nonce",
            "0x123:1",
            "--override-nonce",
            "0x456:2",
            "--override-code",
            "0x123:0x1234",
            "--override-code",
            "0x456:0x5678",
            "--override-state",
            "0x123:0x1:0x1234",
            "--override-state",
            "0x456:0x2:0x5678",
        ]);

        assert_eq!(
            args.overrides.balance_overrides,
            Some(vec!["0x123:0x1234".to_string(), "0x456:0x5678".to_string()])
        );
        assert_eq!(
            args.overrides.nonce_overrides,
            Some(vec!["0x123:1".to_string(), "0x456:2".to_string()])
        );
        assert_eq!(
            args.overrides.code_overrides,
            Some(vec!["0x123:0x1234".to_string(), "0x456:0x5678".to_string()])
        );
        assert_eq!(
            args.overrides.state_overrides,
            Some(vec!["0x123:0x1:0x1234".to_string(), "0x456:0x2:0x5678".to_string()])
        );
    }

    #[test]
    fn test_negative_args_with_flags() {
        // Test that negative args work with flags
        let args = CallArgs::parse_from([
            "foundry-cli",
            "--trace",
            "0xDeaDBeeFcAfEbAbEfAcEfEeDcBaDbEeFcAfEbAbE",
            "process(int256)",
            "-999999",
            "--debug",
        ]);

        assert!(args.trace);
        assert!(args.debug);
        assert_eq!(args.args, vec!["-999999"]);
    }

    #[test]
    fn test_transaction_opts_with_trace() {
        // Test that transaction options are correctly parsed when using --trace
        let args = CallArgs::parse_from([
            "foundry-cli",
            "--trace",
            "--gas-limit",
            "1000000",
            "--gas-price",
            "20000000000",
            "--priority-gas-price",
            "2000000000",
            "--nonce",
            "42",
            "--value",
            "1000000000000000000", // 1 ETH
            "--blob-gas-price",
            "10000000000",
            "0xDeaDBeeFcAfEbAbEfAcEfEeDcBaDbEeFcAfEbAbE",
            "balanceOf(address)",
            "0x123456789abcdef123456789abcdef123456789a",
        ]);

        assert!(args.trace);
        assert_eq!(args.tx.gas_limit, Some(U256::from(1000000u32)));
        assert_eq!(args.tx.gas_price, Some(U256::from(20000000000u64)));
        assert_eq!(args.tx.priority_gas_price, Some(U256::from(2000000000u64)));
        assert_eq!(args.tx.nonce, Some(U64::from(42)));
        assert_eq!(args.tx.value, Some(U256::from(1000000000000000000u64)));
        assert_eq!(args.tx.blob_gas_price, Some(U256::from(10000000000u64)));
    }

    #[test]
    fn debug_trace_call_conflicts_with_trace() {
        let result = CallArgs::try_parse_from(["foundry-cli", "--trace", "--debug-trace-call"]);
        assert!(result.is_err(), "--trace and --debug-trace-call must be mutually exclusive");
    }

    #[test]
    fn debug_trace_call_rejects_local_trace_flags() {
        for flag in ["--debug", "--decode-internal"] {
            let result = CallArgs::try_parse_from([
                "foundry-cli",
                "--debug-trace-call",
                "0xDeaDBeeFcAfEbAbEfAcEfEeDcBaDbEeFcAfEbAbE",
                flag,
            ]);
            assert!(result.is_err(), "--debug-trace-call must reject {flag}");
        }
        // --evm-version takes a value, so it is checked separately from the boolean flags above.
        let result = CallArgs::try_parse_from([
            "foundry-cli",
            "--debug-trace-call",
            "0xDeaDBeeFcAfEbAbEfAcEfEeDcBaDbEeFcAfEbAbE",
            "--evm-version",
            "shanghai",
        ]);
        assert!(result.is_err(), "--debug-trace-call must reject --evm-version");
    }

    #[test]
    fn debug_trace_call_ignores_configured_internal_decoding() {
        let args = CallArgs::parse_from(["foundry-cli", "--debug-trace-call"]);
        let config = TracingConfig { decode_internal: true, ..Default::default() };

        assert!(!args.resolve_tracing(&config, 0).decode_internal);
    }
}
