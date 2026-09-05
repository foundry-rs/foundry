use super::creation_code::{
    constructor_args_offset, constructor_with_args, fetch_creation_code, load_abi,
};
use alloy_dyn_abi::DynSolType;
use alloy_primitives::{Address, Bytes};
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::LoadConfig,
};

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
        let Self { contract, abi_path, .. } = self;

        let bytecode = fetch_creation_code(&mut config, contract).await?;
        let abi = load_abi(contract, &config, abi_path.as_deref()).await?;
        let constructor = constructor_with_args(&abi)?;
        let split = constructor_args_offset(constructor, &bytecode)?;

        for (input, arg) in constructor.inputs.iter().zip(bytecode[split..].chunks(32)) {
            let decoded = DynSolType::parse(&input.ty)?.abi_decode(arg)?;
            sh_println!("{} → {decoded:?}", Bytes::copy_from_slice(arg))?;
        }
        Ok(())
    }
}
