use super::{multi::MultiChainSequence, sequence::ScriptSequence, verify::VerifyBundle, *};
use alloy_primitives::Bytes;
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_providers::Middleware;
use ethers_signers::Signer;
use eyre::Result;
use foundry_cli::utils::LoadConfig;
use foundry_common::{contracts::flatten_contracts, try_get_http_provider, types::ToAlloy};
use foundry_debugger::DebuggerArgs;
use std::sync::Arc;

/// Helper alias type for the collection of data changed due to the new sender.
type NewSenderChanges = (CallTraceDecoder, Libraries, ArtifactContracts<ContractBytecodeSome>);

impl ScriptArgs {
    /// Executes the script
    pub async fn run_script(mut self) -> Result<()> {
        trace!(target: "script", "executing script command");

        let (config, evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;
        let mut script_config = ScriptConfig {
            // dapptools compatibility
            sender_nonce: 1,
            config,
            evm_opts,
            debug: self.debug,
            ..Default::default()
        };

        self.maybe_load_private_key(&mut script_config)?;

        if let Some(ref fork_url) = script_config.evm_opts.fork_url {
            // when forking, override the sender's nonce to the onchain value
            script_config.sender_nonce =
                forge::next_nonce(script_config.evm_opts.sender, fork_url, None).await?
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
            let debugger = DebuggerArgs {
                debug: result.debug.clone().unwrap_or_default(),
                decoder: &decoder,
                sources,
                breakpoints: result.breakpoints.clone(),
            };
            debugger.run()?;
        }

        if let Some((new_traces, updated_libraries, updated_contracts)) = self
            .maybe_prepare_libraries(
                &mut script_config,
                project,
                default_known_contracts,
                predeploy_libraries,
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

        self.handle_broadcastable_transactions(result, libraries, &decoder, script_config, verify)
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
    ) -> Result<Option<NewSenderChanges>> {
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
    ) -> Result<()> {
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
    ) -> Result<()> {
        trace!(target: "script", "resuming single deployment");

        let fork_url = script_config
            .evm_opts
            .fork_url
            .as_deref()
            .ok_or_else(|| eyre::eyre!("Missing `--fork-url` field."))?;
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
                0,                           // irrelevant, since we're not creating any
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
    ) -> Result<(Libraries, ArtifactContracts<ContractBytecodeSome>)> {
        // if we had a new sender that requires relinking, we need to
        // get the nonce mainnet for accurate addresses for predeploy libs
        let nonce = forge::next_nonce(
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
    fn maybe_load_private_key(&mut self, script_config: &mut ScriptConfig) -> Result<()> {
        if let Some(ref private_key) = self.wallets.private_key {
            self.wallets.private_keys = Some(vec![private_key.clone()]);
        }
        if let Some(wallets) = self.wallets.private_keys()? {
            if wallets.len() == 1 {
                script_config.evm_opts.sender = wallets.first().unwrap().address().to_alloy()
            }
        }
        Ok(())
    }
}
