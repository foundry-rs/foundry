use super::{
    multi::MultiChainSequence, sequence::ScriptSequence, verify::VerifyBundle, ScriptArgs,
    ScriptConfig, ScriptResult,
};
use crate::cmd::script::{
    build::{LinkedBuildData, LinkedState, PreprocessedState},
    receipts,
};
use alloy_primitives::Address;
use ethers_providers::Middleware;
use ethers_signers::Signer;
use eyre::Result;
use forge::traces::CallTraceDecoder;
use foundry_cli::utils::LoadConfig;
use foundry_common::{provider::ethers::try_get_http_provider, types::ToAlloy};
use foundry_compilers::artifacts::Libraries;
use foundry_debugger::Debugger;
use foundry_evm::inspectors::cheatcodes::{BroadcastableTransaction, ScriptWallets};
use foundry_wallets::WalletSigner;
use std::{collections::HashMap, sync::Arc};

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

        if let Some(sender) = self.maybe_load_private_key()? {
            script_config.evm_opts.sender = sender;
        }

        if let Some(ref fork_url) = script_config.evm_opts.fork_url {
            // when forking, override the sender's nonce to the onchain value
            script_config.sender_nonce =
                forge::next_nonce(script_config.evm_opts.sender, fork_url, None).await?
        } else {
            // if not forking, then ignore any pre-deployed library addresses
            script_config.config.libraries = Default::default();
        }

        let state = PreprocessedState { args: self.clone(), script_config: script_config.clone() };

        let LinkedState { build_data, .. } = state.compile()?.link()?;
        script_config.target_contract = Some(build_data.build_data.target.clone());

        let mut verify = VerifyBundle::new(
            &script_config.config.project()?,
            &script_config.config,
            build_data.get_flattened_contracts(false),
            self.retry,
            self.verifier.clone(),
        );

        // Execute once with default sender.
        let sender = script_config.evm_opts.sender;

        let multi_wallet = self.wallets.get_multi_wallet().await?;
        let script_wallets = ScriptWallets::new(multi_wallet, self.evm_opts.sender);

        let contract = build_data.get_target_contract()?;

        // We need to execute the script even if just resuming, in case we need to collect private
        // keys from the execution.
        let mut result = self
            .execute(
                &mut script_config,
                contract,
                sender,
                &build_data.predeploy_libraries,
                script_wallets.clone(),
            )
            .await?;

        if self.resume || (self.verify && !self.broadcast) {
            let signers = script_wallets.into_multi_wallet().into_signers()?;
            return self.resume_deployment(script_config, build_data, verify, &signers).await;
        }

        let known_contracts = build_data.get_flattened_contracts(true);
        let mut decoder = self.decode_traces(&script_config, &mut result, &known_contracts)?;

        if self.debug {
            let mut debugger = Debugger::builder()
                .debug_arenas(result.debug.as_deref().unwrap_or_default())
                .decoder(&decoder)
                .sources(build_data.build_data.sources.clone())
                .breakpoints(result.breakpoints.clone())
                .build();
            debugger.try_run()?;
        }

        let (maybe_new_traces, build_data) = self
            .maybe_prepare_libraries(
                &mut script_config,
                build_data,
                &mut result,
                script_wallets.clone(),
            )
            .await?;

        if let Some(new_traces) = maybe_new_traces {
            decoder = new_traces;
        }

        if self.json {
            self.show_json(&script_config, &result)?;
        } else {
            self.show_traces(&script_config, &decoder, &mut result).await?;
        }

        verify.known_contracts = build_data.get_flattened_contracts(false);
        self.check_contract_sizes(&result, &build_data.highlevel_known_contracts)?;

        let signers = script_wallets.into_multi_wallet().into_signers()?;

        self.handle_broadcastable_transactions(
            result,
            build_data.libraries,
            &decoder,
            script_config,
            verify,
            &signers,
        )
        .await
    }

    // In case there are libraries to be deployed, it makes sure that these are added to the list of
    // broadcastable transactions with the appropriate sender.
    async fn maybe_prepare_libraries(
        &mut self,
        script_config: &mut ScriptConfig,
        build_data: LinkedBuildData,
        result: &mut ScriptResult,
        script_wallets: ScriptWallets,
    ) -> Result<(Option<CallTraceDecoder>, LinkedBuildData)> {
        if let Some(new_sender) = self.maybe_new_sender(
            &script_config.evm_opts,
            result.transactions.as_ref(),
            !build_data.predeploy_libraries.is_empty(),
        )? {
            // We have a new sender, so we need to relink all the predeployed libraries.
            let build_data = self
                .rerun_with_new_deployer(
                    script_config,
                    new_sender,
                    result,
                    build_data,
                    script_wallets,
                )
                .await?;

            // redo traces for the new addresses
            let new_traces = self.decode_traces(
                &*script_config,
                result,
                &build_data.get_flattened_contracts(true),
            )?;

            return Ok((Some(new_traces), build_data));
        }

        // Add predeploy libraries to the list of broadcastable transactions.
        let mut lib_deploy = self.create_deploy_transactions(
            script_config.evm_opts.sender,
            script_config.sender_nonce,
            &build_data.predeploy_libraries,
            &script_config.evm_opts.fork_url,
        );

        if let Some(txs) = &mut result.transactions {
            for tx in txs.iter() {
                lib_deploy.push_back(BroadcastableTransaction {
                    rpc: tx.rpc.clone(),
                    transaction: tx.transaction.clone(),
                });
            }
            *txs = lib_deploy;
        }

        Ok((None, build_data))
    }

    /// Resumes the deployment and/or verification of the script.
    async fn resume_deployment(
        &mut self,
        script_config: ScriptConfig,
        build_data: LinkedBuildData,
        verify: VerifyBundle,
        signers: &HashMap<Address, WalletSigner>,
    ) -> Result<()> {
        if self.multi {
            return self
                .multi_chain_deployment(
                    MultiChainSequence::load(
                        &script_config.config,
                        &self.sig,
                        script_config.target_contract(),
                    )?,
                    build_data.libraries,
                    &script_config.config,
                    verify,
                    signers,
                )
                .await;
        }
        self.resume_single_deployment(
            script_config,
            build_data,
            verify,
            signers,
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
        build_data: LinkedBuildData,
        mut verify: VerifyBundle,
        signers: &HashMap<Address, WalletSigner>,
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

        if self.verify {
            deployment_sequence.verify_preflight_check(&script_config.config, &verify)?;
        }

        receipts::wait_for_pending(provider, &mut deployment_sequence).await?;

        if self.resume {
            self.send_transactions(&mut deployment_sequence, fork_url, signers).await?;
        }

        if self.verify {
            let libraries = Libraries::parse(&deployment_sequence.libraries)?
                .with_stripped_file_prefixes(build_data.build_data.linker.root.as_path());
            // We might have predeployed libraries from the broadcasting, so we need to
            // relink the contracts with them, since their mapping is
            // not included in the solc cache files.
            let build_data = build_data.build_data.link_with_libraries(libraries)?;

            verify.known_contracts = build_data.get_flattened_contracts(true);

            deployment_sequence.verify_contracts(&script_config.config, verify).await?;
        }

        Ok(())
    }

    /// Reruns the execution with a new sender and relinks the libraries accordingly
    async fn rerun_with_new_deployer(
        &mut self,
        script_config: &mut ScriptConfig,
        new_sender: Address,
        first_run_result: &mut ScriptResult,
        build_data: LinkedBuildData,
        script_wallets: ScriptWallets,
    ) -> Result<LinkedBuildData> {
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

        let libraries = script_config.config.libraries_with_remappings()?;

        let build_data = build_data.build_data.link(libraries, new_sender, nonce)?;
        let contract = build_data.get_target_contract()?;

        let mut txs = self.create_deploy_transactions(
            new_sender,
            nonce,
            &build_data.predeploy_libraries,
            &script_config.evm_opts.fork_url,
        );

        let result = self
            .execute(
                script_config,
                contract,
                new_sender,
                &build_data.predeploy_libraries,
                script_wallets,
            )
            .await?;

        if let Some(new_txs) = &result.transactions {
            for new_tx in new_txs.iter() {
                txs.push_back(BroadcastableTransaction {
                    rpc: new_tx.rpc.clone(),
                    transaction: new_tx.transaction.clone(),
                });
            }
        }

        *first_run_result = result;
        first_run_result.transactions = Some(txs);

        Ok(build_data)
    }

    /// In case the user has loaded *only* one private-key, we can assume that he's using it as the
    /// `--sender`
    fn maybe_load_private_key(&mut self) -> Result<Option<Address>> {
        let maybe_sender = self
            .wallets
            .private_keys()?
            .filter(|pks| pks.len() == 1)
            .map(|pks| pks.first().unwrap().address().to_alloy());
        Ok(maybe_sender)
    }
}
