use super::*;
use crate::cmd::get_cached_entry_by_name;
use ethers::{
    prelude::{
        artifacts::Libraries, cache::SolFilesCache, ArtifactId, Graph, Project,
        ProjectCompileOutput,
    },
    solc::{
        artifacts::{CompactContractBytecode, ContractBytecodeSome},
        contracts::ArtifactContracts,
        info::ContractInfo,
    },
    types::{Address, U256},
};
use eyre::Context;
use foundry_common::compile;
use foundry_utils::{linker, linker::LinkedArtifact};
use std::{collections::BTreeMap, fs, str::FromStr};
use tracing::{trace, warn};

impl ScriptArgs {
    /// Compiles the file or project and the verify metadata.
    pub fn compile(&mut self, script_config: &mut ScriptConfig) -> eyre::Result<BuildOutput> {
        trace!(target: "script", "compiling script");

        self.build(script_config)
    }

    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&mut self, script_config: &mut ScriptConfig) -> eyre::Result<BuildOutput> {
        let (project, output) = self.get_project_and_output(script_config)?;

        let mut sources: BTreeMap<u32, String> = BTreeMap::new();

        let contracts = output
            .into_artifacts()
            .map(|(id, artifact)| -> eyre::Result<_> {
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
                Ok((id, artifact))
            })
            .collect::<eyre::Result<ArtifactContracts>>()?;

        let mut output = self.link(
            project,
            contracts,
            script_config.config.parsed_libraries()?,
            script_config.evm_opts.sender,
            script_config.sender_nonce,
        )?;

        output.sources = sources;
        script_config.target_contract = Some(output.target.clone());

        Ok(output)
    }

    pub fn link(
        &self,
        project: Project,
        contracts: ArtifactContracts,
        libraries_addresses: Libraries,
        sender: Address,
        nonce: U256,
    ) -> eyre::Result<BuildOutput> {
        let target_file = dunce::canonicalize(&self.path)
            .wrap_err("Couldn't convert contract path to absolute path.")?;

        // todo: extract to a function?
        let mut matches: Vec<_> = contracts
            .keys()
            .filter(|id| {
                if let Some(target_contract_name) = &self.target_contract {
                    &id.name == target_contract_name && id.source == target_file
                } else {
                    id.source == target_file
                }
            })
            .collect();
        let target = match matches.len() {
            0 => eyre::bail!("Could not find target contract: {}", target_file.to_string_lossy()),
            1 => matches.pop().unwrap(),
            _ => eyre::bail!("Multiple contracts in the target path. Please specify the contract name with `--tc ContractName`")
        };
        // todo: end

        // todo: how do we construct highlevel_known_contracts
        let LinkedArtifact { bytecode, dependencies, .. } =
            linker::link_single(&contracts, &libraries_addresses, sender, nonce, target.clone())
                .wrap_err("linking failed")?;

        let mut contract = contracts.get(target).unwrap().clone();
        contract.bytecode = Some(bytecode);

        let mut new_libraries = Vec::with_capacity(dependencies.len());
        for dep in &dependencies {
            println!("- {}", dep); // todo: rm (although it looks nice)

            new_libraries.push(dep.library_line());
        }

        // Merge with user provided libraries
        let mut new_libraries = Libraries::parse(&new_libraries)?;
        for (file, libraries) in libraries_addresses.libs.into_iter() {
            new_libraries.libs.entry(file).or_default().extend(libraries)
        }

        Ok(BuildOutput {
            target: target.clone(),
            contract,
            known_contracts: contracts,
            // todo: what do we do here
            highlevel_known_contracts: ArtifactContracts(BTreeMap::new()),
            predeploy_libraries: dependencies.into_iter().map(|dep| dep.bytecode).collect(),
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

        let filters = self.opts.skip.clone().unwrap_or_default();
        // We received a valid file path.
        // If this file does not exist, `dunce::canonicalize` will
        // result in an error and it will be handled below.
        if let Ok(target_contract) = dunce::canonicalize(&self.path) {
            let output = compile::compile_target_with_filter(
                &target_contract,
                &project,
                self.opts.args.silent,
                self.verify,
                filters,
            )?;
            return Ok((project, output))
        }

        if !project.paths.has_input_files() {
            eyre::bail!("The project doesn't have any input files. Make sure the `script` directory is configured properly in foundry.toml. Otherwise, provide the path to the file.")
        }

        let contract = ContractInfo::from_str(&self.path)?;
        self.target_contract = Some(contract.name.clone());

        // We received `contract_path:contract_name`
        if let Some(path) = contract.path {
            let path =
                dunce::canonicalize(path).wrap_err("Could not canonicalize the target path")?;
            let output = compile::compile_target_with_filter(
                &path,
                &project,
                self.opts.args.silent,
                self.verify,
                filters,
            )?;
            self.path = path.to_string_lossy().to_string();
            return Ok((project, output))
        }

        // We received `contract_name`, and need to find its file path.
        let output = if self.opts.args.silent {
            compile::suppress_compile(&project)
        } else {
            compile::compile(&project, false, false)
        }?;
        let cache =
            SolFilesCache::read_joined(&project.paths).wrap_err("Could not open compiler cache")?;

        let (path, _) = get_cached_entry_by_name(&cache, &contract.name)
            .wrap_err("Could not find target contract in cache")?;
        self.path = path.to_string_lossy().to_string();

        Ok((project, output))
    }
}

/// Resolve the import tree of our target path, and get only the artifacts and
/// sources we need. If it's a standalone script, don't filter anything out.
pub fn filter_sources_and_artifacts(
    target: &str,
    sources: BTreeMap<u32, String>,
    highlevel_known_contracts: ArtifactContracts<ContractBytecodeSome>,
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
                target_tree.get(&resolved).map(|source| (id, source.content.as_str().to_string()))
            } else {
                Some((
                    id,
                    fs::read_to_string(&resolved).unwrap_or_else(|_| {
                        panic!("Something went wrong reading the source file: {path:?}")
                    }),
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

pub struct BuildOutput {
    pub project: Project,
    pub target: ArtifactId,
    pub contract: CompactContractBytecode,
    pub known_contracts: ArtifactContracts,
    pub highlevel_known_contracts: ArtifactContracts<ContractBytecodeSome>,
    pub libraries: Libraries,
    pub predeploy_libraries: Vec<ethers::types::Bytes>,
    pub sources: BTreeMap<u32, String>,
}
