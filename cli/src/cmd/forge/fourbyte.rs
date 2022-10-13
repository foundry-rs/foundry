use crate::{
    cmd::forge::build::{CoreBuildArgs, ProjectPathsArgs},
    opts::forge::CompilerArgs,
};
use clap::Parser;
use ethers::prelude::artifacts::output_selection::ContractOutputSelection;
use foundry_common::{
    compile,
    selectors::{import_selectors, SelectorImportData},
};

#[derive(Debug, Clone, Parser)]
pub struct UploadSelectorsArgs {
    #[clap(help = "The name of the contract to upload selectors for.")]
    pub contract: String,

    #[clap(flatten, next_help_heading = "PROJECT OPTIONS")]
    pub project_paths: ProjectPathsArgs,
}

impl UploadSelectorsArgs {
    /// Builds a contract and uploads the ABI to selector database
    pub async fn run(self) -> eyre::Result<()> {
        let UploadSelectorsArgs { contract, project_paths } = self;

        let build_args = CoreBuildArgs {
            project_paths: project_paths.clone(),
            compiler: CompilerArgs {
                extra_output: vec![ContractOutputSelection::Abi],
                ..Default::default()
            },
            ..Default::default()
        };

        let project = build_args.project()?;
        let outcome = compile::suppress_compile(&project)?;
        let found_artifact = outcome.find_first(&contract);
        let artifact = found_artifact.ok_or_else(|| {
            eyre::eyre!("Could not find artifact `{contract}` in the compiled artifacts")
        })?;

        let import_data = SelectorImportData::Abi(vec![artifact
            .abi
            .clone()
            .ok_or(eyre::eyre!("Unable to fetch abi"))?]);

        // upload abi to selector database
        import_selectors(import_data).await?.describe();

        Ok(())
    }
}
