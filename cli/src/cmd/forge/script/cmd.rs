use super::{sequence::ScriptSequence, *};
use crate::cmd::{forge::script::verify::VerifyBundle, unwrap_contracts, LoadConfig};
use ethers::{
    prelude::{artifacts::CompactContractBytecode, ArtifactId, Middleware, Signer},
    types::{transaction::eip2718::TypedTransaction, U256},
};
use foundry_common::get_http_provider;
use std::sync::Arc;
use tracing::trace;

impl ScriptArgs {
    /// Executes the script
    pub async fn run_script(mut self) -> eyre::Result<()> {
        trace!("executing script command");

        let (config, evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;
        let mut script_config = ScriptConfig {
            // dapptools compatibility
            sender_nonce: U256::one(),
            config,
            evm_opts,
            called_function: None,
            backend: None,
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

        let build_output = self.compile(&script_config)?;

        let mut verify = VerifyBundle::new(
            &build_output.project,
            &script_config.config,
            unwrap_contracts(&build_output.highlevel_known_contracts, false),
            self.retry.clone(),
        );

        let BuildOutput {
            project,
            target,
            contract,
            mut highlevel_known_contracts,
            predeploy_libraries,
            known_contracts: default_known_contracts,
            sources,
            mut libraries,
        } = build_output;

        // Execute once with default sender.
        let sender = script_config.evm_opts.sender;
        let mut result =
            self.execute(&mut script_config, contract, sender, &predeploy_libraries).await?;

        if self.resume || (self.verify && !self.broadcast) {
            let fork_url = self
                .evm_opts
                .fork_url
                .clone()
                .ok_or_else(|| eyre::eyre!("You must provide an RPC URL (see --fork-url)."))?;

            let provider = Arc::new(get_http_provider(&fork_url));
            let chain = provider.get_chainid().await?.as_u64();

            verify.set_chain(&script_config.config, chain.into());

            let broadcasted = self.broadcast || self.resume;
            let mut deployment_sequence = match ScriptSequence::load(
                &script_config.config,
                &self.sig,
                &target,
                chain,
                broadcasted,
            ) {
                Ok(seq) => seq,
                // If the script was simulated, but there was no attempt to broadcast yet,
                // try to read the script sequence from the `dry-run/` folder
                Err(_) if broadcasted => {
                    ScriptSequence::load(&script_config.config, &self.sig, &target, chain, false)?
                }
                Err(err) => eyre::bail!(err),
            };

            receipts::wait_for_pending(provider, &mut deployment_sequence).await?;

            if self.resume {
                self.send_transactions(&mut deployment_sequence, &fork_url, result.script_wallets)
                    .await?;
            }

            if self.verify {
                // We might have predeployed libraries from the broadcasting, so we need to relink
                // the contracts with them, since their mapping is not included in the solc cache
                // files.
                let BuildOutput { highlevel_known_contracts, .. } = self.link(
                    project,
                    default_known_contracts,
                    Libraries::parse(&deployment_sequence.libraries)?,
                    script_config.config.sender, // irrelevant, since we're not creating any
                    U256::zero(),                // irrelevant, since we're not creating any
                )?;

                verify.known_contracts = unwrap_contracts(&highlevel_known_contracts, false);

                deployment_sequence.verify_contracts(verify, chain).await?;
            }
        } else {
            let known_contracts = unwrap_contracts(&highlevel_known_contracts, true);

            let mut decoder = self.decode_traces(&script_config, &mut result, &known_contracts)?;

            if self.debug {
                self.run_debugger(&decoder, sources, result, project, highlevel_known_contracts)?;
            } else {
                if let Some(new_sender) = self.maybe_new_sender(
                    &script_config.evm_opts,
                    result.transactions.as_ref(),
                    &predeploy_libraries,
                )? {
                    // We have a new sender, so we need to relink all the predeployed libraries.
                    (libraries, highlevel_known_contracts) = self
                        .rerun_with_new_deployer(
                            project,
                            &mut script_config,
                            new_sender,
                            &mut result,
                            default_known_contracts,
                        )
                        .await?;
                    // redo traces
                    decoder = self.decode_traces(
                        &script_config,
                        &mut result,
                        &unwrap_contracts(&highlevel_known_contracts, true),
                    )?;
                } else {
                    // Add predeploy libraries to the list of broadcastable transactions.
                    let mut lib_deploy = self.create_deploy_transactions(
                        script_config.evm_opts.sender,
                        script_config.sender_nonce,
                        &predeploy_libraries,
                    );

                    if let Some(txs) = &mut result.transactions {
                        for tx in txs.iter() {
                            lib_deploy.push_back(TypedTransaction::Legacy(tx.clone().into()));
                        }
                        *txs = lib_deploy;
                    }
                }

                if self.json {
                    self.show_json(&script_config, &result)?;
                } else {
                    self.show_traces(&script_config, &decoder, &mut result).await?;
                }

                verify.known_contracts = unwrap_contracts(&highlevel_known_contracts, false);

                self.check_contract_sizes(
                    result.transactions.as_ref(),
                    &highlevel_known_contracts,
                )?;

                self.handle_broadcastable_transactions(
                    &target,
                    result,
                    libraries,
                    &mut decoder,
                    script_config,
                    verify,
                )
                .await?;
            }
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
        default_known_contracts: BTreeMap<ArtifactId, CompactContractBytecode>,
    ) -> eyre::Result<(Libraries, BTreeMap<ArtifactId, ContractBytecodeSome>)> {
        // if we had a new sender that requires relinking, we need to
        // get the nonce mainnet for accurate addresses for predeploy libs
        let nonce = foundry_utils::next_nonce(
            new_sender,
            script_config
                .evm_opts
                .fork_url
                .as_ref()
                .expect("You must provide an RPC URL (see --fork-url) when broadcasting."),
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

        let mut txs = self.create_deploy_transactions(new_sender, nonce, &predeploy_libraries);

        let result =
            self.execute(script_config, contract, new_sender, &predeploy_libraries).await?;

        if let Some(new_txs) = &result.transactions {
            for new_tx in new_txs.iter() {
                txs.push_back(TypedTransaction::Legacy(new_tx.clone().into()));
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
