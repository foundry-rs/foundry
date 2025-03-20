use alloy_primitives::hex;
use clap::Parser;
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, Table};
use eyre::Result;
use foundry_cli::{
    opts::{BuildOpts, CompilerOpts, ProjectPathOpts},
    utils::{cache_local_signatures, FoundryPathExt},
};
use foundry_common::{
    compile::{compile_target, PathOrContractInfo, ProjectCompiler},
    selectors::{import_selectors, SelectorImportData},
};
use foundry_compilers::{artifacts::output_selection::ContractOutputSelection, info::ContractInfo};
use foundry_config::Config;
use std::fs::canonicalize;

/// CLI arguments for `forge selectors`.
#[derive(Clone, Debug, Parser)]
pub enum SelectorsSubcommands {
    /// Check for selector collisions between contracts
    #[command(visible_alias = "co")]
    Collision {
        /// The first of the two contracts for which to look selector collisions for, in the form
        /// `(<path>:)?<contractname>`.
        first_contract: ContractInfo,

        /// The second of the two contracts for which to look selector collisions for, in the form
        /// `(<path>:)?<contractname>`.
        second_contract: ContractInfo,

        #[command(flatten)]
        build: Box<BuildOpts>,
    },

    /// Upload selectors to registry
    #[command(visible_alias = "up")]
    Upload {
        /// The name of the contract to upload selectors for.
        /// Can also be in form of `path:contract name`.
        #[arg(required_unless_present = "all")]
        contract: Option<PathOrContractInfo>,

        /// Upload selectors for all contracts in the project.
        #[arg(long, required_unless_present = "contract")]
        all: bool,

        #[command(flatten)]
        project_paths: ProjectPathOpts,
    },

    /// List selectors from current workspace
    #[command(visible_alias = "ls")]
    List {
        /// The name of the contract to list selectors for.
        #[arg(help = "The name of the contract to list selectors for.")]
        contract: Option<String>,

        #[command(flatten)]
        project_paths: ProjectPathOpts,
    },

    /// Find if a selector is present in the project
    #[command(visible_alias = "f")]
    Find {
        /// The selector to search for
        #[arg(help = "The selector to search for (with or without 0x prefix)")]
        selector: String,

        #[command(flatten)]
        project_paths: ProjectPathOpts,
    },

    /// Cache project selectors (enables trace with local contracts functions and events).
    #[command(visible_alias = "c")]
    Cache {
        #[command(flatten)]
        project_paths: ProjectPathOpts,
    },
}

impl SelectorsSubcommands {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Cache { project_paths } => {
                sh_println!("Caching selectors for contracts in the project...")?;
                let build_args = BuildOpts {
                    project_paths,
                    compiler: CompilerOpts {
                        extra_output: vec![ContractOutputSelection::Abi],
                        ..Default::default()
                    },
                    ..Default::default()
                };

