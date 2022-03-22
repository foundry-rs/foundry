use crate::{
    cmd::{forge::build::BuildArgs, Cmd},
    opts::evm::EvmArgs,
    utils,
};

use ansi_term::Colour;
use clap::Parser;
use ethers::types::{Address, Bytes, U256};

use forge::{
    executor::{
        opts::EvmOpts, DatabaseRef, DeployResult, Executor, ExecutorBuilder, RawCallResult,
    },
    trace::TraceKind,
    CALLER,
};
use foundry_config::{figment::Figment, Config};
use foundry_utils::{encode_args, IntoFunction};
use hex::ToHex;

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(RunArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    #[clap(help = "the bytecode to execute")]
    pub bytecode: String,

    // Optional Calldata
    #[clap(help = "the calldata to pass to the contract")]
    pub calldata: Option<String>,

    /// Open the bytecode execution in debug mode
    #[clap(long, help = "debug the bytecode execution")]
    pub debug: bool,

    #[clap(flatten)]
    opts: BuildArgs,

    #[clap(flatten)]
    pub evm_opts: EvmArgs,
}

impl Cmd for RunArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        // Load figment
        let figment: Figment = From::from(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let verbosity = evm_opts.verbosity;
        let config = Config::from_provider(figment).sanitized();

        // Parse bytecode string
        let bytecode_vec = self.bytecode.strip_prefix("0x").unwrap_or(&self.bytecode);
        let parsed_bytecode = Bytes::from(hex::decode(bytecode_vec)?);

        // Parse Calldata
        let calldata: Bytes =
            if let Some(calldata) = self.calldata.unwrap_or("0x".to_string()).strip_prefix("0x") {
                hex::decode(calldata)?.into()
            } else {
                let args: Vec<String> = vec![];
                encode_args(&IntoFunction::into("".to_string()), &args)?.into()
            };

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

        // Create the runner
        let mut runner = Runner::new(builder.build(), evm_opts.sender);

        // Deploy the bytecode
        let DeployResult { address, .. } = runner.setup(parsed_bytecode)?;

        // Run the bytecode at the deployed address
        let rcr = runner.run(address, calldata)?;

        // TODO: Waterfall debug
        // Ex: https://twitter.com/danielvf/status/1503756428212936710

        // Unwrap Traces
        let mut traces =
            rcr.traces.map(|traces| vec![(TraceKind::Execution, traces)]).unwrap_or_default();

        if verbosity >= 3 {
            if traces.is_empty() {
                eyre::bail!("Unexpected error: No traces despite verbosity level. Please report this as a bug: https://github.com/gakonst/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
            }

            if rcr.reverted {
                println!("Traces:");
                for (kind, trace) in &mut traces {
                    let should_include = match kind {
                        TraceKind::Setup => (verbosity >= 5) || (verbosity == 4),
                        TraceKind::Execution => verbosity > 3,
                        _ => false,
                    };

                    if should_include {
                        // TODO: Create decoder using local fork
                        // decoder.decode(trace);
                        println!("{}", trace);
                    }
                }
                println!();
            }
        }

        if rcr.reverted {
            println!("{}", Colour::Red.paint("[REVERT]"));
            println!("Gas consumed: {}", rcr.gas);
        } else {
            println!("{}", Colour::Green.paint("[SUCCESS]"));
            let o = rcr.result.encode_hex::<String>();
            if o.len() > 0 {
                println!("Output: {}", o);
            } else {
                println!("{}", Colour::Yellow.paint("No Output"));
            }
            println!("Gas consumed: {}", rcr.gas);
        }

        Ok(())
    }
}

struct Runner<DB: DatabaseRef> {
    pub executor: Executor<DB>,
    pub sender: Address,
}

impl<DB: DatabaseRef> Runner<DB> {
    pub fn new(executor: Executor<DB>, sender: Address) -> Self {
        Self { executor, sender }
    }

    pub fn setup(&mut self, code: Bytes) -> eyre::Result<DeployResult> {
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
