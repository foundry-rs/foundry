use crate::{
    ScriptArgs, ScriptConfig,
    broadcast::{BundledState, remaining_unsigned_transactions_for_recovery},
    execute::LinkedState,
    multi_sequence::MultiChainSequence,
    progress::ScriptProgress,
    recovery::recovery_exists,
    sequence::ScriptSequenceKind,
    session::{
        RemainingScriptTransaction, SignerScope, script_session_expected_sender_if_configured,
    },
};
use alloy_network::AnyNetwork;
use alloy_primitives::{Address, B256, map::AddressHashSet};
use alloy_provider::Provider;
use eyre::{ContextCompat, OptionExt, Result};
use forge_script_sequence::ScriptSequence;
use foundry_cheatcodes::Wallets;
use foundry_cli::opts::TempoOpts;
use foundry_common::{
    ContractData, ContractsByArtifact, ContractsByArtifactBuilder, compile::ProjectCompiler,
    provider::ProviderBuilder,
};
use foundry_compilers::{
    ArtifactId, ProjectCompileOutput,
    artifacts::{BytecodeObject, Libraries},
    compilers::{Language, multi::MultiCompilerLanguage},
    info::ContractInfo,
    utils::source_files_iter,
};
use foundry_evm::{core::evm::FoundryEvmNetwork, traces::debug::ContractSources};
use foundry_linking::Linker;
use foundry_wallets::{MultiWalletOpts, wallet_browser::signer::BrowserSigner};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

/// Container for the compiled contracts.
#[derive(Clone, Debug)]
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
    pub async fn link<FEN: FoundryEvmNetwork>(
        self,
        script_config: &ScriptConfig<FEN>,
    ) -> Result<LinkedBuildData> {
        let create2_deployer = script_config.evm_opts.create2_deployer;
        let can_use_create2 = script_config
            .evm_opts
            .can_use_create2_deployer_resolved(script_config.resolved_fork()?)
            .await?;

        let known_libraries = script_config.config.libraries_with_remappings()?;

        let maybe_create2_link_output = can_use_create2
            .then(|| {
                self.get_linker()
                    .link_with_create2_detailed(
                        known_libraries.clone(),
                        create2_deployer,
                        script_config.config.create2_library_salt,
                        [&self.target],
                    )
                    .ok()
            })
            .flatten();

        let (libraries, predeploy_libs) = if let Some(output) = maybe_create2_link_output {
            (
                output.output.libraries,
                ScriptPredeployLibraries::Create2 {
                    onchain: output.linked_libraries,
                    salt: script_config.config.create2_library_salt,
                    local: Vec::new(),
                },
            )
        } else {
            let output = self.get_linker().link_with_nonce_or_address_detailed(
                known_libraries,
                script_config.evm_opts.sender,
                script_config.sender_nonce,
                [&self.target],
            )?;

            (
                output.output.libraries,
                ScriptPredeployLibraries::Default {
                    onchain: output.linked_libraries,
                    local: Vec::new(),
                },
            )
        };

        LinkedBuildData::new(libraries, predeploy_libs, self)
    }

    /// Links the build data with the given libraries. Expects supplied libraries set being enough
    /// to fully link target contract.
    pub fn link_with_libraries(self, libraries: Libraries) -> Result<LinkedBuildData> {
        LinkedBuildData::new(
            libraries,
            ScriptPredeployLibraries::Default { onchain: Vec::new(), local: Vec::new() },
            self,
        )
    }
}

#[derive(Clone, Debug)]
pub enum ScriptPredeployLibraries {
    Default {
        onchain: Vec<foundry_linking::LinkedLibrary>,
        local: Vec<foundry_linking::LinkedLibrary>,
    },
    Create2 {
        onchain: Vec<foundry_linking::LinkedLibrary>,
        salt: B256,
        local: Vec<foundry_linking::LinkedLibrary>,
    },
}

impl ScriptPredeployLibraries {
    pub const fn libraries_count(&self) -> usize {
        match self {
            Self::Default { onchain, .. } => onchain.len(),
            Self::Create2 { onchain, .. } => onchain.len(),
        }
    }
}

