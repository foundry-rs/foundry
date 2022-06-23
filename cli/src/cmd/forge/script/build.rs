use crate::{
    cmd::{get_cached_entry_by_name, unwrap_contracts},
    compile,
};
use ethers::{
    prelude::{
        artifacts::Libraries, cache::SolFilesCache, ArtifactId, Graph, Project,
        ProjectCompileOutput,
    },
    solc::{
        artifacts::{CompactContractBytecode, ContractBytecode, ContractBytecodeSome},
        info::ContractInfo,
    },
    types::{Address, U256},
};
use eyre::{Context, ContextCompat};
use foundry_utils::PostLinkInput;
use std::{collections::BTreeMap, fs, str::FromStr};
use tracing::warn;

use super::*;

impl ScriptArgs {
    /// Compiles the file or project and the verify metadata.
    pub fn compile(
        &mut self,
        script_config: &ScriptConfig,
    ) -> eyre::Result<(BuildOutput, VerifyBundle)> {
        let build_output = self.build(script_config)?;

        let verify = VerifyBundle::new(
            &build_output.project,
            &script_config.config,
            unwrap_contracts(&build_output.highlevel_known_contracts, false),
        );

        Ok((build_output, verify))
    }

    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&mut self, script_config: &ScriptConfig) -> eyre::Result<BuildOutput> {
        let (project, output) = self.get_project_and_output(script_config)?;

        let mut contracts: BTreeMap<ArtifactId, CompactContractBytecode> = BTreeMap::new();
        let mut sources: BTreeMap<u32, String> = BTreeMap::new();

        for (id, artifact) in output.into_artifacts() {
            // Sources are only required for the debugger, but it *might* mean that there's
            // something wrong with the build and/or artifacts.
            if let Some(source) = artifact.source_file() {
                sources.insert(
                    source.id,
                    source
                        .ast
                        .ok_or(eyre::eyre!("Source from artifact has no AST."))?
                        .absolute_path,
                );
            } else {
                warn!("source not found for artifact={:?}", id);
            }
            contracts.insert(id, artifact.into());
        }

        let mut output = self.link(
            project,
            contracts,
            script_config.config.parsed_libraries()?,
            script_config.evm_opts.sender,
            script_config.sender_nonce,
        )?;
        output.sources = sources;
        Ok(output)
    }

    pub fn link(
        &self,
        project: Project,
        contracts: BTreeMap<ArtifactId, CompactContractBytecode>,
        libraries_addresses: Libraries,
        sender: Address,
        nonce: U256,
    ) -> eyre::Result<BuildOutput> {
        let mut run_dependencies = vec![];
        let mut contract = CompactContractBytecode::default();
        let mut highlevel_known_contracts = BTreeMap::new();

        let mut target_fname = dunce::canonicalize(&self.path)
            .wrap_err("Couldn't convert contract path to absolute path.")?
            .to_str()
            .wrap_err("Bad path to string.")?
            .to_string();

        let no_target_name = if let Some(target_name) = &self.target_contract {
            target_fname = target_fname + ":" + target_name;
            false
        } else {
            true
        };

        let mut extra_info = ExtraLinkingInfo {
            no_target_name,
            target_fname,
            contract: &mut contract,
            dependencies: &mut run_dependencies,
            matched: false,
            target_id: None,
        };

        // link_with_nonce_or_address expects absolute paths
        let mut libs = libraries_addresses.clone();
        for (file, libraries) in libraries_addresses.libs.iter() {
            if file.is_relative() {
                let mut absolute_path = project.root().clone();
                absolute_path.push(file);
                libs.libs.insert(absolute_path, libraries.clone());
            }
        }

        foundry_utils::link_with_nonce_or_address(
            contracts.clone(),
            &mut highlevel_known_contracts,
            libs,
            sender,
            nonce,
            &mut extra_info,
            |file, key| (format!("{}.json:{}", key, key), file, key),
            |post_link_input| {
                let PostLinkInput {
                    contract,
                    known_contracts: highlevel_known_contracts,
                    id,
                    extra,
                    dependencies,
                } = post_link_input;

                // if it's the target contract, grab the info
                if extra.no_target_name {
                    if id.source == std::path::Path::new(&extra.target_fname) {
                        if extra.matched {
                            eyre::bail!("Multiple contracts in the target path. Please specify the contract name with `--tc ContractName`")
                        }
                        *extra.dependencies = dependencies;
                        *extra.contract = contract.clone();
                        extra.matched = true;
                        extra.target_id = Some(id.clone());
                    }
                } else {
                    let split: Vec<&str> = extra.target_fname.split(':').collect();
                    let path = std::path::Path::new(split[0]);
                    let name = split[1];
                    if path == id.source && name == id.name {
                        *extra.dependencies = dependencies;
                        *extra.contract = contract.clone();
                        extra.matched = true;
                        extra.target_id = Some(id.clone());
                    }
                }

                let tc: ContractBytecode = contract.into();
                highlevel_known_contracts.insert(id, tc.unwrap());
                Ok(())
            },
        )?;

        let target = extra_info.target_id.expect("Target not found?");

        let (new_libraries, predeploy_libraries): (Vec<_>, Vec<_>) =
            run_dependencies.into_iter().unzip();

        // Merge with user provided libraries
        let mut new_libraries = Libraries::parse(&new_libraries)?;
        for (file, libraries) in libraries_addresses.libs.into_iter() {
            new_libraries.libs.entry(file).or_insert(BTreeMap::new()).extend(libraries.into_iter())
        }

        Ok(BuildOutput {
            target,
            contract,
            known_contracts: contracts,
            highlevel_known_contracts,
            predeploy_libraries,
            sources: BTreeMap::new(),
            project,
            libraries: new_libraries,
        })
    }

    pub fn get_project_and_output(
        &mut self,
        script_config: &ScriptConfig,
    ) -> eyre::Result<(Project, ProjectCompileOutput)> {
        let project = script_config.config.project()?;

        // We received a file path.
        if let Ok(target_contract) = dunce::canonicalize(&self.path) {
            let output = compile::compile_target(
                &target_contract,
                &project,
                self.opts.args.silent,
                self.verify,
            )?;
            return Ok((project, output))
        }

        let contract = ContractInfo::from_str(&self.path)?;
        self.target_contract = Some(contract.name.clone());

        // We received `contract_path:contract_name`
        if let Some(path) = contract.path {
            let path = dunce::canonicalize(&path)?;
            let output =
                compile::compile_target(&path, &project, self.opts.args.silent, self.verify)?;
            self.path = path.to_string_lossy().to_string();
            return Ok((project, output))
        }

        // We received `contract_name`, and need to find out its file path.
        let output = if self.opts.args.silent {
            compile::suppress_compile(&project)
        } else {
            compile::compile(&project, false, false)
        }?;
        let cache = SolFilesCache::read_joined(&project.paths)?;

        let (path, _) = get_cached_entry_by_name(&cache, &contract.name)?;
        self.path = path.to_string_lossy().to_string();

        Ok((project, output))
    }
}

