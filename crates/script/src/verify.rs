use crate::{
    ScriptArgs, ScriptConfig,
    build::LinkedBuildData,
    sequence::{ScriptSequenceKind, get_commit_hash},
};
use alloy_network::{Network, ReceiptResponse};
use alloy_primitives::{Address, TxHash, hex};
use eyre::{Result, eyre};
use forge_script_sequence::{AdditionalContract, ScriptSequence};
use forge_verify::{
    RetryArgs, VerifierArgs, VerifyArgs,
    provider::{ExternalVerificationContext, VerificationProviderType},
};
use foundry_cli::opts::{EtherscanOpts, ProjectPathOpts};
use foundry_common::{ContractsByArtifact, FoundryReceiptResponse};
use foundry_compilers::{Project, artifacts::EvmVersion, info::ContractInfo};
use foundry_config::{Chain, Config};
use foundry_evm::core::evm::FoundryEvmNetwork;
use semver::Version;

mod external;

use external::{ExternalResolver, MAX_PROVENANCE_ADDRESSES, MatchResult, match_candidates};

const MAX_EXTERNAL_JOBS: usize = 32;

/// State after we have broadcasted the script.
/// It is assumed that at this point [BroadcastedState::sequence] contains receipts for all
/// broadcasted transactions.
pub struct BroadcastedState<FEN: FoundryEvmNetwork> {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig<FEN>,
    pub build_data: LinkedBuildData,
    pub sequence: ScriptSequenceKind<FEN::Network>,
}

impl<FEN: FoundryEvmNetwork> BroadcastedState<FEN> {
    pub async fn verify(self) -> Result<()> {
        let Self { args, script_config, build_data, mut sequence, .. } = self;

        let verify = VerifyBundle::new(
            &script_config.config.project()?,
            &script_config.config,
            build_data.known_contracts,
            args.retry,
            args.verifier,
            args.verify_external,
        );

        for sequence in sequence.sequences_mut() {
            verify_contracts::<FEN>(sequence, &script_config.config, verify.clone()).await?;
        }

        Ok(())
    }
}

/// Data struct to help `ScriptSequence` verify contracts on `etherscan`.
#[derive(Clone)]
pub struct VerifyBundle {
    pub num_of_optimizations: Option<usize>,
    pub known_contracts: ContractsByArtifact,
    pub project_paths: ProjectPathOpts,
    pub etherscan: EtherscanOpts,
    pub retry: RetryArgs,
    pub verifier: VerifierArgs,
    pub via_ir: bool,
    pub verify_external: bool,
    source_api_key: Option<String>,
}

impl VerifyBundle {
    pub fn new(
        project: &Project,
        config: &Config,
        known_contracts: ContractsByArtifact,
        retry: RetryArgs,
        verifier: VerifierArgs,
        verify_external: bool,
    ) -> Self {
        let num_of_optimizations =
            if config.optimizer == Some(true) { config.optimizer_runs } else { None };

        let config_path = config.get_config_path();

        let project_paths = ProjectPathOpts {
            root: Some(project.paths.root.clone()),
            contracts: Some(project.paths.sources.clone()),
            remappings: project.paths.remappings.clone(),
            remappings_env: None,
            cache_path: Some(project.paths.cache.clone()),
            lib_paths: project.paths.libraries.clone(),
            hardhat: config.profile == Config::HARDHAT_PROFILE,
            config_path: config_path.exists().then_some(config_path),
        };

        let via_ir = config.via_ir;

        Self {
            num_of_optimizations,
            known_contracts,
            etherscan: Default::default(),
            project_paths,
            retry,
            verifier,
            via_ir,
            verify_external,
            source_api_key: None,
        }
    }

    /// Configures the chain and sets the etherscan key, if available
    pub fn set_chain(&mut self, config: &Config, chain: Chain) {
        // If dealing with multiple chains, we need to be able to change in between the config
        // chain_id.
        let config_key = source_api_key(config, chain);
        self.source_api_key = config_key.clone();
        self.etherscan.key =
            self.verifier.resolve_api_key(config_key.as_deref()).map(str::to_owned);
        self.etherscan.chain = Some(chain);
    }

