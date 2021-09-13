use structopt::StructOpt;

use dapptools::dapp::MultiContractRunner;
use evm::{backend::MemoryVicinity, Config};

use ansi_term::Colour;
use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
    str::FromStr,
};

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
        #[structopt(help = "print the test results in json format", long, short)]
        json: bool,

        #[structopt(flatten)]
        env: Env,

        #[structopt(
            long = "--match",
            short = "-m",
            help = "only run test methods matching regex",
            default_value = ".*"
        )]
        pattern: regex::Regex,

        #[structopt(flatten)]
        opts: BuildOpts,
    },
    Build {
        #[structopt(flatten)]
        opts: BuildOpts,
    },
}

#[derive(Debug, StructOpt)]
#[structopt(about = "build your smart contracts")]
struct BuildOpts {
    #[structopt(
        help = "glob path to your smart contracts",
        long,
        short,
        default_value = "./src/**/*.sol"
    )]
    contracts: String,

    #[structopt(help = "the remappings", long, short)]
    remappings: Vec<String>,
    #[structopt(env = "DAPP_REMAPPINGS")]
    remappings_env: Option<String>,

    #[structopt(help = "the paths where your libraries are installed", long)]
    lib_paths: Vec<String>,

    #[structopt(
        help = "path to where the contract artifacts are stored",
        long = "out",
        short,
        default_value = "./out/dapp.sol.json"
    )]
    out_path: PathBuf,

    #[structopt(help = "skip re-compilation", long, short)]
    no_compile: bool,

    #[structopt(help = "choose the evm version", long, default_value = "berlin")]
    evm_version: EvmVersion,
}

#[derive(Clone, Debug)]
pub enum EvmVersion {
    Frontier,
    Istanbul,
    Berlin,
}

impl EvmVersion {
    fn cfg(self) -> Config {
        use EvmVersion::*;
        match self {
            Frontier => Config::frontier(),
            Istanbul => Config::istanbul(),
            Berlin => Config::berlin(),
        }
    }
}

impl FromStr for EvmVersion {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use EvmVersion::*;
        Ok(match s.to_lowercase().as_str() {
            "frontier" => Frontier,
            "istanbul" => Istanbul,
            "berlin" => Berlin,
            // TODO: Add London.
            _ => eyre::bail!("unsupported evm version: {}", s),
        })
    }
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
            opts:
                BuildOpts {
                    contracts,
                    remappings,
                    remappings_env,
                    lib_paths,
                    out_path,
                    evm_version,
                    no_compile,
                },
            env,
            json,
            pattern,
        } => {
            let cfg = evm_version.cfg();
            let remappings = merge(remappings, remappings_env);
            let lib_paths = default_path(lib_paths)?;

            let runner = MultiContractRunner::new(
                &contracts,
                remappings,
                lib_paths,
                out_path,
                &cfg,
                env.gas_limit,
                env.vicinity(),
                no_compile,
            )?;
            let results = runner.test(pattern)?;

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
        Subcommands::Build {
            opts:
                BuildOpts {
                    contracts,
                    remappings,
                    remappings_env,
                    lib_paths,
                    out_path,
                    evm_version: _,
                    no_compile,
                },
        } => {
            // build the contracts
            let remappings = merge(remappings, remappings_env);
            let lib_paths = default_path(lib_paths)?;
            // TODO: Do we also want to include the file path in the contract map so
            // that we're more compatible with dapptools' artifact?
            let contracts = MultiContractRunner::build(
                &contracts,
                remappings,
                lib_paths,
                out_path.clone(),
                no_compile,
            )?;

            let out_file = open_file(out_path)?;

            // dump as json
            serde_json::to_writer(out_file, &contracts)?;
        }
    }

    Ok(())
}

/// Default deps path
const LIB: &str = "lib";
const DEFAULT_OUT_FILE: &str = "dapp.sol.json";

fn default_path(path: Vec<String>) -> eyre::Result<Vec<String>> {
    Ok(if path.is_empty() {
        vec![std::env::current_dir()?
            .join(LIB)
            .into_os_string()
            .into_string()
            .expect("could not parse libs path. is it not utf-8 maybe?")]
    } else {
        path
    })
}

// merge the cli-provided remappings vector with the
// new-line separated env var
fn merge(mut remappings: Vec<String>, remappings_env: Option<String>) -> Vec<String> {
    // merge the cli-provided remappings vector with the
    // new-line separated env var
    if let Some(env) = remappings_env {
        remappings.extend_from_slice(&env.split('\n').map(|x| x.to_string()).collect::<Vec<_>>());
        // deduplicate the extra remappings
        remappings.sort_unstable();
        remappings.dedup();
    }

    remappings
}

/// Opens the file at `out_path` for R/W and creates it if it doesn't exist.
fn open_file(out_path: PathBuf) -> eyre::Result<File> {
    Ok(if out_path.is_file() {
        // get the file if it exists
        OpenOptions::new().write(true).open(out_path)?
    } else if out_path.is_dir() {
        // get the directory if it exists & the default file path
        let out_path = out_path.join(DEFAULT_OUT_FILE);

        // get a file handler (overwrite any contents of the existing file)
        OpenOptions::new().write(true).create(true).open(out_path)?
    } else {
        // otherwise try to create the entire path

        // in case it's a directory, we must mkdir it
        let out_path = if out_path
            .to_str()
            .ok_or_else(|| eyre::eyre!("not utf-8 path"))?
            .ends_with('/')
        {
            std::fs::create_dir_all(&out_path)?;
            out_path.join(DEFAULT_OUT_FILE)
        } else {
            // if it's a file path, we must mkdir the parent
            let parent = out_path
                .parent()
                .ok_or_else(|| eyre::eyre!("could not get parent of {:?}", out_path))?;
            std::fs::create_dir_all(parent)?;
            out_path
        };

        // finally we get the handler
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out_path)?
    })
}
