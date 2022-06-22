use crate::{
    cmd::{get_cached_entry_by_name, unwrap_contracts},
    compile,
    opts::forge::ContractInfo,
};
use ethers::{
    prelude::{
        artifacts::Libraries, cache::SolFilesCache, ArtifactId, Graph, Project,
        ProjectCompileOutput, ProjectPathsConfig,
    },
    solc::artifacts::{CompactContractBytecode, ContractBytecode, ContractBytecodeSome},
    types::{Address, U256},
};
use eyre::{Context, ContextCompat};
use foundry_utils::PostLinkInput;
use std::{collections::BTreeMap, str::FromStr};
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

        let output = match dunce::canonicalize(&self.path) {
            // We got passed an existing path to the contract
            Ok(target_contract) => {
                self.standalone_check(&target_contract, &project.paths)?;

                compile::compile_files(&project, vec![target_contract], self.opts.args.silent)?
            }
            Err(_) => {
                // We either got passed `contract_path:contract_name` or the contract name.
                let contract = ContractInfo::from_str(&self.path)?;
                let (path, output) = if let Some(path) = contract.path {
                    let path = dunce::canonicalize(&path)?;

                    self.standalone_check(&path, &project.paths)?;

                    let output = compile::compile_files(
                        &project,
                        vec![path.clone()],
                        self.opts.args.silent,
                    )?;

                    (path, output)
                } else {
                    let output = if self.opts.args.silent {
                        compile::suppress_compile(&project)
                    } else {
                        compile::compile(&project, false, false)
                    }?;
                    let cache = SolFilesCache::read_joined(&project.paths)?;

                    let res = get_cached_entry_by_name(&cache, &contract.name)?;
                    (res.0, output)
                };

                self.path = path.to_string_lossy().to_string();
                self.target_contract = Some(contract.name);
                output
            }
        };

        // We always compile our contract path, since it's not possible to get srcmaps from cached
        // artifacts
        Ok((project, output))
    }

    /// Throws an error if `target` is a standalone script and `verify` is requested. We only allow
    /// `verify` inside projects.
    fn standalone_check(
        &self,
        target_contract: &PathBuf,
        project_paths: &ProjectPathsConfig,
    ) -> eyre::Result<()> {
        let graph = Graph::resolve(project_paths)?;
        if graph.files().get(target_contract).is_none() && self.verify {
            eyre::bail!("You can only verify deployments from inside a project! Make sure it exists with `forge tree`.");
        }
        Ok(())
    }
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
