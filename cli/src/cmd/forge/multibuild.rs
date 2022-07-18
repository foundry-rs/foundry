//! multibuild command

use crate::{
    cmd::{forge::build::CoreBuildArgs, Cmd},
    compile::ProjectCompiler,
};
use clap::Parser;
use ethers::solc::{Project, RELEASES};
use foundry_config::{Config, SolcReq};
use semver::Version;
use serde::Serialize;

/// Command for building with multiple Solidity versions
#[derive(Clone, Debug, Default, Parser, Serialize)]
pub struct MultibuildArgs {
    /// The Solidity version from which to build.
    ///
    /// Valid values are in the format `x.y.z`.
    #[clap(long, value_name = "SOLC_VERSION")]
    #[serde(skip)]
    from: String,

    /// The Solidity version up to which to build. Must be greater than or equal to `from`.
    ///
    /// Valid values are in the format `x.y.z`.
    #[clap(long, value_name = "SOLC_VERSION")]
    #[serde(skip)]
    to: String,

    #[clap(flatten, next_help_heading = "BUILD OPTIONS")]
    opts: CoreBuildArgs,
}

impl MultibuildArgs {
    /// Returns the flattened [`CoreBuildArgs`]
    pub fn build_args(&self) -> &CoreBuildArgs {
        &self.opts
    }

    /// Returns the `Project` for the current workspace.
    ///
    /// This loads the `foundry_config::Config` for the current workspace (see
    /// [`utils::find_project_root_path`]) merges the cli `BuildArgs` into it, sets the provided
    /// `solc`, and finally it returns [`foundry_config::Config::project()`]
    pub fn project(&self, solc: SolcReq) -> eyre::Result<Project> {
        let mut config: Config = self.build_args().into();
        config.solc = Some(solc);
        Ok(config.project()?)
    }
}

impl Cmd for MultibuildArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        if !RELEASES.2 {
            return Err(eyre::eyre!(
                "Unknown error occurred while accessing the Solidity version list"
            ))
        }

        // Get the sorted list of all Solidity versions.
        let versions: &Vec<Version> = &RELEASES.1;

        // Get the index for `from`.
        let from_index: usize = versions
            .iter()
            .position(|v| v.to_string() == self.from)
            .ok_or_else(|| eyre::eyre!("{} is not a valid Solidity version", self.from))?;

        // Get the index for `to`.
        let to_index: usize = versions
            .iter()
            .position(|v| v.to_string() == self.to)
            .ok_or_else(|| eyre::eyre!("{} is not a valid Solidity version", self.to))?;

        // `to` must be greater than or equal to `from`.
        if from_index > to_index {
            return Err(eyre::eyre!(
                "The `to` version must be greater than or equal to the `from` version"
            ))
        }

        // Run the "build" command over the provided range of Solidity versions. The `try_for_each`
        // iterator applies a fallible function and stops at the first error, returning it,
        // if it encounters one.
        let compiler = ProjectCompiler::default();
        let mut range = from_index..=to_index;
        range.try_for_each(|i| -> eyre::Result<_> {
            let project = self.project(SolcReq::Version(versions[i].clone()))?;
            compiler.compile(&project)?;
            if i != to_index {
                println!();
            }
            Ok(())
        })?;

        Ok(())
    }
}
