//! build command

use ethers::{
    solc::{
        artifacts::{Optimizer, Settings},
        remappings::Remapping,
        MinimalCombinedArtifacts, Project, ProjectCompileOutput, ProjectPathsConfig, SolcConfig,
    },
    types::Address,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::{cmd::Cmd, opts::forge::CompilerArgs, utils};

use clap::{Parser, ValueHint};
#[cfg(feature = "evmodin-evm")]
use evmodin::util::mocked_host::MockedHost;
#[cfg(feature = "sputnik-evm")]
use sputnik::backend::MemoryVicinity;

#[derive(Debug, Clone, Parser)]
pub struct BuildArgs {
    #[clap(
        help = "the project's root path. By default, this is the root directory of the current Git repository or the current working directory if it is not part of a Git repository",
        long,
        value_hint = ValueHint::DirPath
    )]
    pub root: Option<PathBuf>,

    #[clap(
        env = "DAPP_SRC",
        help = "the directory relative to the root under which the smart contracts are",
        long,
        short,
        value_hint = ValueHint::DirPath
    )]
    pub contracts: Option<PathBuf>,

    #[clap(help = "the remappings", long, short)]
    pub remappings: Vec<ethers::solc::remappings::Remapping>,
    #[clap(long = "remappings-env", env = "DAPP_REMAPPINGS")]
    pub remappings_env: Option<String>,

    #[clap(
        help = "the paths where your libraries are installed",
        long,
        value_hint = ValueHint::DirPath
    )]
    pub lib_paths: Vec<PathBuf>,

    #[clap(
        help = "path to where the contract artifacts are stored",
        long = "out",
        short,
        value_hint = ValueHint::DirPath
    )]
    pub out_path: Option<PathBuf>,

    #[clap(flatten)]
    pub compiler: CompilerArgs,

    #[clap(help = "ignore warnings with specific error codes", long)]
    pub ignored_error_codes: Vec<u64>,

    #[clap(
        help = "if set to true, skips auto-detecting solc and uses what is in the user's $PATH ",
        long
    )]
    pub no_auto_detect: bool,

    #[clap(
        help = "force recompilation of the project, deletes the cache and artifacts folders",
        long
    )]
    pub force: bool,

    #[clap(
        help = "uses hardhat style project layout. This a convenience flag and is the same as `--contracts contracts --lib-paths node_modules`",
        long,
        conflicts_with = "contracts",
        alias = "hh"
    )]
    pub hardhat: bool,

    #[clap(help = "add linked libraries", long, env = "DAPP_LIBRARIES")]
    pub libraries: Vec<String>,
}

impl Cmd for BuildArgs {
    type Output = ProjectCompileOutput<MinimalCombinedArtifacts>;
    fn run(self) -> eyre::Result<Self::Output> {
        let project = self.project()?;
        super::compile(&project)
    }
}

impl BuildArgs {
    /// Determines the source directory within the given root
    fn contracts_path(&self, root: impl AsRef<Path>) -> PathBuf {
        let root = root.as_ref();
        if let Some(ref contracts) = self.contracts {
            root.join(contracts)
        } else if self.hardhat {
            root.join("contracts")
        } else {
            // no contract source directory was provided, determine the source directory
            ProjectPathsConfig::find_source_dir(&root)
        }
    }

    /// Determines the artifacts directory within the given root
    fn artifacts_path(&self, root: impl AsRef<Path>) -> PathBuf {
        let root = root.as_ref();
        if let Some(ref artifacts) = self.out_path {
            root.join(artifacts)
        } else if self.hardhat {
            root.join("artifacts")
        } else {
            // no artifacts source directory was provided, determine the artifacts directory
            ProjectPathsConfig::find_artifacts_dir(&root)
        }
    }

    /// Determines the libraries
    fn libs(&self, root: impl AsRef<Path>) -> Vec<PathBuf> {
        let root = root.as_ref();
        if self.lib_paths.is_empty() {
            if self.hardhat {
                vec![root.join("node_modules")]
            } else {
                // no libs directories provided
                ProjectPathsConfig::find_libs(&root)
            }
        } else {
            let mut libs = self.lib_paths.clone();
            if self.hardhat && !self.lib_paths.iter().any(|lib| lib.ends_with("node_modules")) {
                // if --hardhat was set, ensure it is present in the lib set
                libs.push(root.join("node_modules"));
            }
            libs
        }
    }

    /// Converts all build arguments to the corresponding project config
    ///
    /// Defaults to DAppTools-style repo layout, but can be customized.
    pub fn project(&self) -> eyre::Result<Project> {
        // 1. Set the root dir
        let root = self.root.clone().unwrap_or_else(|| {
            utils::find_git_root_path().unwrap_or_else(|_| std::env::current_dir().unwrap())
        });
        let root = dunce::canonicalize(&root)?;

        // 2. Set the contracts dir
        let contracts = self.contracts_path(&root);

        // 3. Set the output dir
        let artifacts = self.artifacts_path(&root);

        // 4. Set where the libraries are going to be read from
        // default to the lib path being the `lib/` dir
        let lib_paths = self.libs(&root);

        // get all the remappings corresponding to the lib paths
        let mut remappings: Vec<_> = lib_paths.iter().flat_map(Remapping::find_many).collect();

        // extend them with the once manually provided in the opts
        remappings.extend_from_slice(&self.remappings);

        // extend them with the one via the env vars
        if let Some(ref env) = self.remappings_env {
            remappings.extend(remappings_from_newline(env))
        }

        // extend them with the one via the requirements.txt
        if let Ok(ref remap) = std::fs::read_to_string(root.join("remappings.txt")) {
            remappings.extend(remappings_from_newline(remap))
        }

        // helper function for parsing newline-separated remappings
        fn remappings_from_newline(remappings: &str) -> impl Iterator<Item = Remapping> + '_ {
            remappings.split('\n').filter(|x| !x.is_empty()).map(|x| {
                Remapping::from_str(x)
                    .unwrap_or_else(|_| panic!("could not parse remapping: {}", x))
            })
        }

