use crate::cmd::{unwrap_contracts, Cmd, ScriptSequence};

use ethers::{
    abi::Abi,
    prelude::{artifacts::CompactContractBytecode, ArtifactId},
    types::{transaction::eip2718::TypedTransaction, U256},
};
use forge::executor::opts::EvmOpts;

use foundry_config::{figment::Figment, Config};
use foundry_utils::RuntimeOrHandle;

use super::*;

impl Cmd for ScriptArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        RuntimeOrHandle::new().block_on(self.run_script())
    }
}

impl ScriptArgs {
    pub async fn run_script(self) -> eyre::Result<()> {
        let figment: Figment = From::from(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut script_config = ScriptConfig {
            // dapptools compatibility
            sender_nonce: U256::one(),
            config: Config::from_provider(figment).sanitized(),
            evm_opts,
            called_function: None,
        };

        if let (Some(deployer), Some(fork_url)) =
            (self.deployer, script_config.evm_opts.fork_url.as_ref())
        {
            script_config.sender_nonce = foundry_utils::next_nonce(deployer, fork_url, None)?
        }

        let BuildOutput {
            project,
            target,
            contract,
            highlevel_known_contracts,
            predeploy_libraries,
            known_contracts: default_known_contracts,
            sources,
        } = self.build(&script_config, self.deployer)?;

        if self.force_resume || self.resume {
            let mut deployment_sequence =
                ScriptSequence::load(&self.sig, &target, &script_config.config.out)?;
            self.send_transactions(&mut deployment_sequence).await?;
        } else {
            let mut known_contracts = unwrap_contracts(&highlevel_known_contracts);

            // execute once with default sender
            let mut result = self
                .execute(&mut script_config, contract, self.deployer, &predeploy_libraries)
                .await?;

            let mut decoder = self.decode_traces(&mut result, &known_contracts)?;

            if self.debug {
                self.run_debugger(&decoder, sources, result, project, highlevel_known_contracts)?;
            } else {
                if let Some(new_sender) = self.maybe_new_sender(
                    &script_config.evm_opts,
                    result.transactions.as_ref(),
                    &predeploy_libraries,
                ) {
                    known_contracts = self
                        .rerun_with_new_deployer(
                            project,
                            &mut script_config,
                            new_sender,
                            &mut result,
                            default_known_contracts,
                        )
                        .await?;
                    // redo traces
                    decoder = self.decode_traces(&mut result, &known_contracts)?;
                } else {
                    // prepend predeploy libraries
                    let mut lib_deploy = self.create_transactions_from_data(
                        self.deployer,
                        script_config.sender_nonce,
                        &predeploy_libraries,
                    );

                    if let Some(txs) = &mut result.transactions {
                        txs.iter().for_each(|tx| {
                            lib_deploy.push_back(TypedTransaction::Legacy(into_legacy(tx.clone())));
                        });
                        *txs = lib_deploy;
                    }
                }

                self.show_traces(&script_config, &decoder, &mut result)?;

                self.handle_broadcastable_transactions(
                    &target,
                    result,
                    &mut decoder,
                    &script_config,
                )
                .await?;
            }
        }

        Ok(())
    }
}

impl ScriptArgs {
    /// Reruns the execution with a new sender and relinks the libraries accordingly
    async fn rerun_with_new_deployer(
        &self,
        project: Project,
        script_config: &mut ScriptConfig,
        new_sender: Address,
        first_run_result: &mut ScriptResult,
        default_known_contracts: BTreeMap<ArtifactId, CompactContractBytecode>,
    ) -> eyre::Result<BTreeMap<ArtifactId, (Abi, Vec<u8>)>> {
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
        )?;
        script_config.sender_nonce = nonce;

        let BuildOutput { contract, highlevel_known_contracts, predeploy_libraries, .. } =
            self.link(project, default_known_contracts, new_sender, nonce)?;

        first_run_result.transactions =
            Some(self.create_transactions_from_data(Some(new_sender), nonce, &predeploy_libraries));

        let result =
            self.execute(script_config, contract, Some(new_sender), &predeploy_libraries).await?;

        first_run_result.success &= result.success;
        first_run_result.gas = result.gas;
        first_run_result.logs = result.logs;
        first_run_result.traces.extend(result.traces);
        first_run_result.debug = result.debug;
        first_run_result.labeled_addresses.extend(result.labeled_addresses);

        match (&mut first_run_result.transactions, result.transactions) {
            (Some(txs), Some(new_txs)) => {
                new_txs.iter().for_each(|tx| {
                    txs.push_back(TypedTransaction::Legacy(into_legacy(tx.clone())));
                });
            }
            (None, Some(new_txs)) => {
                first_run_result.transactions = Some(new_txs);
            }
            _ => {}
        }

        Ok(unwrap_contracts(&highlevel_known_contracts))
    }

    pub async fn handle_broadcastable_transactions(
        &self,
        target: &ArtifactId,
        result: ScriptResult,
        decoder: &mut CallTraceDecoder,
        script_config: &ScriptConfig,
    ) -> eyre::Result<()> {
        if let Some(txs) = result.transactions {
            println!("==========================");
            println!("Simulated On-chain Traces:\n");
            if let Ok(gas_filled_txs) = self.execute_transactions(txs, script_config, decoder).await
            {
                println!("\n\n==========================");
                if !result.success {
                    panic!("\nSIMULATION FAILED");
                } else {
                    let txs = gas_filled_txs;
                    let mut deployment_sequence =
                        ScriptSequence::new(txs, &self.sig, target, &script_config.config.out)?;
                    deployment_sequence.save()?;

                    if self.broadcast {
                        self.send_transactions(&mut deployment_sequence).await?;
                    } else {
                        println!("\nSIMULATION COMPLETE. To broadcast these transactions, add --broadcast and wallet configuration(s) to the previous command. See forge script --help for more.");
                    }
                }
            } else {
                eyre::bail!("One or more transactions failed when simulating the on-chain version. Check the trace by re-running with `-vvv`")
            }
        } else if self.broadcast {
            eyre::bail!("No onchain transactions generated in script");
        }
        Ok(())
    }
}
