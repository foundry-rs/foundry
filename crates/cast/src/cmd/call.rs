use crate::{
    Cast,
    traces::TraceKind,
    tx::{CastTxBuilder, SenderKind},
};
use alloy_ens::NameOrAddress;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag, BlockOverrides,
    state::{StateOverride, StateOverridesBuilder},
};
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, TraceResult, handle_traces, parse_ether_value},
};
use foundry_common::shell;
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{
    Config,
    figment::{
        self, Metadata, Profile,
        value::{Dict, Map},
    },
};
use foundry_evm::{
    executors::TracingExecutor,
    opts::EvmOpts,
    traces::{InternalTraceMode, TraceMode},
};
use itertools::Either;
use regex::Regex;
use revm::context::TransactionType;
use std::{str::FromStr, sync::LazyLock};

use super::run::fetch_contracts_bytecode_from_trace;

// matches override pattern <address>:<slot>:<value>
// e.g. 0x123:0x1:0x1234
static OVERRIDE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^:]+):([^:]+):([^:]+)$").unwrap());

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

    /// Disables the labels in the traces.
    /// Can only be set with `--trace`.
    #[arg(long, default_value_t = false, requires = "trace")]
    disable_labels: bool,

    /// Opens an interactive debugger.
    /// Can only be used with `--trace`.
    #[arg(long, requires = "trace")]
    debug: bool,

    #[arg(long, requires = "trace")]
    decode_internal: bool,

    /// Labels to apply to the traces; format: `address:label`.
    /// Can only be used with `--trace`.
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

    /// Enable Odyssey features.
    #[arg(long, alias = "alphanet")]
    pub odyssey: bool,

    #[command(subcommand)]
    command: Option<CallSubcommands>,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,

    /// Use current project artifacts for trace decoding.
    #[arg(long, visible_alias = "la")]
    pub with_local_artifacts: bool,

    /// Override the balance of an account.
    /// Format: address:balance
    #[arg(long = "override-balance", value_name = "ADDRESS:BALANCE")]
    pub balance_overrides: Option<Vec<String>>,

    /// Override the nonce of an account.
    /// Format: address:nonce
    #[arg(long = "override-nonce", value_name = "ADDRESS:NONCE")]
    pub nonce_overrides: Option<Vec<String>>,

    /// Override the code of an account.
    /// Format: address:code
    #[arg(long = "override-code", value_name = "ADDRESS:CODE")]
    pub code_overrides: Option<Vec<String>>,

    /// Override the state of an account.
    /// Format: address:slot:value
    #[arg(long = "override-state", value_name = "ADDRESS:SLOT:VALUE")]
    pub state_overrides: Option<Vec<String>>,

    /// Override the state diff of an account.
    /// Format: address:slot:value
    #[arg(long = "override-state-diff", value_name = "ADDRESS:SLOT:VALUE")]
    pub state_diff_overrides: Option<Vec<String>>,

    /// Override the block timestamp.
    #[arg(long = "block.time", value_name = "TIME")]
    pub block_time: Option<u64>,

    /// Override the block number.
    #[arg(long = "block.number", value_name = "NUMBER")]
    pub block_number: Option<u64>,
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
    pub async fn run(self) -> Result<()> {
        let figment = self.eth.rpc.clone().into_figment(self.with_local_artifacts).merge(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut config = Config::from_provider(figment)?.sanitized();
        let state_overrides = self.get_state_overrides()?;
        let block_overrides = self.get_block_overrides()?;

        let Self {
            to,
            mut sig,
            mut args,
            mut tx,
            eth,
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
            ..
        } = self;

        if let Some(data) = data {
            sig = Some(data);
        }

        let provider = utils::get_provider(&config)?;
        let sender = SenderKind::from_wallet_opts(eth.wallet).await?;
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
            .build_raw(sender)
            .await?;

        if trace {
            if let Some(BlockId::Number(BlockNumberOrTag::Number(block_number))) = self.block {
                // Override Config `fork_block_number` (if set) with CLI value.
                config.fork_block_number = Some(block_number);
            }

            let create2_deployer = evm_opts.create2_deployer;
            let (mut env, fork, chain, odyssey) =
                TracingExecutor::get_fork_material(&config, evm_opts).await?;

            // modify settings that usually set in eth_call
            env.evm_env.cfg_env.disable_block_gas_limit = true;
            env.evm_env.block_env.gas_limit = u64::MAX;

            // Apply the block overrides.
            if let Some(block_overrides) = block_overrides {
                if let Some(number) = block_overrides.number {
                    env.evm_env.block_env.number = number.to();
                }
                if let Some(time) = block_overrides.time {
                    env.evm_env.block_env.timestamp = U256::from(time);
                }
            }

            let trace_mode = TraceMode::Call
                .with_debug(debug)
                .with_decode_internal(if decode_internal {
                    InternalTraceMode::Full
                } else {
                    InternalTraceMode::None
                })
                .with_state_changes(shell::verbosity() > 4);
            let mut executor = TracingExecutor::new(
                env,
                fork,
                evm_version,
                trace_mode,
                odyssey,
                create2_deployer,
                state_overrides,
            )?;

            let value = tx.value.unwrap_or_default();
            let input = tx.inner.input.into_input().unwrap_or_default();
            let tx_kind = tx.inner.to.expect("set by builder");
            let env_tx = &mut executor.env_mut().tx;

            if let Some(tx_type) = tx.inner.transaction_type {
                env_tx.tx_type = tx_type;
            }

            if let Some(access_list) = tx.inner.access_list {
                env_tx.access_list = access_list;

                if env_tx.tx_type == TransactionType::Legacy as u8 {
                    env_tx.tx_type = TransactionType::Eip2930 as u8;
                }
            }

            if let Some(auth) = tx.inner.authorization_list {
                env_tx.authorization_list = auth.into_iter().map(Either::Left).collect();

                env_tx.tx_type = TransactionType::Eip7702 as u8;
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

            let contracts_bytecode = fetch_contracts_bytecode_from_trace(&provider, &trace).await?;
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
            )
            .await?;

            return Ok(());
        }

        let response = Cast::new(&provider)
            .call(&tx, func.as_ref(), block, state_overrides, block_overrides)
            .await?;

        if response == "0x"
            && let Some(contract_address) = tx.to.and_then(|tx_kind| tx_kind.into_to())
        {
            let code = provider.get_code_at(contract_address).await?;
            if code.is_empty() {
                sh_warn!("Contract code is empty")?;
            }
        }
        sh_println!("{}", response)?;

        Ok(())
    }

    /// Parse state overrides from command line arguments.
    pub fn get_state_overrides(&self) -> eyre::Result<Option<StateOverride>> {
        // Early return if no override set - <https://github.com/foundry-rs/foundry/issues/10705>
        if [
            self.balance_overrides.as_ref(),
            self.nonce_overrides.as_ref(),
            self.code_overrides.as_ref(),
            self.state_overrides.as_ref(),
            self.state_diff_overrides.as_ref(),
        ]
        .iter()
        .all(Option::is_none)
        {
            return Ok(None);
        }

        let mut state_overrides_builder = StateOverridesBuilder::default();

        // Parse balance overrides
        for override_str in self.balance_overrides.iter().flatten() {
            let (addr, balance) = address_value_override(override_str)?;
            state_overrides_builder =
                state_overrides_builder.with_balance(addr.parse()?, balance.parse()?);
        }

        // Parse nonce overrides
        for override_str in self.nonce_overrides.iter().flatten() {
            let (addr, nonce) = address_value_override(override_str)?;
            state_overrides_builder =
                state_overrides_builder.with_nonce(addr.parse()?, nonce.parse()?);
        }

        // Parse code overrides
        for override_str in self.code_overrides.iter().flatten() {
            let (addr, code_str) = address_value_override(override_str)?;
            state_overrides_builder =
                state_overrides_builder.with_code(addr.parse()?, Bytes::from_str(code_str)?);
        }

        // Parse state overrides
        for override_str in self.state_overrides.iter().flatten() {
            let (addr, slot, value) = address_slot_value_override(override_str)?;
            state_overrides_builder =
                state_overrides_builder.with_state(addr, [(slot.into(), value.into())]);
        }

        // Parse state diff overrides
        for override_str in self.state_diff_overrides.iter().flatten() {
            let (addr, slot, value) = address_slot_value_override(override_str)?;
            state_overrides_builder =
                state_overrides_builder.with_state_diff(addr, [(slot.into(), value.into())]);
        }

        Ok(Some(state_overrides_builder.build()))
    }

    /// Parse block overrides from command line arguments.
    pub fn get_block_overrides(&self) -> eyre::Result<Option<BlockOverrides>> {
        let mut overrides = BlockOverrides::default();
        if let Some(number) = self.block_number {
            overrides = overrides.with_number(U256::from(number));
        }
        if let Some(time) = self.block_time {
            overrides = overrides.with_time(time);
        }
        if overrides.is_empty() { Ok(None) } else { Ok(Some(overrides)) }
    }
}

