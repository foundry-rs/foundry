use crate::cmd::{forge::build::BuildArgs, Cmd};
use clap::Parser;

use forge::ContractRunner;
use foundry_utils::IntoFunction;

use ethers::types::{Address, Bytes, U256};
use sputnik::ExitReason;

use crate::opts::evm::EvmArgs;
use ansi_term::Colour;
use evm_adapters::{
    evm_opts::{BackendKind, EvmOpts},
    sputnik::helpers::vm,
    Evm,
};
use foundry_config::{figment::Figment, Config};

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(ExecArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct ExecArgs {
    #[clap(help = "the bytecode to execute")]
    pub bytecode: String,

    #[clap(flatten)]
    pub evm_opts: EvmArgs,

    #[clap(flatten)]
    opts: BuildArgs,
}

impl Cmd for ExecArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        // Keeping it like this for simplicity.
        #[cfg(not(feature = "sputnik-evm"))]
        unimplemented!("`exec` does not work with EVMs other than Sputnik yet");

        let figment: Figment = From::from(&self);
        let mut evm_opts = figment.extract::<EvmOpts>()?;
        let config = Config::from_provider(figment).sanitized();
        let evm_version = config.evm_version;
        if evm_opts.debug {
            evm_opts.verbosity = 3;
        }

        let mut cfg = crate::utils::sputnik_cfg(&evm_version);
        cfg.create_contract_limit = None;
        let vicinity = evm_opts.vicinity()?;
        let backend = evm_opts.backend(&vicinity)?;

        // Parse bytecode string
        let bytecode_vec = self.bytecode.strip_prefix("0x").unwrap_or(&self.bytecode);
        let parsed_bytecode = Bytes::from(hex::decode(bytecode_vec)?);

        // Create the evm executor
        let mut evm = vm();

        // Deploy our bytecode
        let custVal = U256::from(0);
        let (addr, _, _, _) = evm.deploy(Address::zero(), parsed_bytecode, custVal).unwrap();

        // Configure EVM
        evm.gas_limit = u64::MAX;

        // TODO: support arbitrary input
        // let sig = ethers::utils::id("foo()").to_vec();

        // Call the address with an empty input
        let (retBytes, retReason, retU64, retVecStr) =
            evm.call_raw(Address::zero(), addr, Default::default(), custVal, true)?;

        // Match on the return exit reason
        match retReason {
            ExitReason::Succeed(s) => {
                println!("{}", Colour::Green.paint(format!("SUCCESS [{:?}]", s)));
                println!("");
                println!("==== Execution Return Bytes ====");
                println!("{}", retBytes);
            }
            ExitReason::Error(e) => {
                println!("{}", Colour::Red.paint(format!("ERROR [{:?}]", e)));
            }
            ExitReason::Revert(r) => {
                println!("{}", Colour::Yellow.paint(format!("REVERT [{:?}]", r)));
            }
            ExitReason::Fatal(f) => {
                println!("{}", Colour::Red.paint(format!("FATAL [{:?}]", f)));
            }
        }

        Ok(())
    }
}

impl ExecArgs {
    pub fn build(&self, _: Config, _: &EvmOpts) -> eyre::Result<()> {
        Ok(())
    }
}