        // remove any potential duplicates
        remappings.sort_unstable();
        remappings.dedup();

        // build the path
        let mut paths_builder =
            ProjectPathsConfig::builder().root(&root).sources(contracts).artifacts(artifacts);

        if !remappings.is_empty() {
            paths_builder = paths_builder.remappings(remappings);
        }

        let paths = paths_builder.build()?;

        let optimizer = Optimizer {
            enabled: Some(self.compiler.optimize),
            runs: Some(self.compiler.optimize_runs as usize),
        };

        // unflatten the libraries
        let mut libraries = BTreeMap::default();
        for l in self.libraries.iter() {
            let mut items = l.split(':');
            let file = String::from(items.next().expect("could not parse libraries"));
            let lib = String::from(items.next().expect("could not parse libraries"));
            let addr = String::from(items.next().expect("could not parse libraries"));
            libraries.entry(file).or_insert_with(BTreeMap::default).insert(lib, addr);
        }

        // build the project w/ allowed paths = root and all the libs
        let solc_settings = Settings {
            optimizer,
            evm_version: Some(self.compiler.evm_version),
            libraries,
            ..Default::default()
        };
        let mut builder = Project::builder()
            .paths(paths)
            .allowed_path(&root)
            .allowed_paths(lib_paths)
            .solc_config(SolcConfig::builder().settings(solc_settings).build()?);

        if self.no_auto_detect {
            builder = builder.no_auto_detect();
        }

        for error_code in &self.ignored_error_codes {
            builder = builder.ignore_error_code(*error_code);
        }

        let project = builder.build()?;

        // if `--force` is provided, it proceeds to remove the cache
        // and recompile the contracts.
        if self.force {
            project.cleanup()?;
        }

        Ok(project)
    }
}

#[derive(Clone, Debug)]
pub enum EvmType {
    #[cfg(feature = "sputnik-evm")]
    Sputnik,
    #[cfg(feature = "evmodin-evm")]
    EvmOdin,
}

impl FromStr for EvmType {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            #[cfg(feature = "sputnik-evm")]
            "sputnik" => EvmType::Sputnik,
            #[cfg(feature = "evmodin-evm")]
            "evmodin" => EvmType::EvmOdin,
            other => eyre::bail!("unknown EVM type {}", other),
        })
    }
}

#[derive(Debug, Clone, Parser)]
pub struct Env {
    #[clap(help = "the block gas limit", long, default_value_t = u64::MAX)]
    pub gas_limit: u64,

    #[clap(help = "the chainid opcode value", long, default_value = "1")]
    pub chain_id: u64,

    #[clap(help = "the tx.gasprice value during EVM execution", long, default_value = "0")]
    pub gas_price: u64,

    #[clap(help = "the base fee in a block", long, default_value = "0")]
    pub block_base_fee_per_gas: u64,

    #[clap(
        help = "the tx.origin value during EVM execution",
        long,
        default_value_t = Address::zero()
    )]
    pub tx_origin: Address,

    #[clap(
        help = "the block.coinbase value during EVM execution",
        long,
        default_value_t = Address::zero()
    )]
    pub block_coinbase: Address,
    #[clap(
        help = "the block.timestamp value during EVM execution",
        long,
        default_value = "0",
        env = "DAPP_TEST_TIMESTAMP"
    )]
    pub block_timestamp: u64,

    #[clap(help = "the block.number value during EVM execution", long, default_value = "0")]
    #[clap(env = "DAPP_TEST_NUMBER")]
    pub block_number: u64,

    #[clap(help = "the block.difficulty value during EVM execution", long, default_value = "0")]
    pub block_difficulty: u64,

    #[clap(help = "the block.gaslimit value during EVM execution", long)]
    pub block_gas_limit: Option<u64>,
    // TODO: Add configuration option for base fee.
}

impl Env {
    #[cfg(feature = "sputnik-evm")]
    pub fn sputnik_state(&self) -> MemoryVicinity {
        MemoryVicinity {
            chain_id: self.chain_id.into(),

            gas_price: self.gas_price.into(),
            origin: self.tx_origin,

            block_coinbase: self.block_coinbase,
            block_number: self.block_number.into(),
            block_timestamp: self.block_timestamp.into(),
            block_difficulty: self.block_difficulty.into(),
            block_base_fee_per_gas: self.block_base_fee_per_gas.into(),
            block_gas_limit: self.block_gas_limit.unwrap_or(self.gas_limit).into(),
            block_hashes: Vec::new(),
        }
    }

    #[cfg(feature = "evmodin-evm")]
    pub fn evmodin_state(&self) -> MockedHost {
        let mut host = MockedHost::default();

        host.tx_context.chain_id = self.chain_id.into();
        host.tx_context.tx_gas_price = self.gas_price.into();
        host.tx_context.tx_origin = self.tx_origin;
        host.tx_context.block_coinbase = self.block_coinbase;
        host.tx_context.block_number = self.block_number;
        host.tx_context.block_timestamp = self.block_timestamp;
        host.tx_context.block_difficulty = self.block_difficulty.into();
        host.tx_context.block_gas_limit = self.block_gas_limit.unwrap_or(self.gas_limit);

        host
    }
}
