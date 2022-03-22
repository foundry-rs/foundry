use std::collections::BTreeMap;

use crate::{
  utils,
  cmd::{forge::build::BuildArgs, Cmd},
  opts::evm::EvmArgs
};

use clap::Parser;
use ansi_term::Colour;
use ethers::{
  abi::{Abi, RawLog},
  types::{Address, Bytes, U256}
};

use forge::{
    debug::DebugArena,
    decode::decode_console_logs,
    executor::{
        opts::EvmOpts, CallResult, DatabaseRef, DeployResult, EvmError, Executor, ExecutorBuilder,
        RawCallResult,
    },
    trace::{identifier::LocalTraceIdentifier, CallTraceArena, CallTraceDecoder, TraceKind},
    CALLER,
};
use foundry_utils::{IntoFunction, encode_args};
use foundry_config::{figment::Figment, Config};


// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(RunArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    #[clap(help = "the bytecode to execute")]
    pub bytecode: String,

    // Optional Calldata
    #[clap(help = "the calldata to pass to the contract")]
    pub calldata: Option<String>,

    /// Open the script in the debugger.
    #[clap(long)]
    pub debug: bool,

    #[clap(flatten)]
    opts: BuildArgs,

    #[clap(flatten)]
    pub evm_opts: EvmArgs,
}

impl Cmd for RunArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        // Parse bytecode string
        let bytecode_vec = self.bytecode.strip_prefix("0x").unwrap_or(&self.bytecode);
        let parsed_bytecode = Bytes::from(hex::decode(bytecode_vec)?);

        println!("Got bytecode: {:?}", parsed_bytecode.to_vec());

        // Load figment
        let figment: Figment = From::from(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let verbosity = 5; // evm_opts.verbosity;
        let config = Config::from_provider(figment).sanitized();

        // Create executor
        let mut builder = ExecutorBuilder::new()
            .with_cheatcodes(evm_opts.ffi)
            .with_config(evm_opts.evm_env())
            .with_spec(crate::utils::evm_spec(&config.evm_version))
            .with_fork(utils::get_fork(&evm_opts, &config.rpc_storage_caching));
        if verbosity >= 3 {
            builder = builder.with_tracing();
        }
        if self.debug {
            builder = builder.with_tracing().with_debugger();
        }

        // Parse Calldata
        let calldata: Bytes = if let Some(calldata) = self.calldata.unwrap_or("0x".to_string()).strip_prefix("0x") {
            hex::decode(calldata)?.into()
        } else {
            let args: Vec<String> = vec![];
            encode_args(&IntoFunction::into("".to_string()), &args)?.into()
        };
        println!("Calldata: {:?}", calldata.to_vec());

        // Create the runner
        let mut runner =
            Runner::new(builder.build(), evm_opts.initial_balance, evm_opts.sender);

        // Deploy the bytecode
        let DeployResult {
            address,
            gas,
            ..
        } = runner.setup(parsed_bytecode)?;

        println!("Deployed contract at: {:?}", address);

        // Run the bytecode at the deployed address
        let rcr = runner.run(
            address,
            calldata,
        )?;

        println!("Raw Call Result: {:?}", rcr);

        // TODO: Waterfall debug

        if rcr.reverted {
            println!("{}", Colour::Red.paint("x FAILURE"));
        } else {
            println!("{}", Colour::Green.paint("✔ SUCCESS"));
        }

        println!("Gas used: {}", rcr.gas);

        Ok(())
    }
}

struct Runner<DB: DatabaseRef> {
    pub executor: Executor<DB>,
    pub initial_balance: U256,
    pub sender: Address,
}

impl<DB: DatabaseRef> Runner<DB> {
    pub fn new(executor: Executor<DB>, initial_balance: U256, sender: Address) -> Self {
        Self { executor, initial_balance, sender }
    }

    pub fn setup(
        &mut self,
        code: Bytes,
    ) -> eyre::Result<DeployResult> {
        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(self.sender, U256::MAX);
        self.executor.set_balance(*CALLER, U256::MAX);

        // We set the nonce of the deployer accounts to 1 to get the same addresses as DappTools
        self.executor.set_nonce(self.sender, 1);

        // Deploy an instance of the contract
        Ok(self.executor.deploy(self.sender, code.0, 0u32.into()).expect("couldn't deploy"))
    }

    pub fn run(&mut self, address: Address, calldata: Bytes) -> eyre::Result<RawCallResult> {
        Ok(self.executor.call_raw(self.sender, address, calldata.0, 0_u64.into())?)
    }
}
