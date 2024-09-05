use crate::{
    broadcast::BundledState,
    execute::LinkedState,
    multi_sequence::MultiChainSequence,
    sequence::{ScriptSequence, ScriptSequenceKind},
    ScriptArgs, ScriptConfig,
};
use alloy_primitives::{Bytes, B256};
use alloy_provider::Provider;
use eyre::{OptionExt, Result};
use foundry_cheatcodes::ScriptWallets;
use foundry_common::{
    compile::ProjectCompiler, provider::try_get_http_provider, ContractData, ContractsByArtifact,
};
use foundry_compilers::{
    artifacts::{BytecodeObject, Libraries},
    compilers::{multi::MultiCompilerLanguage, Language},
    info::ContractInfo,
    utils::source_files_iter,
    ArtifactId, ProjectCompileOutput,
};
use foundry_evm::{constants::DEFAULT_CREATE2_DEPLOYER, traces::debug::ContractSources};
use foundry_linking::Linker;
use std::{path::PathBuf, str::FromStr, sync::Arc};

/// Container for the compiled contracts.
#[derive(Debug)]
pub struct BuildData {
    /// Root of the project.
    pub project_root: PathBuf,
    /// The compiler output.
    pub output: ProjectCompileOutput,
    /// ID of target contract artifact.
    pub target: ArtifactId,
}

impl BuildData {
    pub fn get_linker(&self) -> Linker<'_> {
        Linker::new(self.project_root.clone(), self.output.artifact_ids().collect())
    }

    /// Links contracts. Uses CREATE2 linking when possible, otherwise falls back to
    /// default linking with sender nonce and address.
    pub async fn link(self, script_config: &ScriptConfig) -> Result<LinkedBuildData> {
        let can_use_create2 = if let Some(fork_url) = &script_config.evm_opts.fork_url {
            let provider = try_get_http_provider(fork_url)?;
            let deployer_code = provider.get_code_at(DEFAULT_CREATE2_DEPLOYER).await?;

            !deployer_code.is_empty()
        } else {
            // If --fork-url is not provided, we are just simulating the script.
            true
        };

        let known_libraries = script_config.config.libraries_with_remappings()?;

        let maybe_create2_link_output = can_use_create2
            .then(|| {
                self.get_linker()
                    .link_with_create2(
                        known_libraries.clone(),
                        DEFAULT_CREATE2_DEPLOYER,
                        script_config.config.create2_library_salt,
                        &self.target,
                    )
                    .ok()
            })
            .flatten();

        let (libraries, predeploy_libs) = if let Some(output) = maybe_create2_link_output {
            (
                output.libraries,
                ScriptPredeployLibraries::Create2(
                    output.libs_to_deploy,
                    script_config.config.create2_library_salt,
                ),
            )
        } else {
            let output = self.get_linker().link_with_nonce_or_address(
                known_libraries,
                script_config.evm_opts.sender,
                script_config.sender_nonce,
                [&self.target],
            )?;

            (output.libraries, ScriptPredeployLibraries::Default(output.libs_to_deploy))
        };

        LinkedBuildData::new(libraries, predeploy_libs, self)
    }

    /// Links the build data with the given libraries. Expects supplied libraries set being enough
    /// to fully link target contract.
    pub fn link_with_libraries(self, libraries: Libraries) -> Result<LinkedBuildData> {
        LinkedBuildData::new(libraries, ScriptPredeployLibraries::Default(Vec::new()), self)
    }
}

#[derive(Debug)]
pub enum ScriptPredeployLibraries {
    Default(Vec<Bytes>),
    Create2(Vec<Bytes>, B256),
}

impl ScriptPredeployLibraries {
    pub fn libraries_count(&self) -> usize {
        match self {
            Self::Default(libs) => libs.len(),
            Self::Create2(libs, _) => libs.len(),
        }
    }
}

/// Container for the linked contracts and their dependencies
#[derive(Debug)]
pub struct LinkedBuildData {
    /// Original build data, might be used to relink this object with different libraries.
    pub build_data: BuildData,
    /// Known fully linked contracts.
    pub known_contracts: ContractsByArtifact,
    /// Libraries used to link the contracts.
    pub libraries: Libraries,
    /// Libraries that need to be deployed by sender before script execution.
    pub predeploy_libraries: ScriptPredeployLibraries,
    /// Source files of the contracts. Used by debugger.
    pub sources: ContractSources,
}

impl LinkedBuildData {
    pub fn new(
        libraries: Libraries,
        predeploy_libraries: ScriptPredeployLibraries,
        build_data: BuildData,
    ) -> Result<Self> {
        let sources = ContractSources::from_project_output(
            &build_data.output,
            &build_data.project_root,
            Some(&libraries),
        )?;

        let known_contracts =
            ContractsByArtifact::new(build_data.get_linker().get_linked_artifacts(&libraries)?);

        Ok(Self { build_data, known_contracts, libraries, predeploy_libraries, sources })
    }

    /// Fetches target bytecode from linked contracts.
    pub fn get_target_contract(&self) -> Result<&ContractData> {
        self.known_contracts
            .get(&self.build_data.target)
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

        let mut target_name = args.target_contract.clone();

        // If we've received correct path, use it as target_path
        // Otherwise, parse input as <path>:<name> and use the path from the contract info, if
        // present.
        let target_path = if let Ok(path) = dunce::canonicalize(&args.path) {
            path
        } else {
            let contract = ContractInfo::from_str(&args.path)?;
            target_name = Some(contract.name.clone());
            if let Some(path) = contract.path {
                dunce::canonicalize(path)?
            } else {
                project.find_contract_path(contract.name.as_str())?
            }
        };

        #[allow(clippy::redundant_clone)]
        let sources_to_compile = source_files_iter(
            project.paths.sources.as_path(),
            MultiCompilerLanguage::FILE_EXTENSIONS,
        )
        .chain([target_path.to_path_buf()]);

        let output = ProjectCompiler::new()
            .quiet_if(args.opts.silent)
            .files(sources_to_compile)
            .compile(&project)?;

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
    pub async fn link(self) -> Result<LinkedState> {
        let Self { args, script_config, script_wallets, build_data } = self;

        let build_data = build_data.link(&script_config).await?;

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
                    .map(|t| t.transaction.from().expect("from is missing in script artifact"))
            });

            let available_signers = self
                .script_wallets
                .signers()
                .map_err(|e| eyre::eyre!("Failed to get available signers: {}", e))?;

            if !froms.all(|from| available_signers.contains(&from)) {
                // IF we are missing required signers, execute script as we might need to collect
                // private keys from the execution.
                let executed = self.link().await?.prepare_execution().await?.execute().await?;
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
