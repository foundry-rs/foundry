//! Subcommands for forge
//!
//! All subcommands should respect the `foundry_config::Config`.
//! If a subcommand accepts values that are supported by the `Config`, then the subcommand should
//! implement `figment::Provider` which allows the subcommand to override the config's defaults, see
//! [`foundry_config::Config`].
//!
//! See [`BuildArgs`] for a reference implementation.
//! And [`RunArgs`] for how to merge `Providers`.
//!
//! # Example
//!
//! create a `clap` subcommand into a `figment::Provider` and integrate it in the
//! `foundry_config::Config`:
//!
//! ```rust
//! use crate::{cmd::build::BuildArgs, opts::evm::EvmArgs};
//! use clap::Parser;
//! use foundry_config::{figment::Figment, *};
//!
//! // A new clap subcommand that accepts both `EvmArgs` and `BuildArgs`
//! #[derive(Debug, Clone, Parser)]
//! pub struct MyArgs {
//!     #[clap(flatten)]
//!     evm_opts: EvmArgs,
//!     #[clap(flatten)]
//!     opts: BuildArgs,
//! }
//!
//! // add `Figment` and `Config` converters
//! foundry_config::impl_figment_convert!(MyArgs, opts, evm_opts);
//! let args = MyArgs::parse_from(["build"]);
//!
//! let figment: Figment = From::from(&args);
//! let evm_opts = figment.extract::<EvmOpts>().unwrap();
//!
//! let config: Config = From::from(&args);
//! ```

pub mod bind;
pub mod build;
pub mod config;
pub mod create;
pub mod flatten;
pub mod fmt;
pub mod init;
pub mod inspect;
pub mod install;
pub mod remappings;
pub mod run;
pub mod snapshot;
pub mod test;
pub mod tree;
pub mod verify;
pub mod watch;

use crate::{opts::forge::ContractInfo, term};
use ethers::{
    abi::Abi,
    prelude::{
        artifacts::{CompactBytecode, CompactDeployedBytecode},
        report::NoReporter,
    },
    solc::cache::SolFilesCache,
};
use std::{collections::BTreeMap, path::PathBuf};

/// Common trait for all cli commands
pub trait Cmd: clap::Parser + Sized {
    type Output;
    fn run(self) -> eyre::Result<Self::Output>;
}

use ethers::solc::{
    artifacts::CompactContractBytecode, Artifact, FileFilter, Project, ProjectCompileOutput,
};

use foundry_utils::to_table;

/// Given a project and its compiled artifacts, proceeds to return the ABI, Bytecode and
/// Runtime Bytecode of the given contract.
pub fn read_artifact(
    project: &Project,
    compiled: ProjectCompileOutput,
    contract: ContractInfo,
) -> eyre::Result<(Abi, CompactBytecode, CompactDeployedBytecode)> {
    Ok(match contract.path {
        Some(path) => get_artifact_from_path(project, path, contract.name)?,
        None => get_artifact_from_name(contract, compiled)?,
    })
}

/// Helper function for finding a contract by ContractName
// TODO: Is there a better / more ergonomic way to get the artifacts given a project and a
// contract name?
fn get_artifact_from_name(
    contract: ContractInfo,
    compiled: ProjectCompileOutput,
) -> eyre::Result<(Abi, CompactBytecode, CompactDeployedBytecode)> {
    let mut has_found_contract = false;
    let mut contract_artifact = None;

    for (artifact_id, artifact) in compiled.into_artifacts() {
        if artifact_id.name == contract.name {
            if has_found_contract {
                eyre::bail!("contract with duplicate name. pass path")
            }
            has_found_contract = true;
            contract_artifact = Some(artifact);
        }
    }

    Ok(match contract_artifact {
        Some(artifact) => (
            artifact
                .abi
                .map(Into::into)
                .ok_or_else(|| eyre::Error::msg(format!("abi not found for {}", contract.name)))?,
            artifact.bytecode.ok_or_else(|| {
                eyre::Error::msg(format!("bytecode not found for {}", contract.name))
            })?,
            artifact.deployed_bytecode.ok_or_else(|| {
                eyre::Error::msg(format!("bytecode not found for {}", contract.name))
            })?,
        ),
        None => {
            eyre::bail!("could not find artifact")
        }
    })
}

/// Find using src/ContractSource.sol:ContractName
fn get_artifact_from_path(
    project: &Project,
    contract_path: String,
    contract_name: String,
) -> eyre::Result<(Abi, CompactBytecode, CompactDeployedBytecode)> {
    // Get sources from the requested location
    let abs_path = dunce::canonicalize(PathBuf::from(contract_path))?;

    let cache = SolFilesCache::read_joined(&project.paths)?;

    // Read the artifact from disk
    let artifact: CompactContractBytecode = cache.read_artifact(abs_path, &contract_name)?;

    Ok((
        artifact
            .abi
            .ok_or_else(|| eyre::Error::msg(format!("abi not found for {}", contract_name)))?,
        artifact
            .bytecode
            .ok_or_else(|| eyre::Error::msg(format!("bytecode not found for {}", contract_name)))?,
        artifact
            .deployed_bytecode
            .ok_or_else(|| eyre::Error::msg(format!("bytecode not found for {}", contract_name)))?,
    ))
}
