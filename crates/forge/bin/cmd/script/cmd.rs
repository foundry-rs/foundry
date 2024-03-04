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

        let pre_simulation = self
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

        if pre_simulation.args.debug {
            pre_simulation.run_debugger()?;
        }

        if pre_simulation.args.json {
            pre_simulation.show_json()?;
        } else {
            pre_simulation.show_traces().await?;
        }

        // Move from `PreSimulationState` to `BundledState` either by resuming or simulating
        // transactions.
        let bundled = if pre_simulation.args.resume ||
            (pre_simulation.args.verify && !pre_simulation.args.broadcast)
        {
            pre_simulation.resume().await?
        } else {
            pre_simulation.args.check_contract_sizes(
                &pre_simulation.execution_result,
                &pre_simulation.build_data.highlevel_known_contracts,
            )?;

            if pre_simulation.execution_artifacts.rpc_data.missing_rpc {
                shell::println("\nIf you wish to simulate on-chain transactions pass a RPC URL.")?;
                return Ok(());
            }

            let state = pre_simulation.fill_metadata().await?;

            if state.transactions.is_empty() {
                return Ok(());
            }

            state.bundle().await?
        };

        if !bundled.args.broadcast && !bundled.args.resume {
            shell::println("\nSIMULATION COMPLETE. To broadcast these transactions, add --broadcast and wallet configuration(s) to the previous command. See forge script --help for more.")?;
            return Ok(());
        }

        if bundled.args.verify {
            bundled.verify_preflight_check()?;
        }

        let broadcasted = bundled.wait_for_pending().await?.broadcast().await?;

        if broadcasted.args.verify {
            broadcasted.verify().await?;
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
