use crate::tx::{CastTxBuilder, SenderKind};
use alloy_primitives::{TxKind, U256};
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use cast::{traces::TraceKind, Cast};
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, handle_traces, parse_ether_value, TraceResult},
};
use foundry_common::ens::NameOrAddress;
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{
    figment::{
        self,
        value::{Dict, Map},
        Figment, Metadata, Profile,
    },
    Config,
};
use foundry_evm::{executors::TracingExecutor, opts::EvmOpts};
use std::str::FromStr;

/// CLI arguments for `cast call`.
#[derive(Debug, Parser)]
pub struct CallArgs {
    /// The destination of the transaction.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    /// Data for the transaction.
    #[arg(
        long,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    /// Forks the remote rpc, executes the transaction locally and prints a trace
    #[arg(long, default_value_t = false)]
    trace: bool,

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

    /// Print the decoded output as JSON.
    #[arg(long, short, help_heading = "Display options")]
    json: bool,

    /// Enable Alphanet features.
    #[arg(long)]
    pub alphanet: bool,

    #[command(subcommand)]
    command: Option<CallSubcommands>,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,
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
        let figment = Into::<Figment>::into(&self.eth).merge(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut config = Config::try_from(figment)?.sanitized();

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
            json,
            ..
        } = self;

        if let Some(data) = data {
            sig = Some(data);
        }

        let provider = utils::get_provider(&config)?;
        let sender = SenderKind::from_wallet_opts(eth.wallet).await?;
        let from = sender.address();

        let tx_kind = if let Some(to) = to {
            TxKind::Call(to.resolve(&provider).await?)
        } else {
            TxKind::Create
        };

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
            .with_tx_kind(tx_kind)
            .with_code_sig_and_args(code, sig, args)
            .await?
            .build_raw(sender)
            .await?;

        if trace {
            if let Some(BlockId::Number(BlockNumberOrTag::Number(block_number))) = self.block {
                // Override Config `fork_block_number` (if set) with CLI value.
                config.fork_block_number = Some(block_number);
            }

            let (mut env, fork, chain, alphanet) =
                TracingExecutor::get_fork_material(&config, evm_opts).await?;

            // modify settings that usually set in eth_call
            env.cfg.disable_block_gas_limit = true;
            env.block.gas_limit = U256::MAX;

            let mut executor =
                TracingExecutor::new(env, fork, evm_version, debug, decode_internal, alphanet);

            let value = tx.value.unwrap_or_default();
            let input = tx.inner.input.into_input().unwrap_or_default();

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

            handle_traces(trace, &config, chain, labels, debug, decode_internal).await?;

            return Ok(());
        }

        println!("{}", Cast::new(provider).call(&tx, func.as_ref(), block, json).await?);

        Ok(())
    }
}

impl figment::Provider for CallArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("CallArgs")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut map = Map::new();

        if self.alphanet {
            map.insert("alphanet".into(), self.alphanet.into());
        }

        if let Some(evm_version) = self.evm_version {
            map.insert("evm_version".into(), figment::value::Value::serialize(evm_version)?);
        }

        Ok(Map::from([(Config::selected_profile(), map)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{hex, Address};

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
    fn call_sig_and_data_exclusive() {
        let data = hex::encode("hello");
        let to = Address::ZERO;
        let args = CallArgs::try_parse_from([
            "foundry-cli",
            to.to_string().as_str(),
            "signature",
            "--data",
            format!("0x{data}").as_str(),
        ]);

        assert!(args.is_err());
    }
}