impl figment::Provider for CallArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("CallArgs")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut map = Map::new();

        if self.odyssey {
            map.insert("odyssey".into(), self.odyssey.into());
        }

        if let Some(evm_version) = self.evm_version {
            map.insert("evm_version".into(), figment::value::Value::serialize(evm_version)?);
        }

        Ok(Map::from([(Config::selected_profile(), map)]))
    }
}

/// Parse an override string in the format address:value.
fn address_value_override(address_override: &str) -> Result<(&str, &str)> {
    address_override.split_once(':').ok_or_else(|| {
        eyre::eyre!("Invalid override {address_override}. Expected <address>:<value>")
    })
}

/// Parse an override string in the format address:slot:value.
fn address_slot_value_override(address_override: &str) -> Result<(Address, U256, U256)> {
    let captures = OVERRIDE_PATTERN.captures(address_override).ok_or_else(|| {
        eyre::eyre!("Invalid override {address_override}. Expected <address>:<slot>:<value>")
    })?;

    Ok((
        captures[1].parse()?, // Address
        captures[2].parse()?, // Slot (U256)
        captures[3].parse()?, // Value (U256)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, b256, fixed_bytes, hex};

    #[test]
    fn test_get_state_overrides() {
        let call_args = CallArgs::parse_from([
            "foundry-cli",
            "--override-balance",
            "0x0000000000000000000000000000000000000001:2",
            "--override-nonce",
            "0x0000000000000000000000000000000000000001:3",
            "--override-code",
            "0x0000000000000000000000000000000000000001:0x04",
            "--override-state",
            "0x0000000000000000000000000000000000000001:5:6",
            "--override-state-diff",
            "0x0000000000000000000000000000000000000001:7:8",
        ]);
        let overrides = call_args.get_state_overrides().unwrap().unwrap();
        let address = address!("0x0000000000000000000000000000000000000001");
        if let Some(account_override) = overrides.get(&address) {
            if let Some(balance) = account_override.balance {
                assert_eq!(balance, U256::from(2));
            }
            if let Some(nonce) = account_override.nonce {
                assert_eq!(nonce, 3);
            }
            if let Some(code) = &account_override.code {
                assert_eq!(*code, Bytes::from([0x04]));
            }
            if let Some(state) = &account_override.state
                && let Some(value) = state.get(&b256!(
                    "0x0000000000000000000000000000000000000000000000000000000000000005"
                ))
            {
                assert_eq!(
                    *value,
                    b256!("0x0000000000000000000000000000000000000000000000000000000000000006")
                );
            }
            if let Some(state_diff) = &account_override.state_diff
                && let Some(value) = state_diff.get(&b256!(
                    "0x0000000000000000000000000000000000000000000000000000000000000007"
                ))
            {
                assert_eq!(
                    *value,
                    b256!("0x0000000000000000000000000000000000000000000000000000000000000008")
                );
            }
        }
    }

    #[test]
    fn test_get_state_overrides_empty() {
        let call_args = CallArgs::parse_from([""]);
        let overrides = call_args.get_state_overrides().unwrap();
        assert_eq!(overrides, None);
    }

    #[test]
    fn test_get_block_overrides() {
        let mut call_args = CallArgs::parse_from([""]);
        call_args.block_number = Some(1);
        call_args.block_time = Some(2);
        let overrides = call_args.get_block_overrides().unwrap().unwrap();
        assert_eq!(overrides.number, Some(U256::from(1)));
        assert_eq!(overrides.time, Some(2));
    }

    #[test]
    fn test_get_block_overrides_empty() {
        let call_args = CallArgs::parse_from([""]);
        let overrides = call_args.get_block_overrides().unwrap();
        assert_eq!(overrides, None);
    }

    #[test]
    fn test_address_value_override_success() {
        let text = "0x0000000000000000000000000000000000000001:2";
        let (address, value) = address_value_override(text).unwrap();
        assert_eq!(address, "0x0000000000000000000000000000000000000001");
        assert_eq!(value, "2");
    }

    #[test]
    fn test_address_value_override_error() {
        let text = "invalid_value";
        let error = address_value_override(text).unwrap_err();
        assert_eq!(error.to_string(), "Invalid override invalid_value. Expected <address>:<value>");
    }

    #[test]
    fn test_address_slot_value_override_success() {
        let text = "0x0000000000000000000000000000000000000001:2:3";
        let (address, slot, value) = address_slot_value_override(text).unwrap();
        assert_eq!(*address, fixed_bytes!("0x0000000000000000000000000000000000000001"));
        assert_eq!(slot, U256::from(2));
        assert_eq!(value, U256::from(3));
    }

    #[test]
    fn test_address_slot_value_override_error() {
        let text = "invalid_value";
        let error = address_slot_value_override(text).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Invalid override invalid_value. Expected <address>:<slot>:<value>"
        );
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

        assert_eq!(args.balance_overrides, Some(vec!["0x123:0x1234".to_string()]));
        assert_eq!(args.nonce_overrides, Some(vec!["0x123:1".to_string()]));
        assert_eq!(args.code_overrides, Some(vec!["0x123:0x1234".to_string()]));
        assert_eq!(args.state_overrides, Some(vec!["0x123:0x1:0x1234".to_string()]));
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
            args.balance_overrides,
            Some(vec!["0x123:0x1234".to_string(), "0x456:0x5678".to_string()])
        );
        assert_eq!(args.nonce_overrides, Some(vec!["0x123:1".to_string(), "0x456:2".to_string()]));
        assert_eq!(
            args.code_overrides,
            Some(vec!["0x123:0x1234".to_string(), "0x456:0x5678".to_string()])
        );
        assert_eq!(
            args.state_overrides,
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
}
