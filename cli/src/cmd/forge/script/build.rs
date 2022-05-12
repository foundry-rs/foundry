use crate::compile;

use ethers::{
    prelude::ArtifactId,
    solc::artifacts::{CompactContractBytecode, ContractBytecode, ContractBytecodeSome},
    types::{Address, U256},
};
use forge::executor::opts::EvmOpts;

use foundry_config::Config;
use foundry_utils::PostLinkInput;
use std::collections::BTreeMap;

use super::*;

impl ScriptArgs {
    /// Compiles the file with auto-detection and compiler params.
    pub fn build(
        &self,
        config: &Config,
        evm_opts: &EvmOpts,
        sender: Option<Address>,
        nonce: U256,
    ) -> eyre::Result<BuildOutput> {
        let target_contract = dunce::canonicalize(&self.path)?;
        let project = config.ephemeral_no_artifacts_project()?;
        let output = compile::compile_files(&project, vec![target_contract])?;

        let (contracts, _sources) = output.into_artifacts_with_sources();
        let contracts: BTreeMap<ArtifactId, CompactContractBytecode> =
            contracts.into_iter().map(|(id, artifact)| (id, artifact.into())).collect();
        self.link(contracts, sender.unwrap_or(evm_opts.sender), nonce)
    }

    pub fn link(
        &self,
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
        })
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
    pub target: ArtifactId,
    pub contract: CompactContractBytecode,
    pub known_contracts: BTreeMap<ArtifactId, CompactContractBytecode>,
    pub highlevel_known_contracts: BTreeMap<ArtifactId, ContractBytecodeSome>,
    pub predeploy_libraries: Vec<ethers::types::Bytes>,
}
