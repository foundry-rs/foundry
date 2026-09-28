//! Ethereum script execution using the evm2 executor.

use crate::{
    ScriptResult,
    execute::{ExecutedState, PreExecutionState},
};
use alloy_primitives::{Bytes, U256};
use eyre::{Result, ensure};
use foundry_cheatcodes::{CheatsConfig, ethereum::CheatcodeAccessMode};
use foundry_cli::utils::needs_setup;
use foundry_config::Config;
use foundry_evm::{
    constants::CALLER,
    core::{
        ethereum::{EthereumEnv, LocalState},
        evm::EthEvmNetwork,
    },
    ethereum::EthereumExecutor,
};
use std::sync::Arc;

impl PreExecutionState<EthEvmNetwork> {
    /// Executes a linked Ethereum script with evm2, retaining the existing script pipeline.
    pub async fn execute_ethereum(self) -> Result<ExecutedState<EthEvmNetwork>> {
        ensure!(
            self.script_config.evm_opts.fork_url.is_none(),
            "evm2 script forks are not yet supported"
        );
        ensure!(
            self.build_data.predeploy_libraries.libraries_count() == 0,
            "evm2 script library predeployment is not yet supported"
        );
        ensure!(!self.args.debug, "evm2 script debugging is not yet supported");

        let opts = &self.script_config.evm_opts;
        let sender = opts.sender;
        let mut state = LocalState::default();
        if !self.args.broadcast && sender == Config::DEFAULT_SENDER {
            state.set_balance(sender, U256::MAX)?;
        }
        state.set_balance(CALLER, U256::MAX)?;
        let env = EthereumEnv::local_from_config(&self.script_config.config, opts)?;
        let cheats = Arc::new(CheatsConfig::new(
            &self.script_config.config,
            opts.clone(),
            Some(self.build_data.known_contracts.clone()),
            Some(self.build_data.build_data.target.clone()),
            false,
        ));
        let mut executor =
            EthereumExecutor::new_foundry(env, state, cheats, CheatcodeAccessMode::Local);
        if !self.args.broadcast {
            executor.deploy_create2_deployer()?;
        }
        executor.state_mut().set_nonce(sender, self.script_config.sender_nonce)?;

        let deployment_nonce = if sender == CALLER { u64::MAX / 2 } else { 0 };
        let script_address = CALLER.create(deployment_nonce);
        executor.state_mut().set_balance(script_address, opts.initial_balance)?;
        if sender == CALLER {
            executor.state_mut().set_nonce(sender, u64::MAX / 2)?;
        }
        let deployment =
            executor.deploy(CALLER, self.execution_data.bytecode.clone(), U256::ZERO)?;
        if sender == CALLER {
            executor.state_mut().set_nonce(sender, self.script_config.sender_nonce)?;
        }
        ensure!(deployment.status, "Failed to deploy script: {:?}", deployment.stop);
        let address =
            deployment.created_address.expect("successful script deployment has an address");
        let mut logs = executor.inspector_mut().take_logs();

        if needs_setup(&self.execution_data.abi) {
            let setup = executor.transact_raw(
                sender,
                address,
                Bytes::copy_from_slice(&alloy_primitives::keccak256("setUp()")[..4]),
                U256::ZERO,
            )?;
            logs.extend(executor.inspector_mut().take_logs());
            if !setup.status {
                let gas_used = setup.tx_gas_used();
                return Ok(self.into_executed(ScriptResult {
                    returned: setup.output,
                    success: false,
                    gas_used,
                    logs,
                    exit_reason: Some(format!("{:?}", setup.stop)),
                    ..Default::default()
                }));
            }
        }

        let (execution, mut inspector) = executor.inspect_raw(
            sender,
            address,
            self.execution_data.calldata.clone(),
            U256::ZERO,
        )?;
        logs.extend(inspector.take_logs());
        let gas_used = execution.tx_gas_used();
        let result = ScriptResult {
            returned: execution.output,
            success: execution.status,
            gas_used,
            logs,
            exit_reason: Some(format!("{:?}", execution.stop)),
            ..Default::default()
        };

        Ok(self.into_executed(result))
    }

    fn into_executed(
        self,
        execution_result: ScriptResult<alloy_network::Ethereum>,
    ) -> ExecutedState<EthEvmNetwork> {
        ExecutedState {
            args: self.args,
            script_config: self.script_config,
            script_wallets: self.script_wallets,
            browser_wallet: self.browser_wallet,
            build_data: self.build_data,
            execution_data: self.execution_data,
            execution_result,
        }
    }
}
