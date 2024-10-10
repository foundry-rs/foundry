use alloy_primitives::{Address, Bytes};
use clap::{command, Parser};
use eyre::Result;
use foundry_block_explorers::Client;
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils,
};
use foundry_config::Config;

use super::{creation_code::fetch_creation_code, interface::fetch_abi_from_etherscan};

/// CLI arguments for `cast creation-args`.
#[derive(Parser)]
pub struct CreationArgsArgs {
    /// An Ethereum address, for which the bytecode will be fetched.
    contract: Address,

    #[command(flatten)]
    etherscan: EtherscanOpts,
    #[command(flatten)]
    rpc: RpcOpts,
}

impl CreationArgsArgs {
    pub async fn run(self) -> Result<()> {
        let Self { contract, etherscan, rpc } = self;

        let config = Config::from(&etherscan);
        let chain = config.chain.unwrap_or_default();
        let api_key = config.get_etherscan_api_key(Some(chain)).unwrap_or_default();
        let client = Client::new(chain, api_key)?;

        let config = Config::from(&rpc);
        let provider = utils::get_provider(&config)?;

        let bytecode = fetch_creation_code(contract, client, provider).await?;

        let args_arr = parse_creation_args(bytecode, contract, &etherscan).await?;

        for arg in args_arr {
            println!("{arg}");
        }

        Ok(())
    }
}

/// Fetches the constructor arguments values and types from the creation bytecode and ABI.
async fn parse_creation_args(
    bytecode: Bytes,
    contract: Address,
    etherscan: &EtherscanOpts,
) -> Result<Vec<String>> {
    let abi = fetch_abi_from_etherscan(contract, etherscan).await?;
    let abi = abi.into_iter().next().ok_or_else(|| eyre::eyre!("No ABI found."))?;
    let (abi, _) = abi;

    if abi.constructor.is_none() {
        return Err(eyre::eyre!("No constructor found."));
    }

    let constructor = abi.constructor.unwrap();
    if constructor.inputs.is_empty() {
        return Err(eyre::eyre!("No constructor arguments found."));
    }

    let args_size = constructor.inputs.len() * 32;
    let args_bytes = Bytes::from(bytecode[bytecode.len() - args_size..].to_vec());

    let display_args: Vec<String> = args_bytes
        .chunks(32)
        .enumerate()
        .map(|(i, arg)| {
            let arg = arg.to_vec();
            format!("{} {}", constructor.inputs[i].ty, Bytes::from(arg))
        })
        .collect();

    Ok(display_args)
}