/// Resolve the import tree of our target path, and get only the artifacts and
/// sources we need. If it's a standalone script, don't filter anything out.
pub fn filter_sources_and_artifacts(
    target: &str,
    sources: BTreeMap<u32, String>,
    highlevel_known_contracts: BTreeMap<ArtifactId, ContractBytecodeSome>,
    project: Project,
) -> eyre::Result<(BTreeMap<u32, String>, HashMap<String, ContractBytecodeSome>)> {
    // Find all imports
    let graph = Graph::resolve(&project.paths)?;
    let target_path = project.root().join(target);
    let mut target_tree = BTreeMap::new();
    let mut is_standalone = false;

    if let Some(target_index) = graph.files().get(&target_path) {
        target_tree.extend(
            graph
                .all_imported_nodes(*target_index)
                .map(|index| graph.node(index).unpack())
                .collect::<BTreeMap<_, _>>(),
        );

        // Add our target into the tree as well.
        let (target_path, target_source) = graph.node(*target_index).unpack();
        target_tree.insert(target_path, target_source);
    } else {
        is_standalone = true;
    }

    let sources = sources
        .into_iter()
        .filter_map(|(id, path)| {
            let mut resolved = project
                .paths
                .resolve_library_import(&PathBuf::from(&path))
                .unwrap_or_else(|| PathBuf::from(&path));

            if !resolved.is_absolute() {
                resolved = project.root().join(&resolved);
            }

            if !is_standalone {
                target_tree.get(&resolved).map(|source| (id, source.content.clone()))
            } else {
                Some((
                    id,
                    fs::read_to_string(&resolved).expect(&*format!(
                        "Something went wrong reading the source file: {:?}",
                        path
                    )),
                ))
            }
        })
        .collect();

    let artifacts = highlevel_known_contracts
        .into_iter()
        .filter_map(|(id, artifact)| {
            if !is_standalone {
                target_tree.get(&id.source).map(|_| (id.name, artifact))
            } else {
                Some((id.name, artifact))
            }
        })
        .collect();

    Ok((sources, artifacts))
}

struct ExtraLinkingInfo<'a> {
    no_target_name: bool,
    target_fname: String,
    contract: &'a mut CompactContractBytecode,
    dependencies: &'a mut Vec<(String, ethers::types::Bytes)>,
    matched: bool,
    target_id: Option<ArtifactId>,
}

pub struct BuildOutput {
    pub project: Project,
    pub target: ArtifactId,
    pub contract: CompactContractBytecode,
    pub known_contracts: BTreeMap<ArtifactId, CompactContractBytecode>,
    pub highlevel_known_contracts: BTreeMap<ArtifactId, ContractBytecodeSome>,
    pub libraries: Libraries,
    pub predeploy_libraries: Vec<ethers::types::Bytes>,
    pub sources: BTreeMap<u32, String>,
}
