use structopt::StructOpt;

use dapptools::dapp::{Executor, MultiContractRunner};
use evm::{backend::MemoryVicinity, Config};

use ansi_term::Colour;
use std::path::PathBuf;

#[derive(Debug, StructOpt)]
struct Opts {
    #[structopt(subcommand)]
    sub: Subcommands,
}

#[derive(Debug, StructOpt)]
#[structopt(about = "Perform Ethereum RPC calls from the comfort of your command line.")]
enum Subcommands {
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

        #[structopt(help = "print the test results in json format", long, short)]
        json: bool,

        #[structopt(flatten)]
        env: Env,
    },
}

#[derive(Debug, StructOpt)]
struct Env {
    #[structopt(help = "the block gas limit", long, short, default_value = "25000000")]
    pub gas_limit: u64,
    // TODO: Add extra configuration options around blockchain context
}

impl Env {
    fn vicinity(&self) -> MemoryVicinity {
        Executor::new_vicinity()
    }
}

fn main() -> eyre::Result<()> {
    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::Test {
            contracts,
            out_path,
            env,
            json,
        } => {
            let cfg = Config::istanbul();

            let runner = MultiContractRunner::new(
                &contracts,
                out_path,
                &cfg,
                env.gas_limit,
                env.vicinity(),
            )?;
            let results = runner.test()?;

            if json {
                let res = serde_json::to_string(&results)?;
                println!("{}", res);
            } else {
                // Dapptools-style printing
                for (i, (contract_name, tests)) in results.iter().enumerate() {
                    if i > 0 {
                        println!()
                    }
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
                }
            }
        }
    }

    Ok(())
}
