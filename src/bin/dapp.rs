use structopt::StructOpt;

use dapptools::dapp::MultiContractRunner;
use evm::{backend::MemoryVicinity, Config};

use ansi_term::Colour;
use std::path::PathBuf;

use ethers::types::Address;

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
    #[structopt(help = "the block gas limit", long, default_value = "25000000")]
    pub gas_limit: u64,

    #[structopt(help = "the chainid opcode value", long, default_value = "1")]
    pub chain_id: u64,

    #[structopt(
        help = "the tx.gasprice value during EVM execution",
        long,
        default_value = "0"
    )]
    pub gas_price: u64,
    #[structopt(
        help = "the tx.origin value during EVM execution",
        long,
        default_value = "0x0000000000000000000000000000000000000000"
    )]
    pub tx_origin: Address,

    #[structopt(
        help = "the block.coinbase value during EVM execution",
        long,
        // TODO: It'd be nice if we could use Address::zero() here.
        default_value = "0x0000000000000000000000000000000000000000"
    )]
    pub block_coinbase: Address,
    #[structopt(
        help = "the block.timestamp value during EVM execution",
        long,
        default_value = "0"
    )]
    pub block_timestamp: u64,

    #[structopt(
        help = "the block.number value during EVM execution",
        long,
        default_value = "0"
    )]
    pub block_number: u64,

    #[structopt(
        help = "the block.difficulty value during EVM execution",
        long,
        default_value = "0"
    )]
    pub block_difficulty: u64,

    #[structopt(help = "the block.gaslimit value during EVM execution", long)]
    pub block_gas_limit: Option<u64>,
}

impl Env {
    // TODO: Maybe we should allow a way to specify multiple vicinities for use
    // across tests? Probably not, better to do with HEVM cheat codes.
    fn vicinity(&self) -> MemoryVicinity {
        MemoryVicinity {
            chain_id: self.chain_id.into(),

            gas_price: self.gas_price.into(),
            origin: self.tx_origin,

            block_coinbase: self.block_coinbase,
            block_number: self.block_number.into(),
            block_timestamp: self.block_timestamp.into(),
            block_difficulty: self.block_difficulty.into(),
            block_gas_limit: self.block_gas_limit.unwrap_or(self.gas_limit).into(),
            block_hashes: Vec::new(),
        }
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
