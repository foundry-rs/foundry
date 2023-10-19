use clap::Parser;
use ethers::prelude::artifacts::output_selection::ContractOutputSelection;
use eyre::Result;
use foundry_cli::{
    opts::{CompilerArgs, CoreBuildArgs, ProjectPathsArgs},
    utils::FoundryPathExt,
};
use foundry_common::{
    compile,
    selectors::{import_selectors, SelectorImportData},
    shell,
};
use foundry_compilers::artifacts::output_selection::ContractOutputSelection;
use yansi::Paint;

/// CLI arguments for `forge upload-selectors`.
#[derive(Debug, Clone, Parser)]
pub struct UploadSelectorsArgs {
    /// The name of the contract to upload selectors for.
    #[clap(required_unless_present = "all")]
    pub contract: Option<String>,

    /// Upload selectors for all contracts in the project.
    #[clap(long, required_unless_present = "contract")]
    pub all: bool,

    #[clap(flatten)]
    pub project_paths: ProjectPathsArgs,
}

impl UploadSelectorsArgs {
    /// Builds a contract and uploads the ABI to selector database
    pub async fn run(self) -> Result<()> {
        shell::println(Paint::yellow("Warning! This command is deprecated and will be removed in v1, use `forge selectors upload` instead"))?;

        let UploadSelectorsArgs { contract, all, project_paths } = self;

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
        let artifacts = if all {
            outcome
                .into_artifacts_with_files()
                .filter(|(file, _, _)| {
                    let is_sources_path =
                        file.starts_with(&project.paths.sources.to_string_lossy().to_string());
                    let is_test = file.is_sol_test();

                    is_sources_path && !is_test
                })
                .map(|(_, contract, artifact)| (contract, artifact))
                .collect()
        } else {
            let contract = contract.unwrap();
            let found_artifact = outcome.find_first(&contract);
            let artifact = found_artifact
                .ok_or_else(|| {
                    eyre::eyre!("Could not find artifact `{contract}` in the compiled artifacts")
                })?
                .clone();
            vec![(contract, artifact)]
        };

        let mut artifacts = artifacts.into_iter().peekable();
        while let Some((contract, artifact)) = artifacts.next() {
            let abi = artifact.abi.ok_or(eyre::eyre!("Unable to fetch abi"))?;
            if abi.abi.functions.is_empty() &&
                abi.abi.events.is_empty() &&
                abi.abi.errors.is_empty()
            {
                continue
            }

            println!("Uploading selectors for {contract}...");

            // upload abi to selector database
            import_selectors(SelectorImportData::Abi(vec![abi])).await?.describe();

            if artifacts.peek().is_some() {
                println!()
            }
        }

        Ok(())
    }
}
