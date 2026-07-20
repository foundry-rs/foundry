use super::{
    call_overrides::CallOverrideOpts,
    run::{fetch_contracts_bytecode_from_trace, fetch_contracts_bytecode_via_rpc},
};
use crate::{
    Cast,
    debug::handle_traces,
    rpc_trace::{call_frame_to_arena, is_method_not_found_error, is_missing_state_error},
    traces::TraceKind,
    tx::{CastTxBuilder, SenderKind},
};
use alloy_ens::NameOrAddress;
use alloy_network::{Network, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Bytes, TxKind, U256, hex, map::AddressHashMap};
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
    opts::{ChainValueParser, RpcOpts, TransactionOpts},
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
    Chain, Config,
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
        evm::{EthEvmNetwork, FoundryEvmNetwork, TempoEvmNetwork},
    },
    executors::TracingExecutor,
    opts::EvmOpts,
    traces::{InternalTraceMode, SparsedTraceArena, TraceRequirements},
};
use foundry_wallets::WalletOpts;
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

    /// Raw hex-encoded data for the transaction. Used instead of \[SIG\] and \[ARGS\].
    #[arg(
        long,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    /// Forks the remote rpc, executes the transaction locally and prints a trace
    #[arg(long, default_value_t = false)]
    trace: bool,

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

    /// Disables the labels in the traces.
    /// Can only be set with `--trace` or `--debug-trace-call`.
    #[arg(long, default_value_t = false, requires = "trace")]
    disable_labels: bool,

    /// Opens an interactive debugger.
    /// Can only be used with `--trace`.
    #[arg(long, requires = "trace")]
    debug: bool,

    /// Identify internal functions in traces.
    ///
    /// This will trace internal functions and decode stack parameters.
    ///
    /// Parameters stored in memory (such as bytes or arrays) are currently decoded only when a
    /// single function is matched, similarly to `--debug`, for performance reasons.
    #[arg(long, requires = "trace")]
    decode_internal: bool,

    /// Labels to apply to the traces; format: `address:label`.
    /// Can only be used with `--trace` or `--debug-trace-call`.
    #[arg(long, requires = "trace")]
    labels: Vec<String>,

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

    #[command(flatten)]
    rpc: RpcOpts,

    #[command(flatten)]
    wallet: WalletOpts,

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

impl CallArgs {
    pub async fn run(mut self) -> Result<()> {
        // Handle --curl mode early, before any provider interaction
        if self.rpc.curl {
            return self.run_curl().await;
        }

        if self.tx.tempo.is_tempo() {
            return self.run_with_network::<TempoEvmNetwork>().await;
        }

        let figment = self.rpc.clone().into_figment(self.with_local_artifacts).merge(&self);
        let mut evm_opts = figment.extract::<EvmOpts>()?;
        if let Some(chain) = self.chain {
            evm_opts.networks = evm_opts.networks.with_chain_id(chain.id());
        }
        evm_opts.infer_network_from_fork().await;
        if self.chain.is_none() {
            self.chain = evm_opts.env.chain_id.map(Chain::from_id);
        }

        if evm_opts.networks.is_tempo() {
            return self.run_with_network::<TempoEvmNetwork>().await;
        }

        #[cfg(feature = "monad")]
        if evm_opts.networks.is_monad() {
            return self.run_with_network::<MonadEvmNetwork>().await;
        }

        #[cfg(feature = "optimism")]
        if evm_opts.networks.is_optimism() {
            return self.run_with_network::<OpEvmNetwork>().await;
        }

        self.run_with_network::<EthEvmNetwork>().await
    }

    pub async fn run_with_network<FEN: FoundryEvmNetwork>(self) -> Result<()>
    where
        <FEN::Network as Network>::TransactionRequest: FoundryTransactionBuilder<FEN::Network>,
    {
        let figment = self.rpc.clone().into_figment(self.with_local_artifacts).merge(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut config = Config::from_provider(figment)?.sanitized();
        let state_overrides = self.get_state_overrides()?;
        let block_overrides = self.get_block_overrides()?;

        let Self {
            to,
            mut sig,
            mut args,
            mut tx,
            command,
            block,
            trace,
            evm_version,
            debug,
            decode_internal,
            labels,
            data,
            with_local_artifacts,
            disable_labels,
            wallet,
            ..
        } = self;

        if let Some(data) = data {
            sig = Some(data);
        }

        let provider = ProviderBuilder::<FEN::Network>::from_config(&config)?.build()?;
        let sender = SenderKind::from_wallet_opts(wallet).await?;
        let from = sender.address();

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

        let (tx, func) = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .raw()
            .build(sender)
            .await?;

        if self.debug_trace_call {
            let block = self.block.unwrap_or(BlockId::latest());
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

            let chain = alloy_chains::Chain::from_id(provider.get_chain_id().await?);
            handle_traces(
                result,
                &config,
                chain,
                &contracts_bytecode,
                labels,
                with_local_artifacts,
                false,
                false,
                disable_labels,
                None,
                None,
            )
            .await?;

            return Ok(());
        }

        if trace {
            if let Some(BlockId::Number(BlockNumberOrTag::Number(block_number))) = self.block {
                // Override Config `fork_block_number` (if set) with CLI value.
                config.fork_block_number = Some(block_number);
            }

            let create2_deployer = evm_opts.create2_deployer;
            let (mut evm_env, tx_env, fork, chain, networks) =
                TracingExecutor::<FEN>::get_fork_material(&mut config, evm_opts).await?;

            // modify settings that usually set in eth_call
            evm_env.cfg_env.disable_block_gas_limit = true;
            evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);
            evm_env.block_env.set_gas_limit(u64::MAX);

            // Apply the block overrides.
            if let Some(block_overrides) = block_overrides {
                if let Some(number) = block_overrides.number {
                    evm_env.block_env.set_number(number.to());
                }
                if let Some(time) = block_overrides.time {
                    evm_env.block_env.set_timestamp(U256::from(time));
                }
            }

            let trace_requirements = TraceRequirements::none()
                .with_calls(true)
                .with_debug(debug)
                .with_decode_internal(if decode_internal {
                    InternalTraceMode::Full
                } else {
                    InternalTraceMode::None
                })
                .with_state_changes(shell::verbosity() > 4);
            let mut executor = TracingExecutor::<FEN>::new(
                (evm_env, tx_env),
                fork,
                evm_version,
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
            // effect; writing it onto the tx env directly would be overwritten. When no limit is
            // given, the executor keeps the block gas limit (`u64::MAX`) set above.
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
            handle_traces(
                trace,
                &config,
                chain,
                &contracts_bytecode,
                labels,
                with_local_artifacts,
                debug,
                decode_internal,
                disable_labels,
                None,
                None,
            )
            .await?;

            return Ok(());
        }

        let response = Cast::new(&provider)
            .call(&tx, func.as_ref(), block, state_overrides, block_overrides)
            .await?;

        if response == "0x"
            && let Some(contract_address) = tx.to()
        {
            let code = provider.get_code_at(contract_address).await?;
            if code.is_empty() {
                sh_warn!("Contract code is empty")?;
            }
        }

        sh_println!("{}", response)?;

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
    use alloy_primitives::U64;

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
}
