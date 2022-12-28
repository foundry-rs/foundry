use super::{sequence::ScriptSequence, *};
use crate::cmd::{
    forge::script::{multi::MultiChainSequence, verify::VerifyBundle},
    LoadConfig,
};
use ethers::{
    prelude::{Middleware, Signer},
    types::{transaction::eip2718::TypedTransaction, U256},
};
use foundry_common::{contracts::flatten_contracts, diff_score, try_get_http_provider};
use itertools::Itertools;
use ordered_float::OrderedFloat;
use std::sync::Arc;
use tracing::trace;

/// Helper alias type for the collection of data changed due to the new sender.
type NewSenderChanges = (CallTraceDecoder, Libraries, ArtifactContracts<ContractBytecodeSome>);

impl ScriptArgs {
    /// Executes the script
    pub async fn run_script(mut self) -> eyre::Result<()> {
        trace!(target: "script", "executing script command");

        let (config, evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;
        let mut script_config = ScriptConfig {
            // dapptools compatibility
            sender_nonce: U256::one(),
            config,
            evm_opts,
            ..Default::default()
        };

        self.maybe_load_private_key(&mut script_config)?;

        if let Some(ref fork_url) = script_config.evm_opts.fork_url {
            // when forking, override the sender's nonce to the onchain value
            script_config.sender_nonce =
                foundry_utils::next_nonce(script_config.evm_opts.sender, fork_url, None).await?
        } else {
            // if not forking, then ignore any pre-deployed library addresses
            script_config.config.libraries = Default::default();
        }

        let build_output = self.compile(&mut script_config)?;

        let mut verify = VerifyBundle::new(
            &build_output.project,
            &script_config.config,
            flatten_contracts(&build_output.highlevel_known_contracts, false),
            self.retry,
            self.verifier.clone(),
        );

        let BuildOutput {
            project,
            contract,
            mut highlevel_known_contracts,
            predeploy_libraries,
            known_contracts: default_known_contracts,
            sources,
            mut libraries,
            ..
        } = build_output;

        // Execute once with default sender.
        let sender = script_config.evm_opts.sender;

        // We need to execute the script even if just resuming, in case we need to collect private
        // keys from the execution.
        let mut result =
            self.execute(&mut script_config, contract, sender, &predeploy_libraries).await?;

        // We only care about particular predeploy libraries that pertain to the actually deployed
        // contracts. That is to say: dont deploy libraries that are only used by the
        // script.
        let relevant_predeploys =
            get_relevant_predeploys(&result, &build_output.deploy_bytecode_to_dependencies)?;

        // we want to reset libraries to default is relevant predeploys are empty. Otherwise,
        // libraries are updated in the `maybe_prepare_libraries`
        if relevant_predeploys.is_empty() {
            libraries = Default::default();
        }

        // if there is a difference between predeploy libraries and relevant predeploy libraries,
        // we need to try to fix any nonces that got screwed up because of the difference
        if relevant_predeploys.len() < predeploy_libraries.len() {
            let diff = predeploy_libraries.len() - relevant_predeploys.len();
            decrement_nonces(&mut result, diff);
        }

        if self.resume || (self.verify && !self.broadcast) {
            return self
                .resume_deployment(
                    script_config,
                    project,
                    default_known_contracts,
                    libraries,
                    result,
                    verify,
                )
                .await
        }

        let known_contracts = flatten_contracts(&highlevel_known_contracts, true);
        let mut decoder = self.decode_traces(&script_config, &mut result, &known_contracts)?;

        if self.debug {
            return self.run_debugger(&decoder, sources, result, project, highlevel_known_contracts)
        }

        if let Some((new_traces, updated_libraries, updated_contracts)) = self
            .maybe_prepare_libraries(
                &mut script_config,
                project,
                default_known_contracts,
                relevant_predeploys,
                &mut result,
            )
            .await?
        {
            decoder = new_traces;
            highlevel_known_contracts = updated_contracts;
            libraries = updated_libraries;
        }

        if self.json {
            self.show_json(&script_config, &result)?;
        } else {
            self.show_traces(&script_config, &decoder, &mut result).await?;
        }

        verify.known_contracts = flatten_contracts(&highlevel_known_contracts, false);
        self.check_contract_sizes(&result, &highlevel_known_contracts)?;

        self.handle_broadcastable_transactions(
            result,
            libraries,
            &mut decoder,
            script_config,
            verify,
        )
        .await
    }

    // In case there are libraries to be deployed, it makes sure that these are added to the list of
    // broadcastable transactions with the appropriate sender.
    async fn maybe_prepare_libraries(
        &mut self,
        script_config: &mut ScriptConfig,
        project: Project,
        default_known_contracts: ArtifactContracts,
        predeploy_libraries: Vec<Bytes>,
        result: &mut ScriptResult,
    ) -> eyre::Result<Option<NewSenderChanges>> {
        if let Some(new_sender) = self.maybe_new_sender(
            &script_config.evm_opts,
            result.transactions.as_ref(),
            &predeploy_libraries,
        )? {
            // We have a new sender, so we need to relink all the predeployed libraries.
            let (libraries, highlevel_known_contracts) = self
                .rerun_with_new_deployer(
                    project,
                    script_config,
                    new_sender,
                    result,
                    default_known_contracts,
                )
                .await?;

            // redo traces for the new addresses
            let new_traces = self.decode_traces(
                &*script_config,
                result,
                &flatten_contracts(&highlevel_known_contracts, true),
            )?;

            return Ok(Some((new_traces, libraries, highlevel_known_contracts)))
        }

        // Add predeploy libraries to the list of broadcastable transactions.
        let mut lib_deploy = self.create_deploy_transactions(
            script_config.evm_opts.sender,
            script_config.sender_nonce,
            &predeploy_libraries,
            &script_config.evm_opts.fork_url,
        );

        if let Some(txs) = &mut result.transactions {
            for tx in txs.iter() {
                lib_deploy.push_back(BroadcastableTransaction {
                    rpc: tx.rpc.clone(),
                    transaction: TypedTransaction::Legacy(tx.transaction.clone().into()),
                });
            }
            *txs = lib_deploy;
        }

        Ok(None)
    }

    /// Resumes the deployment and/or verification of the script.
    async fn resume_deployment(
        &mut self,
        script_config: ScriptConfig,
        project: Project,
        default_known_contracts: ArtifactContracts,
        libraries: Libraries,
        result: ScriptResult,
        verify: VerifyBundle,
    ) -> eyre::Result<()> {
        if self.multi {
            return self
                .multi_chain_deployment(
                    MultiChainSequence::load(
                        &script_config.config.broadcast,
                        &self.sig,
                        script_config.target_contract(),
                    )?,
                    libraries,
                    &script_config.config,
                    result.script_wallets,
                    verify,
                )
                .await
        }
        self.resume_single_deployment(
            script_config,
            project,
            default_known_contracts,
            result,
            verify,
        )
        .await
        .map_err(|err| {
            eyre::eyre!("{err}\n\nIf you were trying to resume or verify a multi chain deployment, add `--multi` to your command invocation.") 
        })
    }

    /// Resumes the deployment and/or verification of a single RPC script.
    async fn resume_single_deployment(
        &mut self,
        script_config: ScriptConfig,
        project: Project,
        default_known_contracts: ArtifactContracts,
        result: ScriptResult,
        mut verify: VerifyBundle,
    ) -> eyre::Result<()> {
        trace!(target: "script", "resuming single deployment");

        let fork_url = self.evm_opts.ensure_fork_url()?;
        let provider = Arc::new(try_get_http_provider(fork_url)?);

        let chain = provider.get_chainid().await?.as_u64();
        verify.set_chain(&script_config.config, chain.into());

        let broadcasted = self.broadcast || self.resume;
        let mut deployment_sequence = match ScriptSequence::load(
            &script_config.config,
            &self.sig,
            script_config.target_contract(),
            chain,
            broadcasted,
        ) {
            Ok(seq) => seq,
            // If the script was simulated, but there was no attempt to broadcast yet,
            // try to read the script sequence from the `dry-run/` folder
            Err(_) if broadcasted => ScriptSequence::load(
                &script_config.config,
                &self.sig,
                script_config.target_contract(),
                chain,
                false,
            )?,
            Err(err) => eyre::bail!(err),
        };

        receipts::wait_for_pending(provider, &mut deployment_sequence).await?;

        if self.resume {
            self.send_transactions(&mut deployment_sequence, fork_url, &result.script_wallets)
                .await?;
        }

        if self.verify {
            // We might have predeployed libraries from the broadcasting, so we need to
            // relink the contracts with them, since their mapping is
            // not included in the solc cache files.
            let BuildOutput { highlevel_known_contracts, .. } = self.link(
                project,
                default_known_contracts,
                Libraries::parse(&deployment_sequence.libraries)?,
                script_config.config.sender, // irrelevant, since we're not creating any
                U256::zero(),                // irrelevant, since we're not creating any
            )?;

            verify.known_contracts = flatten_contracts(&highlevel_known_contracts, false);

            deployment_sequence.verify_contracts(&script_config.config, verify).await?;
        }

        Ok(())
    }

    /// Reruns the execution with a new sender and relinks the libraries accordingly
    async fn rerun_with_new_deployer(
        &mut self,
        project: Project,
        script_config: &mut ScriptConfig,
        new_sender: Address,
        first_run_result: &mut ScriptResult,
        default_known_contracts: ArtifactContracts,
    ) -> eyre::Result<(Libraries, ArtifactContracts<ContractBytecodeSome>)> {
        // if we had a new sender that requires relinking, we need to
        // get the nonce mainnet for accurate addresses for predeploy libs
        let nonce = foundry_utils::next_nonce(
            new_sender,
            script_config.evm_opts.fork_url.as_ref().ok_or_else(|| {
                eyre::eyre!("You must provide an RPC URL (see --fork-url) when broadcasting.")
            })?,
            None,
        )
        .await?;
        script_config.sender_nonce = nonce;

        let BuildOutput {
            libraries, contract, highlevel_known_contracts, predeploy_libraries, ..
        } = self.link(
            project,
            default_known_contracts,
            script_config.config.parsed_libraries()?,
            new_sender,
            nonce,
        )?;

        let mut txs = self.create_deploy_transactions(
            new_sender,
            nonce,
            &predeploy_libraries,
            &script_config.evm_opts.fork_url,
        );

        let result =
            self.execute(script_config, contract, new_sender, &predeploy_libraries).await?;

        if let Some(new_txs) = &result.transactions {
            for new_tx in new_txs.iter() {
                txs.push_back(BroadcastableTransaction {
                    rpc: new_tx.rpc.clone(),
                    transaction: TypedTransaction::Legacy(new_tx.transaction.clone().into()),
                });
            }
        }

        *first_run_result = result;
        first_run_result.transactions = Some(txs);

        Ok((libraries, highlevel_known_contracts))
    }

    /// In case the user has loaded *only* one private-key, we can assume that he's using it as the
    /// `--sender`
    fn maybe_load_private_key(&mut self, script_config: &mut ScriptConfig) -> eyre::Result<()> {
        if let Some(ref private_key) = self.wallets.private_key {
            self.wallets.private_keys = Some(vec![private_key.clone()]);
        }
        if let Some(wallets) = self.wallets.private_keys()? {
            if wallets.len() == 1 {
                script_config.evm_opts.sender = wallets.get(0).unwrap().address()
            }
        }
        Ok(())
    }
}

#[allow(clippy::mutable_key_type)]
fn get_relevant_predeploys(
    result: &ScriptResult,
    deploy_bytecode_to_dependencies: &BTreeMap<
        ethers::types::Bytes,
        Vec<(String, ethers::types::Bytes)>,
    >,
) -> eyre::Result<Vec<ethers::types::Bytes>> {
    let mut only_relevant_predeploys = vec![];
    if let Some(ref txs) = result.transactions {
        for tx in txs.iter() {
            only_relevant_predeploys
                .extend(relevant_from_match(tx, deploy_bytecode_to_dependencies)?);
        }
    }
    Ok(only_relevant_predeploys)
}

#[allow(clippy::mutable_key_type)]
fn relevant_from_match(
    tx: &BroadcastableTransaction,
    deploy_bytecode_to_dependencies: &BTreeMap<
        ethers::types::Bytes,
        Vec<(String, ethers::types::Bytes)>,
    >,
) -> eyre::Result<Vec<ethers::types::Bytes>> {
    match (&tx.transaction.to(), tx.transaction.data()) {
        (None, Some(data)) => {
            if let Some(info) = deploy_bytecode_to_dependencies.get(data) {
                Ok(info.iter().map(|(_, bcode)| bcode.clone()).collect::<Vec<_>>())
            } else {
                // try fuzzy get:
                if let Some(found) = deploy_bytecode_to_dependencies
                    .iter()
                    .filter_map(|entry| {
                        let score = diff_score(entry.0, data);
                        if score < 0.1 {
                            Some((OrderedFloat(score), entry))
                        } else {
                            None
                        }
                    })
                    .sorted_by_key(|(score, _)| *score)
                    .next()
                {
                    Ok(found.1 .1.iter().map(|(_, bcode)| bcode.clone()).collect::<Vec<_>>())
                } else {
                    Err(eyre::eyre!("Could not find matching known contract when determining dependencies that need to be deployed for a Create call"))
                }
            }
        }
        // TODO: Handle create2?
        // (Some(to), Some(data)) => {

        // },
        _ => Ok(vec![]),
    }
}

fn decrement_nonces(result: &mut ScriptResult, diff: usize) {
    if let Some(ref mut txs) = result.transactions {
        for tx in txs.iter_mut() {
            if let Some(nonce) = tx.transaction.nonce() {
                tx.transaction.set_nonce(nonce - diff);
            }
        }
    }
}
