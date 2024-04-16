use crate::{
    broadcast::BundledState,
    execute::LinkedState,
    multi_sequence::MultiChainSequence,
    sequence::{ScriptSequence, ScriptSequenceKind},
    ScriptArgs, ScriptConfig,
};

use alloy_primitives::{Address, Bytes};
use alloy_provider::Provider;
use eyre::{Context, OptionExt, Result};
use foundry_cheatcodes::ScriptWallets;
use foundry_cli::utils::get_cached_entry_by_name;
use foundry_common::{
    compile::{self, ContractSources, ProjectCompiler},
    provider::alloy::try_get_http_provider,
    ContractData, ContractsByArtifact,
};
use foundry_compilers::{
    artifacts::{BytecodeObject, Libraries},
    cache::SolFilesCache,
    info::ContractInfo,
    ArtifactId, ProjectCompileOutput,
};
use foundry_linking::{LinkOutput, Linker};
use std::{path::PathBuf, str::FromStr, sync::Arc};

/// Container for the compiled contracts.
pub struct BuildData {
    /// Root of the project
    pub project_root: PathBuf,
    /// Linker which can be used to link contracts, owns [ArtifactContracts] map.
    pub output: ProjectCompileOutput,
    /// Id of target contract artifact.
    pub target: ArtifactId,
}

impl BuildData {
    pub fn get_linker(&self) -> Linker {
        Linker::new(self.project_root.clone(), self.output.artifact_ids().collect())
    }

    /// Links the build data with given libraries, using sender and nonce to compute addresses of
    /// missing libraries.
    pub fn link(
        self,
        known_libraries: Libraries,
        sender: Address,
        nonce: u64,
    ) -> Result<LinkedBuildData> {
        let link_output = self.get_linker().link_with_nonce_or_address(
            known_libraries,
            sender,
            nonce,
            &self.target,
        )?;

        LinkedBuildData::new(link_output, self)
    }

    /// Links the build data with the given libraries. Expects supplied libraries set being enough
    /// to fully link target contract.
    pub fn link_with_libraries(self, libraries: Libraries) -> Result<LinkedBuildData> {
        let link_output = self.get_linker().link_with_nonce_or_address(
            libraries,
            Address::ZERO,
            0,
            &self.target,
        )?;

        if !link_output.libs_to_deploy.is_empty() {
            eyre::bail!("incomplete libraries set");
        }

        LinkedBuildData::new(link_output, self)
    }
}

/// Container for the linked contracts and their dependencies
pub struct LinkedBuildData {
    /// Original build data, might be used to relink this object with different libraries.
    pub build_data: BuildData,
    /// Known fully linked contracts.
    pub known_contracts: ContractsByArtifact,
    /// Libraries used to link the contracts.
    pub libraries: Libraries,
    /// Libraries that need to be deployed by sender before script execution.
    pub predeploy_libraries: Vec<Bytes>,
    /// Source files of the contracts. Used by debugger.
    pub sources: ContractSources,
}

impl LinkedBuildData {
    pub fn new(link_output: LinkOutput, build_data: BuildData) -> Result<Self> {
        let sources = ContractSources::from_project_output(
            &build_data.output,
            &build_data.project_root,
            &link_output.libraries,
        )?;

        let known_contracts = ContractsByArtifact(
            build_data
                .get_linker()
                .get_linked_artifacts(&link_output.libraries)?
                .into_iter()
                .filter_map(|(id, contract)| {
                    let name = id.name.clone();
                    let bytecode = contract.bytecode.and_then(|b| b.into_bytes())?;
                    let deployed_bytecode =
                        contract.deployed_bytecode.and_then(|b| b.into_bytes())?;
                    let abi = contract.abi?;

                    Some((id, ContractData { name, abi, bytecode, deployed_bytecode }))
                })
                .collect(),
        );

        Ok(Self {
            build_data,
            known_contracts,
            libraries: link_output.libraries,
            predeploy_libraries: link_output.libs_to_deploy,
            sources,
        })
    }

    /// Fetches target bytecode from linked contracts.
    pub fn get_target_contract(&self) -> Result<ContractData> {
        self.known_contracts
            .get(&self.build_data.target)
            .cloned()
            .ok_or_eyre("target not found in linked artifacts")
    }
}

/// First state basically containing only inputs of the user.
pub struct PreprocessedState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
}

