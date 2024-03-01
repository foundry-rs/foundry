use super::{ScriptArgs, ScriptConfig};
use crate::cmd::script::build::PreprocessedState;
use alloy_primitives::Address;
use ethers_signers::Signer;
use eyre::Result;
use forge::inspectors::cheatcodes::ScriptWallets;
use foundry_cli::utils::LoadConfig;
use foundry_common::{shell, types::ToAlloy};

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