/// Container for the linked contracts and their dependencies
#[derive(Clone, Debug)]
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

        let linked_contracts = build_data.get_linker().get_linked_artifacts(&libraries)?;
        let known_contracts = ContractsByArtifactBuilder::new(
            linked_contracts.iter().map(|(id, artifact)| (id.clone(), artifact.into())),
        )
        .with_storage_layouts(build_data.output.artifact_ids().filter_map(|(id, artifact)| {
            artifact.storage_layout.as_ref().map(|layout| (id, layout.clone()))
        }))
        .build();

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
pub struct PreprocessedState<FEN: FoundryEvmNetwork> {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig<FEN>,
    pub script_wallets: Wallets,
    pub browser_wallet: Option<BrowserSigner<FEN::Network>>,
}

impl<FEN: FoundryEvmNetwork> PreprocessedState<FEN> {
    /// Parses user input and compiles the contracts depending on script target.
    /// After compilation, finds exact [ArtifactId] of the target contract.
    pub fn compile(self) -> Result<CompiledState<FEN>> {
        let Self { args, script_config, script_wallets, browser_wallet } = self;
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

        let sources_to_compile = source_files_iter(
            project.paths.sources.as_path(),
            MultiCompilerLanguage::FILE_EXTENSIONS,
        )
        .chain([target_path.clone()]);

        let output = ProjectCompiler::new()
            .files(sources_to_compile)
            .dynamic_test_linking(script_config.config.dynamic_test_linking)
            .compile(&project)?;

        let mut target_id: Option<ArtifactId> = None;

        // Find target artifact id by name and path in compilation artifacts.
        for (id, contract) in output.artifact_ids().filter(|(id, _)| id.source == target_path) {
            if let Some(name) = &target_name {
                if id.name != *name {
                    continue;
                }
            } else if contract.abi.as_ref().is_none_or(|abi| abi.is_empty())
                || contract.bytecode.as_ref().is_none_or(|b| match &b.object {
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
                    eyre::bail!(
                        "Multiple contracts in the target path. Please specify the contract name with `--tc ContractName`"
                    );
                }
            }
            target_id = Some(id);
        }

        let target = target_id.ok_or_eyre("Could not find target contract")?;

        Ok(CompiledState {
            args,
            script_config,
            script_wallets,
            browser_wallet,
            build_data: BuildData { output, target, project_root: project.root().to_path_buf() },
        })
    }
}

/// State after we have determined and compiled target contract to be executed.
pub struct CompiledState<FEN: FoundryEvmNetwork> {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig<FEN>,
    pub script_wallets: Wallets,
    pub browser_wallet: Option<BrowserSigner<FEN::Network>>,
    pub build_data: BuildData,
}

impl<FEN: FoundryEvmNetwork> CompiledState<FEN> {
    /// Uses provided sender address to compute library addresses and link contracts with them.
    pub async fn link(self) -> Result<LinkedState<FEN>> {
        let Self { args, script_config, script_wallets, browser_wallet, build_data } = self;

        let build_data = build_data.link(&script_config).await?;

        Ok(LinkedState { args, script_config, script_wallets, browser_wallet, build_data })
    }

