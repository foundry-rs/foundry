use crate::{
    executor::{fork::CreateFork, opts::EvmOpts, Backend, Executor, ExecutorBuilder, OnLog},
    utils::evm_spec,
};
use ethers::solc::EvmVersion;
use foundry_config::Config;
use revm::primitives::Env;
use std::ops::{Deref, DerefMut};

pub type TracingExecutor = GenTracingExecutor<()>;

/// A default executor with tracing enabled
pub struct GenTracingExecutor<ONLOG: OnLog> {
    executor: Executor<ONLOG>,
}

impl<ONLOG: OnLog> GenTracingExecutor<ONLOG> {
    pub async fn new(
        env: revm::primitives::Env,
        fork: Option<CreateFork>,
        version: Option<EvmVersion>,
        debug: bool,
    ) -> Self {
        let db = Backend::spawn(fork).await;
        Self {
            // configures a bare version of the evm executor: no cheatcode inspector is enabled,
            // tracing will be enabled only for the targeted transaction
            executor: ExecutorBuilder::new()
                .inspectors(|stack| stack.trace(true).debug(debug))
                .spec(evm_spec(version.unwrap_or_default()))
                .build(env, db),
        }
    }

    /// uses the fork block number from the config
    pub async fn get_fork_material(
        config: &Config,
        mut evm_opts: EvmOpts,
    ) -> eyre::Result<(Env, Option<CreateFork>, Option<ethers::types::Chain>)> {
        evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());
        evm_opts.fork_block_number = config.fork_block_number;

        let env = evm_opts.evm_env().await?;

        let fork = evm_opts.get_fork(config, env.clone());

        Ok((env, fork, evm_opts.get_remote_chain_id()))
    }
}

impl<ONLOG: OnLog> Deref for GenTracingExecutor<ONLOG> {
    type Target = Executor<ONLOG>;

    fn deref(&self) -> &Self::Target {
        &self.executor
    }
}

impl<ONLOG: OnLog> DerefMut for GenTracingExecutor<ONLOG> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.executor
    }
}
