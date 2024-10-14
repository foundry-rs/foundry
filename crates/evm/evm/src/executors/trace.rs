use crate::executors::{Executor, ExecutorBuilder};
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{utils::evm_spec_id, Chain, Config};
use foundry_evm_core::{backend::Backend, fork::CreateFork, opts::EvmOpts};
use foundry_evm_traces::{InternalTraceMode, TraceMode};
use revm::primitives::{Env, SpecId};
use std::ops::{Deref, DerefMut};

/// A default executor with tracing enabled
pub struct TracingExecutor {
    executor: Executor,
}

impl TracingExecutor {
    pub fn new(
        env: revm::primitives::Env,
        fork: Option<CreateFork>,
        version: Option<EvmVersion>,
        debug: bool,
        decode_internal: bool,
        alphanet: bool,
    ) -> Self {
        let db = Backend::spawn(fork);
        let trace_mode =
            TraceMode::Call.with_debug(debug).with_decode_internal(if decode_internal {
                InternalTraceMode::Full
            } else {
                InternalTraceMode::None
            });
        Self {
            // configures a bare version of the evm executor: no cheatcode inspector is enabled,
            // tracing will be enabled only for the targeted transaction
            executor: ExecutorBuilder::new()
                .inspectors(|stack| stack.trace_mode(trace_mode).alphanet(alphanet))
                .spec(evm_spec_id(&version.unwrap_or_default(), alphanet))
                .build(env, db),
        }
    }

    /// Returns the spec id of the executor
    pub fn spec_id(&self) -> SpecId {
        self.executor.spec_id()
    }

    /// uses the fork block number from the config
    pub async fn get_fork_material(
        config: &Config,
        mut evm_opts: EvmOpts,
    ) -> eyre::Result<(Env, Option<CreateFork>, Option<Chain>, bool)> {
        evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());
        evm_opts.fork_block_number = config.fork_block_number;

        let env = evm_opts.evm_env().await?;

        let fork = evm_opts.get_fork(config, env.clone());

        Ok((env, fork, evm_opts.get_remote_chain_id().await, evm_opts.alphanet))
    }
}

impl Deref for TracingExecutor {
    type Target = Executor;

    fn deref(&self) -> &Self::Target {
        &self.executor
    }
}

impl DerefMut for TracingExecutor {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.executor
    }
}
