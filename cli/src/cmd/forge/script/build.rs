use crate::{cmd::get_cached_entry_by_name, compile, opts::forge::ContractInfo};
use eyre::{Context, ContextCompat};

use ethers::{
    prelude::{
        artifacts::Libraries, cache::SolFilesCache, ArtifactId, Project, ProjectCompileOutput,
    },
    solc::artifacts::{CompactContractBytecode, ContractBytecode, ContractBytecodeSome},
    types::{Address, U256},
};

use foundry_utils::PostLinkInput;
use std::{collections::BTreeMap, str::FromStr};

use super::*;

impl ScriptArgs {
    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&mut self, script_config: &ScriptConfig) -> eyre::Result<BuildOutput> {
        let (project, output) = self.get_project_and_output(script_config)?;

        let mut contracts: BTreeMap<ArtifactId, CompactContractBytecode> = BTreeMap::new();
        let mut sources: BTreeMap<u32, String> = BTreeMap::new();

        for (id, artifact) in output.into_artifacts() {
            let source = artifact.source_file().ok_or(eyre::eyre!("Artifact has no source."))?;
            sources.insert(
                source.id,
                source.ast.ok_or(eyre::eyre!("Source from artifact has no AST."))?.absolute_path,
            );

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

        Ok(BuildOutput {
            target: extra_info.target_id.expect("Target not found?"),
            contract,
            known_contracts: contracts,
            highlevel_known_contracts,
            predeploy_libraries: run_dependencies,
            sources: BTreeMap::new(),
            project,
        })
    }

    pub fn get_project_and_output(
        &mut self,
        script_config: &ScriptConfig,
    ) -> eyre::Result<(Project, ProjectCompileOutput)> {
        let project = script_config.config.project()?;

        let output = match dunce::canonicalize(&self.path) {
            // We got passed an existing path to the contract
            Ok(target_contract) => compile::compile_files(&project, vec![target_contract])?,
            Err(_) => {
                // We either got passed `contract_path:contract_name` or the contract name.
                let contract = ContractInfo::from_str(&self.path)?;
                let (path, output) = if let Some(path) = contract.path {
                    let output =
                        compile::compile_files(&project, vec![dunce::canonicalize(&path)?])?;

                    (path, output)
                } else {
                    let output = compile::compile(&project, false, false)?;
                    let cache = SolFilesCache::read_joined(&project.paths)?;

                    let res = get_cached_entry_by_name(&cache, &contract.name)?;
                    (res.0.to_str().ok_or(eyre::eyre!("Invalid path string."))?.to_string(), output)
                };

                self.path = path;
                self.target_contract = Some(contract.name);
                output
            }
        };

        // We always compile our contract path, since it's not possible to get srcmaps from cached
        // artifacts
        Ok((project, output))
    }
}

struct ExtraLinkingInfo<'a> {
    no_target_name: bool,
    target_fname: String,
    contract: &'a mut CompactContractBytecode,
    dependencies: &'a mut Vec<ethers::types::Bytes>,
    matched: bool,
    target_id: Option<ArtifactId>,
}

pub struct BuildOutput {
    pub project: Project,
    pub target: ArtifactId,
    pub contract: CompactContractBytecode,
    pub known_contracts: BTreeMap<ArtifactId, CompactContractBytecode>,
    pub highlevel_known_contracts: BTreeMap<ArtifactId, ContractBytecodeSome>,
    pub predeploy_libraries: Vec<ethers::types::Bytes>,
    pub sources: BTreeMap<u32, String>,
}
