use crate::{
    cmd::{unwrap_contracts, ScriptSequence},
    utils::get_http_provider,
};
use ethers::{
    abi::Abi,
    prelude::{artifacts::CompactContractBytecode, ArtifactId, Middleware, Signer},
    types::{transaction::eip2718::TypedTransaction, U256},
};
use forge::executor::opts::EvmOpts;
use foundry_config::{figment::Figment, Config};

use super::*;

impl ScriptArgs {
    pub async fn run_script(mut self) -> eyre::Result<()> {
        let figment: Figment = From::from(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut script_config = ScriptConfig {
            // dapptools compatibility
            sender_nonce: U256::one(),
            config: Config::from_provider(figment).sanitized(),
            evm_opts,
            called_function: None,
        };

        self.maybe_load_private_key(&mut script_config)?;

        if let Some(fork_url) = script_config.evm_opts.fork_url.as_ref() {
            script_config.sender_nonce =
                foundry_utils::next_nonce(script_config.evm_opts.sender, fork_url, None).await?
        } else {
            script_config.config.libraries = Default::default();
        }

        let BuildOutput {
            project,
            target,
            contract,
            highlevel_known_contracts,
            predeploy_libraries,
            known_contracts: default_known_contracts,
            sources,
        } = self.build(&script_config)?;

        if self.resume {
            let fork_url = self
                .evm_opts
                .fork_url
                .as_ref()
                .expect("You must provide an RPC URL (see --fork-url).")
                .clone();

            let provider = get_http_provider(&fork_url);
            let chain = provider.get_chainid().await?.as_u64();

            let mut deployment_sequence =
                ScriptSequence::load(&script_config.config, &self.sig, &target, chain)?;
            self.send_transactions(&mut deployment_sequence, &fork_url).await?;
        } else {
            let mut known_contracts = unwrap_contracts(&highlevel_known_contracts);

            // execute once with default sender
            let sender = script_config.evm_opts.sender;
            let mut result =
                self.execute(&mut script_config, contract, sender, &predeploy_libraries).await?;

            let mut decoder = self.decode_traces(&script_config, &mut result, &known_contracts)?;

            if self.debug {
                self.run_debugger(&decoder, sources, result, project, highlevel_known_contracts)?;
            } else {
                if let Some(new_sender) = self.maybe_new_sender(
                    &script_config.evm_opts,
                    result.transactions.as_ref(),
                    &predeploy_libraries,
                )? {
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
                    decoder = self.decode_traces(&script_config, &mut result, &known_contracts)?;
                } else {
                    // prepend predeploy libraries
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

                self.show_traces(&script_config, &decoder, &mut result)?;

                self.handle_broadcastable_transactions(
                    &target,
                    result.transactions,
                    &mut decoder,
                    &script_config,
                )
                .await?;
            }
        }

        Ok(())
    }

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
        )
        .await?;
        script_config.sender_nonce = nonce;

        let BuildOutput { contract, highlevel_known_contracts, predeploy_libraries, .. } = self
            .link(
                project,
                default_known_contracts,
                script_config.config.parsed_libraries()?,
                new_sender,
                nonce,
            )?;

        first_run_result.transactions =
            Some(self.create_deploy_transactions(new_sender, nonce, &predeploy_libraries));

        let result =
            self.execute(script_config, contract, new_sender, &predeploy_libraries).await?;

        first_run_result.success &= result.success;
        first_run_result.gas = result.gas;
        first_run_result.logs = result.logs;
        first_run_result.traces.extend(result.traces);
        first_run_result.debug = result.debug;
        first_run_result.labeled_addresses.extend(result.labeled_addresses);

        match (&mut first_run_result.transactions, result.transactions) {
            (Some(txs), Some(new_txs)) => {
                for tx in new_txs.iter() {
                    txs.push_back(TypedTransaction::Legacy(tx.clone().into()));
                }
            }
            (None, Some(new_txs)) => {
                first_run_result.transactions = Some(new_txs);
            }
            _ => {}
        }

        Ok(unwrap_contracts(&highlevel_known_contracts))
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
