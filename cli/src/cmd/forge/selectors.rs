use crate::{
    cmd::forge::build::{CoreBuildArgs, ProjectPathsArgs},
    opts::forge::CompilerArgs,
    utils::FoundryPathExt,
};
use clap::Parser;
use ethers::prelude::artifacts::output_selection::ContractOutputSelection;
use foundry_common::{
    compile,
    selectors::{import_selectors, SelectorImportData},
};

/// CLI arguments for `forge selectors`.
#[derive(Debug, Clone, Parser)]
pub enum SelectorsSubcommands {
    /// Upload selectors to registry
    #[clap(visible_alias = "up")]
    Upload {
        /// The name of the contract to upload selectors for.
        #[clap(required_unless_present = "all")]
        contract: Option<String>,

        /// Upload selectors for all contracts in the project.
        #[clap(long, required_unless_present = "contract")]
        all: bool,

        #[clap(flatten)]
        project_paths: ProjectPathsArgs,
    },
}

impl SelectorsSubcommands {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            SelectorsSubcommands::Upload { contract, all, project_paths } => {
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
                            let is_sources_path = file
                                .starts_with(&project.paths.sources.to_string_lossy().to_string());
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
                            eyre::eyre!(
                                "Could not find artifact `{contract}` in the compiled artifacts"
                            )
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
            }
        }
        Ok(())
    }
}
