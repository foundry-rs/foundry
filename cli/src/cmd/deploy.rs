//! Create command

use crate::{
    cmd::{build::BuildArgs, Cmd},
    opts::{EthereumOpts, WalletType},
};
use ethers::{
    abi::{Abi, Constructor, Token},
    prelude::{artifacts::BytecodeObject, ContractFactory, Http, Middleware, Provider},
    types::Chain,
};

use eyre::Result;
use foundry_utils::parse_tokens;

use crate::opts::forge::ContractInfo;
use clap::Parser;
use std::sync::Arc;

#[derive(Debug, Clone, Parser)]
pub struct DeployArgs {
    #[clap(long, help = "selected deployment scripts to run, a list like `1 4 6`")]
    scripts: Vec<u64>,

    #[clap(flatten)]
    opts: BuildArgs,

    #[clap(flatten)]
    eth: EthereumOpts,

    #[clap(help = "contract source info `<path>:<contractname>` or `<contractname>`")]
    contract: ContractInfo,

    #[clap(
        long,
        help = "use legacy transactions instead of EIP1559 ones. this is auto-enabled for common networks without EIP1559"
    )]
    legacy: bool,
}

impl Cmd for DeployArgs {
    type Output = ();

    fn run(self) -> Result<Self::Output> {
        // Find Project & Compile
        let project = self.opts.project()?;
        let compiled = super::compile(&project)?;

        // Get ABI and BIN
        let (abi, bin, _) = super::read_artifact(&project, compiled, self.contract.clone())?;

        let bin = match bin.object {
            BytecodeObject::Bytecode(_) => bin.object,
            _ => eyre::bail!("Dynamic linking not supported in `deploy` command - deploy the library contract first, then provide the address to link at compile time")
        };


        Ok(())
    }
}

impl DeployArgs {
    async fn deploy<M: Middleware + 'static>(
        self,
        abi: Abi,
        bin: BytecodeObject,
        args: Vec<Token>,
        provider: M,
    ) -> Result<()> {
        todo!()
    }
}

/// Helper function for checking if a chainid corresponds to a legacy chainid
/// without eip1559
fn is_legacy<T: TryInto<Chain>>(chain: T) -> bool {
    let chain = match chain.try_into() {
        Ok(inner) => inner,
        _ => return false,
    };

    use Chain::*;
    // TODO: Add other chains which do not support EIP1559.
    matches!(chain, Optimism | OptimismKovan | Fantom | FantomTestnet)
}
