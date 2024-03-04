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
    pub async fn resume(self) -> Result<BundledState> {
        let Self {
            args,
            script_config,
            script_wallets,
            mut build_data,
            execution_data,
            execution_result: _,
            execution_artifacts,
        } = self;

        if execution_artifacts.rpc_data.missing_rpc {
            eyre::bail!("Missing `--fork-url` field.")
        }

        let sequence = match execution_artifacts.rpc_data.total_rpcs.len() {
            2.. => ScriptSequenceKind::Multi(MultiChainSequence::load(
                &script_config.config,
                &args.sig,
                &build_data.build_data.target,
            )?),
            1 => {
                let fork_url = execution_artifacts.rpc_data.total_rpcs.iter().next().unwrap();

                let provider = Arc::new(try_get_http_provider(fork_url)?);
                let chain = provider.get_chainid().await?.as_u64();

                let seq = match ScriptSequence::load(
                    &script_config.config,
                    &args.sig,
                    &build_data.build_data.target,
                    chain,
                    args.broadcast,
                ) {
                    Ok(seq) => seq,
                    // If the script was simulated, but there was no attempt to broadcast yet,
                    // try to read the script sequence from the `dry-run/` folder
                    Err(_) if args.broadcast => ScriptSequence::load(
                        &script_config.config,
                        &args.sig,
                        &build_data.build_data.target,
                        chain,
                        false,
                    )?,
                    Err(err) => {
                        eyre::bail!(err.wrap_err("If you were trying to resume or verify a multi chain deployment, add `--multi` to your command invocation."))
                    }
                };

                // We might have predeployed libraries from the broadcasting, so we need to
                // relink the contracts with them, since their mapping is
                // not included in the solc cache files.
                build_data =
                    build_data.build_data.link_with_libraries(Libraries::parse(&seq.libraries)?)?;

                ScriptSequenceKind::Single(seq)
            }
            0 => eyre::bail!("No RPC URLs"),
        };

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
}
