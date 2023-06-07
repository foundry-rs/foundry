use std::fs::canonicalize;

use crate::{
    cmd::forge::build::{CoreBuildArgs, ProjectPathsArgs},
    opts::forge::CompilerArgs,
    utils::FoundryPathExt,
};
use clap::Parser;
use ethers::prelude::{artifacts::output_selection::ContractOutputSelection, info::ContractInfo};
use foundry_common::{
    compile,
    selectors::{import_selectors, SelectorImportData},
};

/// CLI arguments for `forge selectors`.
#[derive(Debug, Clone, Parser)]
pub enum SelectorsSubcommands {
    /// Check for selector collisions between contracts
    #[clap(visible_alias = "co")]
    Collision {
        /// First contract
        #[clap(
            help = "The first of the two contracts for which to look selector collisions for, in the form `(<path>:)?<contractname>`",
            value_name = "FIRST_CONTRACT"
        )]
        first_contract: ContractInfo,

        /// Second contract
        #[clap(
            help = "The second of the two contracts for which to look selector collisions for, in the form `(<path>:)?<contractname>`",
            value_name = "SECOND_CONTRACT"
        )]
        second_contract: ContractInfo,

        /// Support build args
        #[clap(flatten)]
        build: Box<CoreBuildArgs>,
    },

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
            SelectorsSubcommands::Collision { mut first_contract, mut second_contract, build } => {
                // Build first project
                let first_project = build.project()?;
                let first_outcome = if let Some(ref mut contract_path) = first_contract.path {
                    let target_path = canonicalize(&*contract_path)?;
                    *contract_path = target_path.to_string_lossy().to_string();
                    compile::compile_files(&first_project, vec![target_path], true)
                } else {
                    compile::suppress_compile(&first_project)
                }?;

                // Build second project
                let second_project = build.project()?;
                let second_outcome = if let Some(ref mut contract_path) = second_contract.path {
                    let target_path = canonicalize(&*contract_path)?;
                    *contract_path = target_path.to_string_lossy().to_string();
                    compile::compile_files(&second_project, vec![target_path], true)
                } else {
                    compile::suppress_compile(&second_project)
                }?;

                // Find the artifacts
                let first_found_artifact = first_outcome.find_contract(&first_contract);
                let second_found_artifact = second_outcome.find_contract(&second_contract);

                // Unwrap inner artifacts
                let first_artifact = first_found_artifact.ok_or_else(|| {
                    eyre::eyre!("Failed to extract first artifact bytecode as a string")
                })?;
                let second_artifact = second_found_artifact.ok_or_else(|| {
                    eyre::eyre!("Failed to extract second artifact bytecode as a string")
                })?;

                // Check method selectors for collisions
                let first_method_map = first_artifact.method_identifiers.as_ref().unwrap();
                let second_method_map = second_artifact.method_identifiers.as_ref().unwrap();

                let mut colliding_methods = Vec::new();
                for (k1, v1) in first_method_map {
                    if let Some(k2) =
                        second_method_map
                            .iter()
                            .find_map(|(k2, v2)| if v1 == v2 { Some(k2) } else { None })
                    {
                        colliding_methods.push((k1, k2));
                    }
                }

                if colliding_methods.is_empty() {
                    println!("No colliding method selectors between the two contracts.");
                } else {
                    colliding_methods.iter().for_each(|t| {
                        println!("\"{}\": {:?}", first_method_map.get(t.0).unwrap(), t)
                    });
                    return Err(eyre::eyre!("At least one collision was found"))
                }
            }
        }
        Ok(())
    }
}
