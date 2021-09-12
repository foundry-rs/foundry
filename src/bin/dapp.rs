use structopt::StructOpt;

use dapptools::dapp::{Executor, MultiContractRunner};
use evm::Config;

use ansi_term::Colour;
use std::path::PathBuf;

#[derive(Debug, StructOpt)]
pub struct Opts {
    #[structopt(subcommand)]
    pub sub: Subcommands,
}

#[derive(Debug, StructOpt)]
#[structopt(about = "Perform Ethereum RPC calls from the comfort of your command line.")]
pub enum Subcommands {
    Test {
        #[structopt(
            help = "glob path to your smart contracts",
            long,
            short,
            default_value = "./src/**/*.sol"
        )]
        contracts: String,

        #[structopt(
            help = "path to where the contract artifacts are stored",
            long = "out",
            short,
            default_value = "./out/dapp.sol.json"
        )]
        out_path: PathBuf,
        // TODO: Add extra configuration options around blockchain context
    },
}

fn main() -> eyre::Result<()> {
    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::Test {
            contracts,
            out_path,
        } => {
            let cfg = Config::istanbul();
            let gas_limit = 12_500_000;
            let env = Executor::new_vicinity();

            let runner = MultiContractRunner::new(&contracts, out_path, &cfg, gas_limit, env)?;
            let results = runner.test()?;

            // TODO: Once we add traces in the VM, proceed to print them in a nice and structured
            // way
            for (i, (contract_name, tests)) in results.iter().enumerate() {
                if !tests.is_empty() {
                    if i > 0 {
                        println!()
                    }
                    println!("Running {} tests for {}", tests.len(), contract_name);
                }

                for (name, result) in tests {
                    let status = if result.success {
                        Colour::Green.paint("[PASS]")
                    } else {
                        Colour::Red.paint("[FAIL]")
                    };
                    println!("{} {} (gas: {})", status, name, result.gas_used);
                }
            }
        }
    }

    Ok(())
}
