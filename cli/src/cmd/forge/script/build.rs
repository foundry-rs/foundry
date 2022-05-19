use crate::{cmd::get_cached_entry_by_name, compile, opts::forge::ContractInfo};

use ethers::{
    prelude::{cache::SolFilesCache, ArtifactId, Project, ProjectCompileOutput},
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

        let (contracts, sources) = output.into_artifacts_with_sources();
        let contracts: BTreeMap<ArtifactId, CompactContractBytecode> =
            contracts.into_iter().map(|(id, artifact)| (id, artifact.into())).collect();

        let mut output = self.link(
            project,
            contracts,
            script_config.evm_opts.sender,
            script_config.sender_nonce,
        )?;
        output.sources = sources.into_ids().collect();
        Ok(output)
    }

    pub fn link(
        &self,
        project: Project,
        contracts: BTreeMap<ArtifactId, CompactContractBytecode>,
        sender: Address,
        nonce: U256,
    ) -> eyre::Result<BuildOutput> {
        let mut run_dependencies = vec![];
        let mut contract =
            CompactContractBytecode { abi: None, bytecode: None, deployed_bytecode: None };
        let mut highlevel_known_contracts = BTreeMap::new();

        let mut target_fname = dunce::canonicalize(&self.path)
            .expect("Couldn't convert contract path to absolute path")
            .to_str()
            .expect("Bad path to string")
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
        foundry_utils::link_with_nonce(
            contracts.clone(),
            &mut highlevel_known_contracts,
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

        let project = script_config.config.ephemeral_no_artifacts_project()?;

        let path = match dunce::canonicalize(&self.path) {
            Ok(target_contract) => target_contract,
            Err(_) => {
                let contract = ContractInfo::from_str(&self.path)?;
                let path = if let Some(path) = contract.path {
                    path
                } else {
                    let project = script_config.config.project()?;
                    compile::compile(&project, false, false)?;
                    let cache = SolFilesCache::read_joined(&project.paths)?;

                    let res = get_cached_entry_by_name(&cache, &contract.name)?;
                    res.0.to_str().expect("Invalid path string.").to_string()
                };

                self.path = path;
                self.target_contract = Some(contract.name);
                dunce::canonicalize(&self.path)?
            }
        };

        // We always compile our contract path, since it's not possible to get srcmaps from cached artifacts
        let output = compile::compile_files(&project, vec![path])?;
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