    /// Tries loading the resumed state from the cache files, skipping simulation stage.
    pub async fn resume(mut self) -> Result<BundledState<FEN>> {
        let chain = if self.args.multi {
            None
        } else {
            let fork_url = self.script_config.evm_opts.fork_url.clone().ok_or_eyre("Missing --fork-url field, if you were trying to broadcast a multi-chain sequence, please use --multi flag")?;
            let provider = Arc::new(ProviderBuilder::<AnyNetwork>::new(&fork_url).build()?);
            Some(provider.get_chain_id().await?)
        };

        let mut sequence = if self.sequence_exists(chain, false)? {
            self.try_load_sequence(chain, false)?
        } else {
            // If the script was simulated, but there was no attempt to broadcast yet,
            // read the script sequence from the `dry-run/` folder.
            let mut sequence = self.try_load_sequence(chain, true)?;

            // Promote the complete dry-run sequence before broadcasting it.
            sequence.promote_to_broadcasted(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.target,
            )?;
            sequence
        };

        if self.args.batch {
            let _ = sequence.restore_batch_delegated_pending(
                self.args.resume_attempt,
                self.args.resume_tx_hash,
                self.args.resume_retry,
            )?;
        } else {
            let resolution = sequence.restore_delegated_pending(
                self.args.resume_attempt,
                self.args.resume_tx_hash,
                self.args.resume_retry,
            )?;
            if let Some((sequence_index, _, attempt_id, hash)) = resolution {
                let provider = ProviderBuilder::<FEN::Network>::from_config_with_url(
                    &self.script_config.config,
                    sequence.sequences()[sequence_index].rpc_url(),
                )?
                .build()?;
                let transaction = provider
                    .get_transaction_by_hash(hash)
                    .await?
                    .context("resolved transaction is not available from the recovery endpoint")?;
                sequence.resolve_delegated_hash(attempt_id, hash, &transaction)?;
            }
            let progress = ScriptProgress::default();
            for index in 0..sequence.sequences().len() {
                if sequence.sequences()[index].pending.is_empty() {
                    continue;
                }
                let durable_hashes = sequence.submission_hashes(index);
                let replayable_hashes = sequence.signed_hashes(index);
                let provider = ProviderBuilder::from_config_with_url(
                    &self.script_config.config,
                    sequence.sequences()[index].rpc_url(),
                )?
                .build()?;
                let result = progress
                    .wait_for_pending(
                        index,
                        &mut sequence.sequences_mut()[index],
                        &provider,
                        self.script_config.config.transaction_timeout,
                        self.args.confirmations,
                        (&durable_hashes, &replayable_hashes),
                    )
                    .await;
                sequence.save(true, false)?;
                result?;
                sequence.ensure_delegated_outcomes_known(index)?;
            }
        }

        if !self.args.unlocked
            && !remaining_unsigned_transactions_for_recovery(&sequence).is_empty()
        {
            self.script_wallets =
                Wallets::new(self.args.wallets.get_multi_wallet().await?, self.args.evm.sender);
            self.browser_wallet = self.args.wallets.browser_signer::<FEN::Network>().await?;

            if self.args.evm.sender.is_none() {
                let addresses = self.script_wallets.addresses();
                let sender = self
                    .args
                    .maybe_load_private_key()?
                    .or_else(|| (addresses.len() == 1).then(|| addresses[0]))
                    .or_else(|| self.browser_wallet.as_ref().map(|wallet| wallet.address()));
                if let Some(sender) = sender {
                    self.script_config.update_sender(sender).await?;
                }
            }
        }

        let (args, build_data, script_wallets, browser_wallet, script_config) =
            if self.args.unlocked {
                (
                    self.args,
                    self.build_data,
                    self.script_wallets,
                    self.browser_wallet,
                    self.script_config,
                )
            } else {
                let remaining_transactions =
                    remaining_unsigned_transactions_for_recovery(&sequence);
                let remaining_froms =
                    remaining_transactions.iter().map(|tx| tx.from).collect::<AddressHashSet>();
                let expected_session_sender = script_session_expected_sender_if_configured(
                    &self.script_config.tempo,
                    &remaining_froms,
                )?;
                let has_available_signers = has_available_script_signers(
                    &self.script_config.tempo,
                    &self.args.wallets,
                    &self.script_wallets,
                    expected_session_sender,
                    &remaining_transactions,
                )?;

                if has_available_signers {
                    (
                        self.args,
                        self.build_data,
                        self.script_wallets,
                        self.browser_wallet,
                        self.script_config,
                    )
                } else {
                    // IF we are missing required signers, execute script as we might need to
                    // collect private keys from the execution.
                    let mut state = self;
                    state
                        .script_config
                        .update_tempo_session_sender(&state.args.wallets, state.args.evm.sender)
                        .await?;
                    let executed = state.link().await?.prepare_execution().await?.execute().await?;
                    (
                        executed.args,
                        executed.build_data.build_data,
                        executed.script_wallets,
                        executed.browser_wallet,
                        executed.script_config,
                    )
                }
            };

        // Collect libraries from sequence and link contracts with them.
        let libraries = if sequence.is_multi() {
            // Library linking is not supported for multi-chain sequences.
            Libraries::default()
        } else {
            Libraries::parse(&sequence.sequences()[0].libraries)?
        };

        let linked_build_data = build_data.link_with_libraries(libraries)?;

        Ok(BundledState {
            args,
            script_config,
            script_wallets,
            browser_wallet,
            build_data: linked_build_data,
            sequence,
        })
    }

