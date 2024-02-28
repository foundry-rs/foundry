use super::{
    multi::MultiChainSequence, sequence::ScriptSequence, verify::VerifyBundle, ScriptArgs,
    ScriptConfig,
};
use crate::cmd::script::{
    build::{LinkedBuildData, PreprocessedState},
    executor::ExecutedState,
    receipts,
};
use alloy_primitives::Address;
use ethers_providers::Middleware;
use ethers_signers::Signer;
use eyre::Result;
use foundry_cli::utils::LoadConfig;
use foundry_common::{provider::ethers::try_get_http_provider, types::ToAlloy};
use foundry_compilers::artifacts::Libraries;
use foundry_debugger::Debugger;
use foundry_wallets::WalletSigner;
use std::{collections::HashMap, sync::Arc};

impl ScriptArgs {
    /// Executes the script
    pub async fn run_script(mut self) -> Result<()> {
        trace!(target: "script", "executing script command");

        let (config, evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;
        let mut script_config =
            ScriptConfig { config, evm_opts, debug: self.debug, ..Default::default() };

        if let Some(sender) = self.maybe_load_private_key()? {
            script_config.evm_opts.sender = sender;
        }

        if script_config.evm_opts.fork_url.is_none() {
            // if not forking, then ignore any pre-deployed library addresses
            script_config.config.libraries = Default::default();
        }

        let state = PreprocessedState { args: self.clone(), script_config: script_config.clone() };
        let ExecutedState { build_data, execution_result, execution_data, .. } =
            state.compile()?.link()?.prepare_execution().await?.execute().await?;
        script_config.target_contract = Some(build_data.build_data.target.clone());
        script_config.called_function = Some(execution_data.func);

        let mut verify = VerifyBundle::new(
            &script_config.config.project()?,
            &script_config.config,
            build_data.get_flattened_contracts(false),
            self.retry,
            self.verifier.clone(),
        );

        let script_wallets = execution_data.script_wallets;

        // We need to execute the script even if just resuming, in case we need to collect private
        // keys from the execution.
        let mut result = execution_result;

        if self.resume || (self.verify && !self.broadcast) {
            let signers = script_wallets.into_multi_wallet().into_signers()?;
            return self.resume_deployment(script_config, build_data, verify, &signers).await;
        }

        let known_contracts = build_data.get_flattened_contracts(true);
        let decoder = self.decode_traces(&script_config, &mut result, &known_contracts)?;

        if self.debug {
            let mut debugger = Debugger::builder()
                .debug_arenas(result.debug.as_deref().unwrap_or_default())
                .decoder(&decoder)
                .sources(build_data.build_data.sources.clone())
                .breakpoints(result.breakpoints.clone())
                .build();
            debugger.try_run()?;
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
