use super::{creation_code::fetch_creation_code_from_etherscan, interface::load_abi_from_file};
use alloy_dyn_abi::DynSolType;
use alloy_primitives::{Address, Bytes};
use alloy_provider::Provider;
use clap::Parser;
use eyre::{OptionExt, Result, eyre};
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::{self, LoadConfig, fetch_abi_from_etherscan},
};
use foundry_config::Config;

foundry_config::impl_figment_convert!(ConstructorArgsArgs, etherscan, rpc);

/// CLI arguments for `cast creation-args`.
#[derive(Parser)]
pub struct ConstructorArgsArgs {
    /// An Ethereum address, for which the bytecode will be fetched.
    contract: Address,

    /// Path to file containing the contract's JSON ABI. It's necessary if the target contract is
    /// not verified on Etherscan
    #[arg(long)]
    abi_path: Option<String>,

    #[command(flatten)]
    etherscan: EtherscanOpts,

    #[command(flatten)]
    rpc: RpcOpts,
}

impl ConstructorArgsArgs {
    pub async fn run(self) -> Result<()> {
        let mut config = self.load_config()?;

        let Self { contract, abi_path, etherscan: _, rpc: _ } = self;

        let provider = utils::get_provider(&config)?;
        config.chain = Some(provider.get_chain_id().await?.into());

        let bytecode = fetch_creation_code_from_etherscan(contract, &config, provider).await?;

        let args_arr = parse_constructor_args(bytecode, contract, &config, abi_path).await?;
        for arg in args_arr {
            let _ = sh_println!("{arg}");
        }

        Ok(())
    }
}

/// Fetches the constructor arguments values and types from the creation bytecode and ABI.
async fn parse_constructor_args(
    bytecode: Bytes,
    contract: Address,
    config: &Config,
    abi_path: Option<String>,
) -> Result<Vec<String>> {
    let abi = if let Some(abi_path) = abi_path {
        load_abi_from_file(&abi_path, None)?
    } else {
        fetch_abi_from_etherscan(contract, config).await?
    };

    let abi = abi.into_iter().next().ok_or_eyre("No ABI found.")?;
    let (abi, _) = abi;

    let constructor = abi.constructor.ok_or_else(|| eyre!("No constructor found."))?;

    if constructor.inputs.is_empty() {
        return Err(eyre!("No constructor arguments found."));
    }

    let args_size = constructor.inputs.len() * 32;
    if bytecode.len() < args_size {
        return Err(eyre!(
            "Invalid creation bytecode length: have {} bytes, need at least {} for {} constructor inputs",
            bytecode.len(),
            args_size,
            constructor.inputs.len()
        ));
    }
    let args_bytes = Bytes::from(bytecode[bytecode.len() - args_size..].to_vec());

    let display_args: Vec<String> = args_bytes
        .chunks(32)
        .enumerate()
        .map(|(i, arg)| format_arg(&constructor.inputs[i].ty, arg))
        .collect::<Result<Vec<_>>>()?;

    Ok(display_args)
}

fn format_arg(ty: &str, arg: &[u8]) -> Result<String> {
    let arg_type: DynSolType = ty.parse()?;
    let decoded = arg_type.abi_decode(arg)?;
    let bytes = Bytes::from(arg.to_vec());

    Ok(format!("{bytes} → {decoded:?}"))
}