    /// Given a `VerifyBundle` and contract details, it tries to generate a valid `VerifyArgs` to
    /// use against the `contract_address`.
    pub fn get_verify_args(
        &self,
        contract_address: Address,
        create2_offset: usize,
        data: &[u8],
        libraries: &[String],
        evm_version: EvmVersion,
    ) -> Option<VerifyArgs> {
        for (artifact, contract) in self.known_contracts.iter() {
            let Some(bytecode) = contract.bytecode() else { continue };
            // If it's a CREATE2, the tx.data comes with a 32-byte salt in the beginning
            // of the transaction
            if data.split_at(create2_offset).1.starts_with(bytecode) {
                let constructor_args = data.split_at(create2_offset + bytecode.len()).1.to_vec();

                if artifact.source.extension().is_some_and(|e| e.to_str() == Some("vy")) {
                    warn!("Skipping verification of Vyper contract: {}", artifact.name);
                    return None;
                }

                // Strip artifact profile from contract name when creating contract info.
                let contract = ContractInfo {
                    path: Some(artifact.source.to_string_lossy().to_string()),
                    name: artifact
                        .name
                        .strip_suffix(&format!(".{}", artifact.profile))
                        .unwrap_or_else(|| &artifact.name)
                        .to_string(),
                };

                // We strip the build metadata information, since it can lead to
                // etherscan not identifying it correctly. eg:
                // `v0.8.10+commit.fc410830.Linux.gcc` != `v0.8.10+commit.fc410830`
                let version = Version::new(
                    artifact.version.major,
                    artifact.version.minor,
                    artifact.version.patch,
                );

                let verify = VerifyArgs {
                    address: contract_address,
                    contract: Some(contract),
                    compiler_version: Some(version.to_string()),
                    constructor_args: Some(hex::encode(constructor_args)),
                    constructor_args_path: None,
                    no_auto_detect: false,
                    use_solc: None,
                    num_of_optimizations: self.num_of_optimizations,
                    etherscan: self.etherscan.clone(),
                    rpc: Default::default(),
                    flatten: false,
                    force: false,
                    skip_is_verified_check: true,
                    watch: true,
                    print_submission_result_to_stdout: false,
                    retry: self.retry,
                    libraries: libraries.to_vec(),
                    root: None,
                    verifier: self.verifier.clone(),
                    via_ir: self.via_ir,
                    license_type: None,
                    evm_version: Some(evm_version),
                    show_standard_json_input: false,
                    guess_constructor_args: false,
                    compilation_profile: Some(artifact.profile.clone()),
                    language: None,
                    creation_transaction_hash: None,
                };

                return Some(verify);
            }
        }
        None
    }
}

fn source_api_key(config: &Config, chain: Chain) -> Option<String> {
    config.get_etherscan_api_key(Some(chain)).or_else(|| config.etherscan_api_key.clone())
}

enum VerificationJob {
    Local(VerifyArgs),
    External(VerifyArgs, Box<ExternalVerificationContext>),
}