impl PreprocessedState {
    /// Parses user input and compiles the contracts depending on script target.
    /// After compilation, finds exact [ArtifactId] of the target contract.
    pub fn compile(self) -> Result<CompiledState> {
        let Self { args, script_config, script_wallets } = self;
        let project = script_config.config.project()?;
        let filters = args.skip.clone().unwrap_or_default();

        let mut target_name = args.target_contract.clone();

        // If we've received correct path, use it as target_path
        // Otherwise, parse input as <path>:<name> and use the path from the contract info, if
        // present.
        let target_path = if let Ok(path) = dunce::canonicalize(&args.path) {
            Some(path)
        } else {
            let contract = ContractInfo::from_str(&args.path)?;
            target_name = Some(contract.name.clone());
            if let Some(path) = contract.path {
                Some(dunce::canonicalize(path)?)
            } else {
                None
            }
        };

        // If we've found target path above, only compile it.
        // Otherwise, compile everything to match contract by name later.
        let output = if let Some(target_path) = target_path.clone() {
            compile::compile_target_with_filter(
                &target_path,
                &project,
                args.opts.silent,
                args.verify,
                filters,
            )
        } else if !project.paths.has_input_files() {
            Err(eyre::eyre!("The project doesn't have any input files. Make sure the `script` directory is configured properly in foundry.toml. Otherwise, provide the path to the file."))
        } else {
            ProjectCompiler::new().compile(&project)
        }?;

        // If we still don't have target path, find it by name in the compilation cache.
        let target_path = if let Some(target_path) = target_path {
            target_path
        } else {
            let target_name = target_name.clone().expect("was set above");
            let cache = SolFilesCache::read_joined(&project.paths)
                .wrap_err("Could not open compiler cache")?;
            let (path, _) = get_cached_entry_by_name(&cache, &target_name)
                .wrap_err("Could not find target contract in cache")?;
            path
        };

        let target_path = project.root().join(target_path);

        let mut target_id: Option<ArtifactId> = None;

        // Find target artfifact id by name and path in compilation artifacts.
        for (id, contract) in output.artifact_ids().filter(|(id, _)| id.source == target_path) {
            if let Some(name) = &target_name {
                if id.name != *name {
                    continue;
                }
            } else if contract.abi.as_ref().map_or(true, |abi| abi.is_empty()) ||
                contract.bytecode.as_ref().map_or(true, |b| match &b.object {
                    BytecodeObject::Bytecode(b) => b.is_empty(),
                    BytecodeObject::Unlinked(_) => false,
                })
            {
                // Ignore contracts with empty abi or linked bytecode of length 0 which are
                // interfaces/abstract contracts/libraries.
                continue;
            }

            if let Some(target) = target_id {
                // We might have multiple artifacts for the same contract but with different
                // solc versions. Their names will have form of {name}.0.X.Y, so we are
                // stripping versions off before comparing them.
                let target_name = target.name.split('.').next().unwrap();
                let id_name = id.name.split('.').next().unwrap();
                if target_name != id_name {
                    eyre::bail!("Multiple contracts in the target path. Please specify the contract name with `--tc ContractName`")
                }
            }
            target_id = Some(id);
        }

        let target = target_id.ok_or_eyre("Could not find target contract")?;

        Ok(CompiledState {
            args,
            script_config,
            script_wallets,
            build_data: BuildData { output, target, project_root: project.root().clone() },
        })
    }
}

/// State after we have determined and compiled target contract to be executed.
pub struct CompiledState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: BuildData,
}

impl CompiledState {
    /// Uses provided sender address to compute library addresses and link contracts with them.
    pub fn link(self) -> Result<LinkedState> {
        let Self { args, script_config, script_wallets, build_data } = self;

        let sender = script_config.evm_opts.sender;
        let nonce = script_config.sender_nonce;
        let known_libraries = script_config.config.libraries_with_remappings()?;
        let build_data = build_data.link(known_libraries, sender, nonce)?;

        Ok(LinkedState { args, script_config, script_wallets, build_data })
    }

    /// Tries loading the resumed state from the cache files, skipping simulation stage.
    pub async fn resume(self) -> Result<BundledState> {
        let chain = if self.args.multi {
            None
        } else {
            let fork_url = self.script_config.evm_opts.fork_url.clone().ok_or_eyre("Missing --fork-url field, if you were trying to broadcast a multi-chain sequence, please use --multi flag")?;
            let provider = Arc::new(try_get_http_provider(fork_url)?);
            Some(provider.get_chain_id().await?)
        };

        let sequence = match self.try_load_sequence(chain, false) {
            Ok(sequence) => sequence,
            Err(_) => {
                // If the script was simulated, but there was no attempt to broadcast yet,
                // try to read the script sequence from the `dry-run/` folder
                let mut sequence = self.try_load_sequence(chain, true)?;

                // If sequence was in /dry-run, Update its paths so it is not saved into /dry-run
                // this time as we are about to broadcast it.
                sequence.update_paths_to_broadcasted(
                    &self.script_config.config,
                    &self.args.sig,
                    &self.build_data.target,
                )?;

                sequence.save(true, true)?;
                sequence
            }
        };

        let (args, build_data, script_wallets, script_config) = if !self.args.unlocked {
            let mut froms = sequence.sequences().iter().flat_map(|s| {
                s.transactions
                    .iter()
                    .skip(s.receipts.len())
                    .map(|t| t.transaction.from.expect("from is missing in script artifact"))
            });

            let available_signers = self
                .script_wallets
                .signers()
                .map_err(|e| eyre::eyre!("Failed to get available signers: {}", e))?;

            if !froms.all(|from| available_signers.contains(&from)) {
                // IF we are missing required signers, execute script as we might need to collect
                // private keys from the execution.
                let executed = self.link()?.prepare_execution().await?.execute().await?;
                (
                    executed.args,
                    executed.build_data.build_data,
                    executed.script_wallets,
                    executed.script_config,
                )
            } else {
                (self.args, self.build_data, self.script_wallets, self.script_config)
            }
        } else {
            (self.args, self.build_data, self.script_wallets, self.script_config)
        };

        // Collect libraries from sequence and link contracts with them.
        let libraries = match sequence {
            ScriptSequenceKind::Single(ref seq) => Libraries::parse(&seq.libraries)?,
            // Library linking is not supported for multi-chain sequences
            ScriptSequenceKind::Multi(_) => Libraries::default(),
        };

        let linked_build_data = build_data.link_with_libraries(libraries)?;

        Ok(BundledState {
            args,
            script_config,
            script_wallets,
            build_data: linked_build_data,
            sequence,
        })
    }

    fn try_load_sequence(&self, chain: Option<u64>, dry_run: bool) -> Result<ScriptSequenceKind> {
        if let Some(chain) = chain {
            let sequence = ScriptSequence::load(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.target,
                chain,
                dry_run,
            )?;
            Ok(ScriptSequenceKind::Single(sequence))
        } else {
            let sequence = MultiChainSequence::load(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.target,
                dry_run,
            )?;
            Ok(ScriptSequenceKind::Multi(sequence))
        }
    }
}
