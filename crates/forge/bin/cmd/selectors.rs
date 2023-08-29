use clap::Parser;
use comfy_table::Table;
use ethers::prelude::{artifacts::output_selection::ContractOutputSelection, info::ContractInfo};
use eyre::Result;
use foundry_cli::{
    opts::{CompilerArgs, CoreBuildArgs, ProjectPathsArgs},
    utils::FoundryPathExt,
};
use foundry_common::{
    compile::ProjectCompiler,
    selectors::{import_selectors, SelectorImportData},
};
use std::fs::canonicalize;

/// CLI arguments for `forge selectors`.
#[derive(Debug, Clone, Parser)]
pub enum SelectorsSubcommands {
    /// Check for selector collisions between contracts
    #[clap(visible_alias = "co")]
    Collision {
        /// The first of the two contracts for which to look selector collisions for, in the form
        /// `(<path>:)?<contractname>`.
        first_contract: ContractInfo,

        /// The second of the two contracts for which to look selector collisions for, in the form
        /// `(<path>:)?<contractname>`.
        second_contract: ContractInfo,

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
    pub async fn run(self) -> Result<()> {
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
                let output = ProjectCompiler::new().quiet(true).compile(&project)?;
                let artifacts = if all {
                    output
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
                    let found_artifact = output.find_first(&contract);
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

                    sh_status!("Uploading" => "{contract}")?;

                    // upload abi to selector database
                    import_selectors(SelectorImportData::Abi(vec![abi])).await?.describe()?;

                    if artifacts.peek().is_some() {
                        sh_eprintln!()?;
                    }
                }
            }
            SelectorsSubcommands::Collision { mut first_contract, mut second_contract, build } => {
                // Compile the project with the two contracts included
                let project = build.project()?;
                let mut compiler = ProjectCompiler::new().quiet(true);

                if let Some(contract_path) = &mut first_contract.path {
                    let target_path = canonicalize(&*contract_path)?;
                    *contract_path = target_path.to_string_lossy().to_string();
                    compiler = compiler.files([target_path]);
                }
                if let Some(contract_path) = &mut second_contract.path {
                    let target_path = canonicalize(&*contract_path)?;
                    *contract_path = target_path.to_string_lossy().to_string();
                    compiler = compiler.files([target_path]);
                }

                let output = compiler.compile(&project)?;

                // Check method selectors for collisions
                let methods = |contract: &ContractInfo| -> eyre::Result<_> {
                    let artifact = output
                        .find_contract(contract)
                        .ok_or_else(|| eyre::eyre!("Could not find artifact for {contract}"))?;
                    artifact.method_identifiers.as_ref().ok_or_else(|| {
                        eyre::eyre!("Could not find method identifiers for {contract}")
                    })
                };
                let first_method_map = methods(&first_contract)?;
                let second_method_map = methods(&second_contract)?;

                let colliding_methods: Vec<(&String, &String, &String)> = first_method_map
                    .iter()
                    .filter_map(|(k1, v1)| {
                        second_method_map
                            .iter()
                            .find_map(|(k2, v2)| if **v2 == *v1 { Some((k2, v2)) } else { None })
                            .map(|(k2, v2)| (v2, k1, k2))
                    })
                    .collect();

                if colliding_methods.is_empty() {
                    sh_println!("No colliding method selectors between the two contracts.")?;
                } else {
                    let mut table = Table::new();
                    table.set_header(vec![
                        String::from("Selector"),
                        first_contract.name,
                        second_contract.name,
                    ]);
                    for &t in &colliding_methods {
                        table.add_row(vec![t.0, t.1, t.2]);
                    }
                    sh_println!("{} collisions found:\n{table}", colliding_methods.len())?;
                }
            }
        }
        Ok(())
    }
}