impl VerificationJob {
    async fn run(self) -> Result<()> {
        match self {
            Self::Local(args) => args.run().await,
            Self::External(args, context) => args.run_with_external_context(*context).await,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn external_job(
    resolver: &mut Option<ExternalResolver>,
    config: &Config,
    chain: Chain,
    verify: &VerifyBundle,
    address: Address,
    init_code: &[u8],
    creators: &[Address],
    creation_transaction_hash: TxHash,
) -> Result<VerificationJob, String> {
    if resolver.is_none() {
        *resolver = Some(ExternalResolver::new().map_err(|err| concise(&err.to_string()))?);
    }
    let resolver = resolver.as_mut().unwrap();
    let mut candidate_sets = Vec::new();
    let mut reasons = Vec::new();

    for &creator in creators.iter().take(MAX_PROVENANCE_ADDRESSES) {
        let sources = [
            ("Sourcify", resolver.resolve_sourcify(chain, creator).await),
            (
                "Etherscan",
                resolver
                    .resolve_etherscan(config, chain, creator, verify.source_api_key.clone())
                    .await,
            ),
        ];
        for (provider, source) in sources {
            match source {
                Ok(Some(source)) => match resolver.compile(&source).await {
                    Ok(compiled) => candidate_sets.push(compiled),
                    Err(err) => reasons.push(format!(
                        "{} {creator}: compile failed ({})",
                        source.provider,
                        concise(&err)
                    )),
                },
                Ok(None) => {}
                Err(err) => reasons.push(format!("{provider} {creator}: {}", concise(&err))),
            }
        }
    }

    let matched = match match_candidates(
        init_code,
        candidate_sets.iter().flat_map(|candidates| candidates.iter()),
    ) {
        MatchResult::Unique(matched) => matched,
        MatchResult::None => {
            let context = if reasons.is_empty() {
                "no matching candidates were found".to_string()
            } else {
                format!("no matching candidates were found; {}", reasons.join("; "))
            };
            return Err(context);
        }
        MatchResult::Ambiguous(matches) => {
            let fqns = matches
                .into_iter()
                .map(|matched| format!("{}@{}", matched.fqn, matched.version))
                .collect::<Vec<_>>();
            return Err(format!("ambiguous external candidates: {}", fqns.join(", ")));
        }
    };

    let mut pinned_config = config.clone();
    pinned_config.chain = Some(chain);
    let context = ExternalVerificationContext {
        config: pinned_config,
        compiler_version: matched.version.clone(),
        standard_json_input: matched.input,
        target: matched.fqn,
    };
    let args = VerifyArgs {
        address,
        contract: None,
        compiler_version: Some(matched.version.to_string()),
        constructor_args: Some(hex::encode(matched.constructor_args)),
        constructor_args_path: None,
        no_auto_detect: false,
        use_solc: None,
        num_of_optimizations: None,
        etherscan: verify.etherscan.clone(),
        rpc: Default::default(),
        flatten: false,
        force: false,
        skip_is_verified_check: true,
        watch: true,
        print_submission_result_to_stdout: false,
        retry: verify.retry,
        libraries: Vec::new(),
        root: None,
        verifier: verify.verifier.clone(),
        via_ir: false,
        license_type: None,
        evm_version: None,
        show_standard_json_input: false,
        guess_constructor_args: false,
        compilation_profile: None,
        language: None,
        creation_transaction_hash: Some(creation_transaction_hash),
    };
    Ok(VerificationJob::External(args, Box::new(context)))
}

fn concise(reason: &str) -> String {
    const LIMIT: usize = 160;
    let mut chars = reason.chars().map(|ch| if ch.is_control() { ' ' } else { ch });
    let reason = chars.by_ref().take(LIMIT).collect::<String>();
    if chars.next().is_some() { format!("{reason}…") } else { reason }
}

fn take_matching_index<T>(
    values: &[T],
    consumed: &mut [bool],
    predicate: impl Fn(&T) -> bool,
) -> Option<usize> {
    let index = values
        .iter()
        .enumerate()
        .position(|(index, value)| !consumed[index] && predicate(value))?;
    consumed[index] = true;
    Some(index)
}

/// Given the broadcast log, it matches transactions with receipts, and tries to verify any
/// created contract on etherscan.
async fn verify_contracts<FEN: FoundryEvmNetwork>(
    sequence: &mut ScriptSequence<FEN::Network>,
    config: &Config,
    mut verify: VerifyBundle,
) -> Result<()> {
    trace!(target: "script", "verifying {} contracts [{}]", verify.known_contracts.len(), sequence.chain);

    verify.set_chain(config, sequence.chain.into());

    if verify.etherscan.has_key()
        || verify.verifier.effective_type() != VerificationProviderType::Etherscan
    {
        trace!(target: "script", "prepare future verifications");

        let mut verification_jobs = Vec::with_capacity(sequence.receipts.len());
        let mut unverifiable_contracts = vec![];
        let mut resolver = None;
        let mut external_jobs = 0;
        let mut warned_offline = false;
        let mut consumed_receipts = vec![false; sequence.receipts.len()];

        for tx in &sequence.transactions {
            let Some(tx_hash) = tx.hash else {
                let _ = sh_warn!("Skipping verification for transaction without a hash.");
                continue;
            };
            let Some(receipt_index) =
                take_matching_index(&sequence.receipts, &mut consumed_receipts, |receipt| {
                    receipt.transaction_hash() == tx_hash
                })
            else {
                let _ = sh_warn!(
                    "Skipping verification for transaction {tx_hash}: receipt unavailable."
                );
                continue;
            };
            let receipt = &mut sequence.receipts[receipt_index];
            // create2 hash offset
            let offset = if tx.is_create2()
                && let Some(contract_address) = tx.contract_address
            {
                receipt.set_contract_address(contract_address);
                32
            } else {
                0
            };

            // Verify contract created directly from the transaction
            if let (Some(address), Some(data)) = (receipt.contract_address(), tx.tx().input()) {
                match verify.get_verify_args(
                    address,
                    offset,
                    data,
                    &sequence.libraries,
                    config.evm_version,
                ) {
                    Some(verify) => verification_jobs.push(VerificationJob::Local(verify)),
                    None => unverifiable_contracts.push(address),
                };
            }

            // Verify potential contracts created during the transaction execution
            for AdditionalContract { address, init_code, creator_code_addresses, .. } in
                &tx.additional_contracts
            {
                match verify.get_verify_args(
                    *address,
                    0,
                    init_code.as_ref(),
                    &sequence.libraries,
                    config.evm_version,
                ) {
                    Some(args) => verification_jobs.push(VerificationJob::Local(args)),
                    None if !verify.verify_external => unverifiable_contracts.push(*address),
                    None if config.offline => {
                        if !warned_offline {
                            let _ = sh_warn!(
                                "Skipping external contract verification because offline mode is enabled."
                            );
                            warned_offline = true;
                        }
                    }
                    None if creator_code_addresses.is_empty() => {
                        let _ = sh_warn!(
                            "Skipping external verification for {address}: creator provenance is unavailable (old broadcast logs or skipped simulation)."
                        );
                    }
                    None if external_jobs >= MAX_EXTERNAL_JOBS => {
                        let _ = sh_warn!(
                            "Skipping external verification for {address}: external job limit exceeded."
                        );
                    }
                    None => {
                        external_jobs += 1;
                        match external_job(
                            &mut resolver,
                            config,
                            sequence.chain.into(),
                            &verify,
                            *address,
                            init_code,
                            creator_code_addresses,
                            receipt.transaction_hash(),
                        )
                        .await
                        {
                            Ok(job) => verification_jobs.push(job),
                            Err(reason) => {
                                let _ = sh_warn!(
                                    "Skipping external verification for {address}: {reason}"
                                );
                            }
                        }
                    }
                };
            }
        }

        trace!(target: "script", "collected {} verification jobs and {} unverifiable contracts", verification_jobs.len(), unverifiable_contracts.len());

        check_unverified(sequence, unverifiable_contracts, verify);

        let num_verifications = verification_jobs.len();
        let mut num_of_successful_verifications = 0;
        sh_status!("##\nStart verification for ({num_verifications}) contracts")?;
        for verification in verification_jobs {
            match verification.run().await {
                Ok(_) => {
                    num_of_successful_verifications += 1;
                }
                Err(err) => {
                    sh_err!("Failed to verify contract: {err:#}")?;
                }
            }
        }

        if num_of_successful_verifications < num_verifications {
            return Err(eyre!(
                "Not all ({num_of_successful_verifications} / {num_verifications}) contracts were verified!"
            ));
        }

        sh_status!("All ({num_verifications}) contracts were verified!")?;
    }

    Ok(())
}

fn check_unverified<N: Network>(
    sequence: &ScriptSequence<N>,
    unverifiable_contracts: Vec<Address>,
    verify: VerifyBundle,
) {
    if !unverifiable_contracts.is_empty() {
        let _ = sh_warn!(
            "We haven't found any matching bytecode for the following contracts: {:?}.\n\n\
            This may occur when resuming a verification, but the underlying source code or compiler version has changed.\n\
            Run `forge clean` to make sure builds are in sync with project files, then try again. Alternatively, use `forge verify-contract` to verify contracts that are already deployed.",
            unverifiable_contracts
        );

        if let Some(commit) = &sequence.commit {
            let current_commit = verify
                .project_paths
                .root
                .map(|root| get_commit_hash(&root).unwrap_or_default())
                .unwrap_or_default();

            if &current_commit != commit {
                let _ = sh_warn!(
                    "Script was broadcasted on commit `{commit}`, but we are at `{current_commit}`."
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{concise, source_api_key, take_matching_index};
    use alloy_chains::Chain;
    use foundry_config::Config;

    #[test]
    fn receipt_matching_is_hash_based_and_consumes_duplicate_hashes_in_order() {
        let reversed = [(2, "second"), (1, "first")];
        let mut consumed = [false; 2];
        assert_eq!(take_matching_index(&reversed, &mut consumed, |(hash, _)| *hash == 1), Some(1));
        assert_eq!(take_matching_index(&reversed, &mut consumed, |(hash, _)| *hash == 2), Some(0));
        assert_eq!(consumed, [true, true]);

        let batch = [(7, "first"), (7, "second")];
        let mut consumed = [false; 2];
        let first = take_matching_index(&batch, &mut consumed, |(hash, _)| *hash == 7).unwrap();
        let second = take_matching_index(&batch, &mut consumed, |(hash, _)| *hash == 7).unwrap();
        assert_eq!((batch[first].1, batch[second].1), ("first", "second"));
        assert!(take_matching_index(&batch, &mut consumed, |(hash, _)| *hash == 7).is_none());
    }

    #[test]
    fn source_key_is_only_read_from_etherscan_config() {
        let mut config = Config { etherscan_api_key: Some("source".into()), ..Default::default() };
        let verifier_key = "submission-only";
        assert_eq!(source_api_key(&config, Chain::mainnet()).as_deref(), Some("source"));
        config.etherscan_api_key = None;
        assert_ne!(source_api_key(&config, Chain::mainnet()).as_deref(), Some(verifier_key));
    }

    #[test]
    fn concise_sanitizes_and_bounds_remote_errors() {
        let message = format!("remote\n\u{1b}[31m{}", "x".repeat(200));
        let concise = concise(&message);
        assert!(!concise.chars().any(char::is_control));
        assert!(concise.chars().count() <= 161);
    }
}
