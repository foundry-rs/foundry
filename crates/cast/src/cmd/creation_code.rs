use super::interface::load_abi_from_file;
use crate::SimpleCast;
use alloy_consensus::Transaction;
use alloy_primitives::{Address, Bytes};
use alloy_provider::{Provider, ext::TraceApi};
use alloy_rpc_types::trace::parity::{Action, CreateAction, CreateOutput, TraceOutput};
use clap::Parser;
use eyre::{OptionExt, Result, eyre};
use foundry_block_explorers::Client;
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::{self, LoadConfig, fetch_abi_from_etherscan},
};
use foundry_common::provider::RetryProvider;
use foundry_config::Config;

foundry_config::impl_figment_convert!(CreationCodeArgs, etherscan, rpc);

/// CLI arguments for `cast creation-code`.
#[derive(Parser)]
pub struct CreationCodeArgs {
    /// An Ethereum address, for which the bytecode will be fetched.
    contract: Address,

    /// Path to file containing the contract's JSON ABI. It's necessary if the target contract is
    /// not verified on Etherscan.
    #[arg(long)]
    abi_path: Option<String>,

    /// Disassemble bytecodes into individual opcodes.
    #[arg(long)]
    disassemble: bool,

    /// Return creation bytecode without constructor arguments appended.
    #[arg(long, conflicts_with = "only_args")]
    without_args: bool,

    /// Return only constructor arguments.
    #[arg(long)]
    only_args: bool,

    #[command(flatten)]
    etherscan: EtherscanOpts,

    #[command(flatten)]
    rpc: RpcOpts,
}

impl CreationCodeArgs {
    pub async fn run(self) -> Result<()> {
        let mut config = self.load_config()?;

        let Self { contract, disassemble, without_args, only_args, abi_path, etherscan: _, rpc: _ } =
            self;

        let provider = utils::get_provider(&config)?;
        let chain = provider.get_chain_id().await?;
        config.chain = Some(chain.into());

        let bytecode = fetch_creation_code_from_etherscan(contract, &config, provider).await?;

        let bytecode = parse_code_output(
            bytecode,
            contract,
            &config,
            abi_path.as_deref(),
            without_args,
            only_args,
        )
        .await?;

        if disassemble {
            let _ = sh_println!("{}", SimpleCast::disassemble(&bytecode)?);
        } else {
            let _ = sh_println!("{bytecode}");
        }

        Ok(())
    }
}

/// Parses the creation bytecode and returns one of the following:
/// - The complete bytecode
/// - The bytecode without constructor arguments
/// - Only the constructor arguments
pub async fn parse_code_output(
    bytecode: Bytes,
    contract: Address,
    config: &Config,
    abi_path: Option<&str>,
    without_args: bool,
    only_args: bool,
) -> Result<Bytes> {
    if !without_args && !only_args {
        return Ok(bytecode);
    }

    let abi = if let Some(abi_path) = abi_path {
        load_abi_from_file(abi_path, None)?
    } else {
        fetch_abi_from_etherscan(contract, config).await?
    };

    let abi = abi.into_iter().next().ok_or_eyre("No ABI found.")?;
    let (abi, _) = abi;

    if abi.constructor.is_none() {
        if only_args {
            return Err(eyre!("No constructor found."));
        }
        return Ok(bytecode);
    }

    let constructor = abi.constructor.unwrap();
    if constructor.inputs.is_empty() {
        if only_args {
            return Err(eyre!("No constructor arguments found."));
        }
        return Ok(bytecode);
    }

    let args_size = constructor.inputs.len() * 32;

    let bytecode = if without_args {
        Bytes::from(bytecode[..bytecode.len() - args_size].to_vec())
    } else if only_args {
        Bytes::from(bytecode[bytecode.len() - args_size..].to_vec())
    } else {
        unreachable!();
    };

    Ok(bytecode)
}

/// Fetches the creation code of a contract from Etherscan and RPC.
pub async fn fetch_creation_code_from_etherscan(
    contract: Address,
    config: &Config,
    provider: RetryProvider,
) -> Result<Bytes> {
    let chain = config.chain.unwrap_or_default();
    let api_key = config.get_etherscan_api_key(Some(chain)).unwrap_or_default();
    let client = Client::new(chain, api_key)?;
    let creation_data = client.contract_creation_data(contract).await?;
    let creation_tx_hash = creation_data.transaction_hash;
    let tx_data = provider.get_transaction_by_hash(creation_tx_hash).await?;
    let tx_data = tx_data.ok_or_eyre("Could not find creation tx data.")?;

    let bytecode = if tx_data.to().is_none() {
        // Contract was created using a standard transaction
        tx_data.input().clone()
    } else {
        // Contract was created using a factory pattern or create2
        // Extract creation code from tx traces
        let mut creation_bytecode = None;

        let traces = provider.trace_transaction(creation_tx_hash).await.map_err(|e| {
            eyre!("Could not fetch traces for transaction {}: {}", creation_tx_hash, e)
        })?;

        for trace in traces {
            if let Some(TraceOutput::Create(CreateOutput { address, .. })) = trace.trace.result
                && address == contract
            {
                creation_bytecode = match trace.trace.action {
                    Action::Create(CreateAction { init, .. }) => Some(init),
                    _ => None,
                };
            }
        }

        creation_bytecode.ok_or_else(|| eyre!("Could not find contract creation trace."))?
    };

    Ok(bytecode)
}
