use std::sync::Arc;

use ethers_providers::Middleware;
use eyre::Result;
use foundry_common::provider::ethers::try_get_http_provider;
use foundry_compilers::artifacts::Libraries;

use super::{
    multi_sequence::MultiChainSequence,
    sequence::{ScriptSequence, ScriptSequenceKind},
    states::{BundledState, PreSimulationState},
};

impl PreSimulationState {
    /// Tries loading the resumed state from the cache files, skipping simulation stage.
    pub async fn resume(mut self) -> Result<BundledState> {
        if self.execution_artifacts.rpc_data.missing_rpc {
            eyre::bail!("Missing `--fork-url` field.")
        }

        let chain = match self.execution_artifacts.rpc_data.total_rpcs.len() {
            2.. => None,
            1 => {
                let fork_url = self.execution_artifacts.rpc_data.total_rpcs.iter().next().unwrap();

                let provider = Arc::new(try_get_http_provider(fork_url)?);
                Some(provider.get_chainid().await?.as_u64())
            }
            0 => eyre::bail!("No RPC URLs"),
        };
        
        let sequence = match self.try_load_sequence(chain, false) {
            Ok(sequence) => sequence,
            Err(_) => {
                // If the script was simulated, but there was no attempt to broadcast yet,
                // try to read the script sequence from the `dry-run/` folder
                let mut sequence = self.try_load_sequence(chain, true)?;

                // If sequence was in /dry-run, Update its paths so it is not saved into /dry-run
                // this time as we are about to broadcast it.
                sequence.update_paths_to_broadcasted(
                    &self.script_config.config,
                    &self.args.sig,
                    &self.build_data.build_data.target,
                )?;

                sequence.save(true, true)?;
                sequence
            }
        };

        match sequence {
            ScriptSequenceKind::Single(ref seq) => {
                // We might have predeployed libraries from the broadcasting, so we need to
                // relink the contracts with them, since their mapping is not included in the solc
                // cache files.
                self.build_data = self
                    .build_data
                    .build_data
                    .link_with_libraries(Libraries::parse(&seq.libraries)?)?;
            }
            // Library linking is not supported for multi-chain sequences
            ScriptSequenceKind::Multi(_) => {}
        }

        let Self {
            args,
            script_config,
            script_wallets,
            build_data,
            execution_data,
            execution_result: _,
            execution_artifacts,
        } = self;

        Ok(BundledState {
            args,
            script_config,
            script_wallets,
            build_data,
            execution_data,
            execution_artifacts,
            sequence,
        })
    }

    fn try_load_sequence(&self, chain: Option<u64>, dry_run: bool) -> Result<ScriptSequenceKind> {
        if let Some(chain) = chain {
            let sequence = ScriptSequence::load(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.build_data.target,
                chain,
                dry_run,
            )?;
            Ok(ScriptSequenceKind::Single(sequence))
        } else {
            let sequence = MultiChainSequence::load(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.build_data.target,
                dry_run,
            )?;
            Ok(ScriptSequenceKind::Multi(sequence))
        }
    }
}