    fn try_load_sequence(
        &self,
        chain: Option<u64>,
        dry_run: bool,
    ) -> Result<ScriptSequenceKind<FEN::Network>> {
        if let Some(chain) = chain {
            ScriptSequenceKind::load_single(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.target,
                chain,
                dry_run,
                self.args.batch,
            )
        } else {
            ScriptSequenceKind::load_multi(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.target,
                dry_run,
                self.args.batch,
            )
        }
    }

    fn sequence_exists(&self, chain: Option<u64>, dry_run: bool) -> Result<bool> {
        let paths = if let Some(chain) = chain {
            ScriptSequence::<FEN::Network>::get_paths(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.target,
                chain,
                dry_run,
            )?
        } else {
            MultiChainSequence::<FEN::Network>::get_paths(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.target,
                dry_run,
            )?
        };
        Ok(compatibility_progress_exists(&paths) || recovery_exists(&paths)?)
    }
}

fn compatibility_progress_exists(paths: &(PathBuf, PathBuf)) -> bool {
    [&paths.0, &paths.1].into_iter().any(|path| progress_exists(path))
}

fn progress_exists(path: &Path) -> bool {
    path.exists() || path.with_extension("previous").exists()
}

/// Returns whether every scoped signer needed for resume is already available.
///
/// `Wallets` only tracks signers collected from CLI options and script cheatcodes. A Tempo
/// session signer lives in the Accounts store instead, so resume needs to treat the session
/// root account as available only on the chain covered by the session.
fn has_available_script_signers(
    tempo: &TempoOpts,
    wallets: &MultiWalletOpts,
    script_wallets: &Wallets,
    expected_sender: Option<Address>,
    remaining: &[RemainingScriptTransaction],
) -> Result<bool> {
    let signers = script_wallets
        .signers()
        .map_err(|e| eyre::eyre!("Failed to get available signers: {}", e))?;
    if remaining.is_empty() {
        return Ok(true);
    }

    let session_scope = tempo
        .session_signer_for_multi_wallet_any_chain(wallets, expected_sender)?
        .map(|s| SignerScope::new(s.session.chain_id, s.access_key.account()));

    Ok(remaining.iter().all(|tx| signers.contains(&tx.from) || session_scope == Some(tx.scope())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_counts_as_recoverable_progress() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run-latest.json");
        std::fs::write(path.with_extension("previous"), b"{}").unwrap();

        assert!(progress_exists(&path));
    }

    #[test]
    fn sensitive_export_counts_as_recoverable_progress() {
        let dir = tempfile::tempdir().unwrap();
        let paths = (dir.path().join("run-latest.json"), dir.path().join("run-latest-cache.json"));
        std::fs::write(&paths.1, b"{}").unwrap();

        assert!(compatibility_progress_exists(&paths));
    }

    #[test]
    fn has_available_script_signers_skips_session_resolution_when_remaining_empty() {
        let has_available = has_available_script_signers(
            &TempoOpts { session: Some(B256::repeat_byte(0x99)), ..Default::default() },
            &MultiWalletOpts::default(),
            &Wallets::new(Default::default(), None),
            None,
            &[],
        )
        .unwrap();

        assert!(has_available);
    }
}
