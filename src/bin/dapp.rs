use structopt::StructOpt;

use dapptools::dapp::{Executor, MultiContractRunner};
use evm::Config;

use ansi_term::Colour;

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
        // TODO: Add extra configuration options around blockchain context
    },
}

fn main() -> eyre::Result<()> {
    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::Test { contracts } => {
            let cfg = Config::istanbul();
            let gas_limit = 12_500_000;
            let env = Executor::new_vicinity();

            let runner = MultiContractRunner::new(&contracts, &cfg, gas_limit, env).unwrap();
            let results = runner.test().unwrap();

            // TODO: Once we add traces in the VM, proceed to print them in a nice and structured
            // way
            for (contract_name, tests) in results {
                if !tests.is_empty() {
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
                // skip a line
                println!();
            }
        }
    }

    Ok(())
}