                // compile the project to get the artifacts/abis
                let project = build_args.project()?;
                let outcome = ProjectCompiler::new().quiet(true).compile(&project)?;
                cache_local_signatures(&outcome, Config::foundry_cache_dir().unwrap())?
            }
            Self::Upload { contract, all, project_paths } => {
                let build_args = BuildOpts {
                    project_paths: project_paths.clone(),
                    compiler: CompilerOpts {
                        extra_output: vec![ContractOutputSelection::Abi],
                        ..Default::default()
                    },
                    ..Default::default()
                };

                let project = build_args.project()?;
                let output = if let Some(contract_info) = &contract {
                    let Some(contract_name) = contract_info.name() else {
                        eyre::bail!("No contract name provided.")
                    };

                    let target_path = contract_info
                        .path()
                        .map(Ok)
                        .unwrap_or_else(|| project.find_contract_path(contract_name))?;
                    compile_target(&target_path, &project, false)?
                } else {
                    ProjectCompiler::new().compile(&project)?
                };
                let artifacts = if all {
                    output
                        .into_artifacts_with_files()
                        .filter(|(file, _, _)| {
                            let is_sources_path = file.starts_with(&project.paths.sources);
                            let is_test = file.is_sol_test();

                            is_sources_path && !is_test
                        })
                        .map(|(_, contract, artifact)| (contract, artifact))
                        .collect()
                } else {
                    let contract_info = contract.unwrap();
                    let contract = contract_info.name().unwrap().to_string();

                    let found_artifact = if let Some(path) = contract_info.path() {
                        output.find(project.root().join(path).as_path(), &contract)
                    } else {
                        output.find_first(&contract)
                    };

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
                    let abi = artifact.abi.ok_or_else(|| eyre::eyre!("Unable to fetch abi"))?;
                    if abi.functions.is_empty() && abi.events.is_empty() && abi.errors.is_empty() {
                        continue
                    }

                    sh_println!("Uploading selectors for {contract}...")?;

                    // upload abi to selector database
                    import_selectors(SelectorImportData::Abi(vec![abi])).await?.describe();

                    if artifacts.peek().is_some() {
                        sh_println!()?
                    }
                }
            }
            Self::Collision { mut first_contract, mut second_contract, build } => {
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
                    table.apply_modifier(UTF8_ROUND_CORNERS);
                    table.set_header([
                        String::from("Selector"),
                        first_contract.name,
                        second_contract.name,
                    ]);
                    for method in colliding_methods.iter() {
                        table.add_row([method.0, method.1, method.2]);
                    }
                    sh_println!("{} collisions found:", colliding_methods.len())?;
                    sh_println!("\n{table}\n")?;
                }
            }
            Self::List { contract, project_paths } => {
                sh_println!("Listing selectors for contracts in the project...")?;
                let build_args = BuildOpts {
                    project_paths,
                    compiler: CompilerOpts {
                        extra_output: vec![ContractOutputSelection::Abi],
                        ..Default::default()
                    },
                    ..Default::default()
                };

                // compile the project to get the artifacts/abis
                let project = build_args.project()?;
                let outcome = ProjectCompiler::new().quiet(true).compile(&project)?;
                let artifacts = if let Some(contract) = contract {
                    let found_artifact = outcome.find_first(&contract);
                    let artifact = found_artifact
                        .ok_or_else(|| {
                            let candidates = outcome
                                .artifacts()
                                .map(|(name, _,)| name)
                                .collect::<Vec<_>>();
                            let suggestion = if let Some(suggestion) = foundry_cli::utils::did_you_mean(&contract, candidates).pop() {
                                format!("\nDid you mean `{suggestion}`?")
                            } else {
                                String::new()
                            };
                            eyre::eyre!(
                                "Could not find artifact `{contract}` in the compiled artifacts{suggestion}",
                            )
                        })?
                        .clone();
                    vec![(contract, artifact)]
                } else {
                    outcome
                        .into_artifacts_with_files()
                        .filter(|(file, _, _)| {
                            let is_sources_path = file.starts_with(&project.paths.sources);
                            let is_test = file.is_sol_test();

                            is_sources_path && !is_test
                        })
                        .map(|(_, contract, artifact)| (contract, artifact))
                        .collect()
                };

                let mut artifacts = artifacts.into_iter().peekable();

                while let Some((contract, artifact)) = artifacts.next() {
                    let abi = artifact.abi.ok_or_else(|| eyre::eyre!("Unable to fetch abi"))?;
                    if abi.functions.is_empty() && abi.events.is_empty() && abi.errors.is_empty() {
                        continue
                    }

                    sh_println!("{contract}")?;

                    let mut table = Table::new();
                    table.apply_modifier(UTF8_ROUND_CORNERS);

                    table.set_header(["Type", "Signature", "Selector"]);

                    for func in abi.functions() {
                        let sig = func.signature();
                        let selector = func.selector();
                        table.add_row(["Function", &sig, &hex::encode_prefixed(selector)]);
                    }

                    for event in abi.events() {
                        let sig = event.signature();
                        let selector = event.selector();
                        table.add_row(["Event", &sig, &hex::encode_prefixed(selector)]);
                    }

                    for error in abi.errors() {
                        let sig = error.signature();
                        let selector = error.selector();
                        table.add_row(["Error", &sig, &hex::encode_prefixed(selector)]);
                    }

                    sh_println!("\n{table}\n")?;

                    if artifacts.peek().is_some() {
                        sh_println!()?
                    }
                }
            }

            Self::Find { selector, project_paths } => {
                sh_println!("Searching for selector {selector:?} in the project...")?;

                let build_args = BuildOpts {
                    project_paths,
                    compiler: CompilerOpts {
                        extra_output: vec![ContractOutputSelection::Abi],
                        ..Default::default()
                    },
                    ..Default::default()
                };

                let project = build_args.project()?;
                let outcome = ProjectCompiler::new().quiet(true).compile(&project)?;
                let artifacts = outcome
                    .into_artifacts_with_files()
                    .filter(|(file, _, _)| {
                        let is_sources_path = file.starts_with(&project.paths.sources);
                        let is_test = file.is_sol_test();
                        is_sources_path && !is_test
                    })
                    .collect::<Vec<_>>();

                let mut table = Table::new();
                table.apply_modifier(UTF8_ROUND_CORNERS);

                table.set_header(["Type", "Signature", "Selector", "Contract"]);

                for (_file, contract, artifact) in artifacts {
                    let abi = artifact.abi.ok_or_else(|| eyre::eyre!("Unable to fetch abi"))?;

                    let selector_bytes =
                        hex::decode(selector.strip_prefix("0x").unwrap_or(&selector))?;

                    for func in abi.functions() {
                        if func.selector().as_slice().starts_with(selector_bytes.as_slice()) {
                            table.add_row([
                                "Function",
                                &func.signature(),
                                &hex::encode_prefixed(func.selector()),
                                contract.as_str(),
                            ]);
                        }
                    }

                    for event in abi.events() {
                        if event.selector().as_slice().starts_with(selector_bytes.as_slice()) {
                            table.add_row([
                                "Event",
                                &event.signature(),
                                &hex::encode_prefixed(event.selector()),
                                contract.as_str(),
                            ]);
                        }
                    }

                    for error in abi.errors() {
                        if error.selector().as_slice().starts_with(selector_bytes.as_slice()) {
                            table.add_row([
                                "Error",
                                &error.signature(),
                                &hex::encode_prefixed(error.selector()),
                                contract.as_str(),
                            ]);
                        }
                    }
                }

                if table.row_count() > 0 {
                    sh_println!("\nFound {} instance(s)...", table.row_count())?;
                    sh_println!("\n{table}\n")?;
                } else {
                    return Err(eyre::eyre!("\nSelector not found in the project."));
                }
            }
        }
        Ok(())
    }
}
