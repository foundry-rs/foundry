//! Verify support

use crate::cmd::{
    forge::{
        build::ProjectPathsArgs,
        verify::{VerifierArgs, VerifyArgs},
    },
    retry::RetryArgs,
};
use ethers::{
    abi::Address,
    solc::{info::ContractInfo, Project},
};
use foundry_common::ContractsByArtifact;
use foundry_config::{Chain, Config};
use semver::Version;

/// Data struct to help `ScriptSequence` verify contracts on `etherscan`.
pub struct VerifyBundle {
    pub num_of_optimizations: Option<usize>,
    pub known_contracts: ContractsByArtifact,
    pub etherscan_key: Option<String>,
    pub chain: Chain,
    pub project_paths: ProjectPathsArgs,
    pub retry: RetryArgs,
    pub verifier: VerifierArgs,
}

impl VerifyBundle {
    pub fn new(
        project: &Project,
        config: &Config,
        known_contracts: ContractsByArtifact,
        retry: RetryArgs,
        verifier: VerifierArgs,
    ) -> Self {
        let num_of_optimizations =
            if config.optimizer { Some(config.optimizer_runs) } else { None };

        let config_path = config.get_config_path();

        let project_paths = ProjectPathsArgs {
            root: Some(project.paths.root.clone()),
            contracts: Some(project.paths.sources.clone()),
            remappings: project.paths.remappings.clone(),
            remappings_env: None,
            cache_path: Some(project.paths.cache.clone()),
            lib_paths: project.paths.libraries.clone(),
            hardhat: config.profile == Config::HARDHAT_PROFILE,
            config_path: if config_path.exists() { Some(config_path) } else { None },
        };

        VerifyBundle {
            num_of_optimizations,
            known_contracts,
            etherscan_key: None,
            chain: Default::default(),
            project_paths,
            retry,
            verifier,
        }
    }

    /// Configures the chain and sets the etherscan key, if available
    pub fn set_chain(&mut self, config: &Config, chain: Chain) {
        self.etherscan_key = config.get_etherscan_api_key(Some(chain));
        self.chain = chain;
    }

    /// Given a `VerifyBundle` and contract details, it tries to generate a valid `VerifyArgs` to
    /// use against the `contract_address`.
    pub fn get_verify_args(
        &self,
        contract_address: Address,
        create2_offset: usize,
        data: &[u8],
        libraries: &[String],
    ) -> Option<VerifyArgs> {
        for (artifact, (_contract, bytecode)) in self.known_contracts.iter() {
            // If it's a CREATE2, the tx.data comes with a 32-byte salt in the beginning
            // of the transaction
            if data.split_at(create2_offset).1.starts_with(bytecode) {
                let constructor_args = data.split_at(create2_offset + bytecode.len()).1.to_vec();

                let contract = ContractInfo {
                    path: Some(
                        artifact.source.to_str().expect("There should be an artifact.").to_string(),
                    ),
                    name: artifact.name.clone(),
                };

                // We strip the build metadadata information, since it can lead to
                // etherscan not identifying it correctly. eg:
                // `v0.8.10+commit.fc410830.Linux.gcc` != `v0.8.10+commit.fc410830`
                let version = Version::new(
                    artifact.version.major,
                    artifact.version.minor,
                    artifact.version.patch,
                );

                let verify = VerifyArgs {
                    address: contract_address,
                    contract,
                    compiler_version: Some(version.to_string()),
                    constructor_args: Some(hex::encode(constructor_args)),
                    constructor_args_path: None,
                    num_of_optimizations: self.num_of_optimizations,
                    chain: self.chain,
                    etherscan_key: self.etherscan_key.clone(),
                    flatten: false,
                    force: false,
                    watch: true,
                    retry: self.retry.clone(),
                    libraries: libraries.to_vec(),
                    root: None,
                    verifier: self.verifier.clone(),
                };

                return Some(verify)
            }
        }
        None
    }
}
