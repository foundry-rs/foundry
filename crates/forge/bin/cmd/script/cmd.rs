use super::{verify::VerifyBundle, ScriptArgs, ScriptConfig};
use crate::cmd::script::{
    build::{LinkedBuildData, PreprocessedState},
    receipts,
};
use alloy_primitives::Address;
use ethers_signers::Signer;
use eyre::Result;
use forge::inspectors::cheatcodes::ScriptWallets;
use foundry_cli::utils::LoadConfig;
use foundry_common::{provider::ethers::try_get_http_provider, shell, types::ToAlloy};
use foundry_compilers::artifacts::Libraries;
use std::sync::Arc;

impl ScriptArgs {
    async fn preprocess(self) -> Result<PreprocessedState> {
        let script_wallets =
            ScriptWallets::new(self.wallets.get_multi_wallet().await?, self.evm_opts.sender);

        let (config, mut evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;

        if let Some(sender) = self.maybe_load_private_key()? {
            evm_opts.sender = sender;
        }

        let script_config = ScriptConfig::new(config, evm_opts).await?;

        Ok(PreprocessedState { args: self, script_config, script_wallets })
    }

    /// Executes the script
    pub async fn run_script(self) -> Result<()> {
        trace!(target: "script", "executing script command");

        let state = self
            .preprocess()
            .await?
            .compile()?
            .link()?
            .prepare_execution()
            .await?
            .execute()
            .await?
            .prepare_simulation()
            .await?;

        if state.args.debug {
            state.run_debugger()?;
        }

        let state = if state.args.resume || (state.args.verify && !state.args.broadcast) {
            state.resume().await?
        } else {
            if state.args.json {
                state.show_json()?;
            } else {
                state.show_traces().await?;
            }
            state.args.check_contract_sizes(
                &state.execution_result,
                &state.build_data.highlevel_known_contracts,
            )?;

            if state.script_config.missing_rpc {
                shell::println("\nIf you wish to simulate on-chain transactions pass a RPC URL.")?;
                return Ok(());
            }

            let state = state.fill_metadata().await?;

            if state.transactions.is_empty() {
                return Ok(());
            }

            state.bundle().await?
        };

        if !state.args.broadcast && !state.args.resume {
            shell::println("\nSIMULATION COMPLETE. To broadcast these transactions, add --broadcast and wallet configuration(s) to the previous command. See forge script --help for more.")?;
            return Ok(());
        }

        if state.args.verify {
            state.verify_preflight_check()?;
        }

        let state = state.wait_for_pending().await?.broadcast().await?;

        if state.args.verify {
            state.verify().await?;
        }

        Ok(())
    }

    /*/// Resumes the deployment and/or verification of the script.
    async fn resume_deployment(
        &mut self,
        script_config: ScriptConfig,
        build_data: LinkedBuildData,
        verify: VerifyBundle,
    ) -> Result<()> {
        if self.multi {
            return self
                .multi_chain_deployment(
                    &mut MultiChainSequence::load(
                        &script_config.config,
                        &self.sig,
                        &build_data.build_data.target,
                    )?,
                    build_data.libraries,
                    &script_config.config,
                    verify,
                    &script_config.script_wallets.into_multi_wallet().into_signers()?,
                )
                .await;
        }
        self.resume_single_deployment(
            script_config,
            build_data,
            verify,
        )
        .await
        .map_err(|err| {
            eyre::eyre!("{err}\n\nIf you were trying to resume or verify a multi chain deployment, add `--multi` to your command invocation.")
        })
    }*/

    /*/// Resumes the deployment and/or verification of a single RPC script.
    async fn resume_single_deployment(
        &mut self,
        script_config: ScriptConfig,
        build_data: LinkedBuildData,
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
            &build_data.build_data.target,
            chain,
            broadcasted,
        ) {
            Ok(seq) => seq,
            // If the script was simulated, but there was no attempt to broadcast yet,
            // try to read the script sequence from the `dry-run/` folder
            Err(_) if broadcasted => ScriptSequence::load(
                &script_config.config,
                &self.sig,
                &build_data.build_data.target,
                chain,
                false,
            )?,
            Err(err) => eyre::bail!(err),
        };

        if self.verify {
            deployment_sequence.verify_preflight_check(&script_config.config, &verify)?;
        }

        receipts::wait_for_pending(provider, &mut deployment_sequence).await?;

        let signers = script_config.script_wallets.into_multi_wallet().into_signers()?;

        if self.resume {
            self.send_transactions(&mut deployment_sequence, fork_url, &signers).await?;
        }

        if self.verify {
            let libraries = Libraries::parse(&deployment_sequence.libraries)?
                .with_stripped_file_prefixes(build_data.build_data.linker.root.as_path());
            // We might have predeployed libraries from the broadcasting, so we need to
            // relink the contracts with them, since their mapping is
            // not included in the solc cache files.
            let build_data = build_data.build_data.link_with_libraries(libraries)?;

            verify.known_contracts = build_data.get_flattened_contracts(false);

            deployment_sequence.verify_contracts(&script_config.config, verify).await?;
        }

        Ok(())
    }*/

    /// In case the user has loaded *only* one private-key, we can assume that he's using it as the
    /// `--sender`
    fn maybe_load_private_key(&self) -> Result<Option<Address>> {
        let maybe_sender = self
            .wallets
            .private_keys()?
            .filter(|pks| pks.len() == 1)
            .map(|pks| pks.first().unwrap().address().to_alloy());
        Ok(maybe_sender)
    }
}
