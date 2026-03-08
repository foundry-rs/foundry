use std::sync::Arc;

use crate::inspectors::InspectorStack;
use eyre::WrapErr;
use foundry_cheatcodes::Cheatcodes;
use foundry_evm_core::{
    Env, FoundryInspectorDowncastExt, FoundryInspectorExt, InspectorExt,
    backend::DatabaseExt,
    evm::{FoundryEvmFactory, new_evm_with_inspector},
};
use revm::{context_interface::result::ResultAndState, inspector::NoOpInspector};

/// Ethereum EVM factory — the default for Ethereum-based chains.
///
/// Downcasts the network-agnostic `&mut dyn FoundryInspectorExt` to a concrete
/// inspector type that implements `Inspector<EthEvmContext<…>>`.
///
/// Supports [`InspectorStack`], [`Cheatcodes`], and [`NoOpInspector`].
#[derive(Debug, Clone, Copy, Default)]
pub struct EthFoundryEvmFactory;

impl EthFoundryEvmFactory {
    /// Runs a transaction with a concrete inspector, setting the journaled state depth.
    fn run_with_depth<I: InspectorExt>(
        db: &mut dyn DatabaseExt,
        env: Env,
        inspector: &mut I,
        depth: usize,
    ) -> eyre::Result<ResultAndState> {
        let tx = env.tx.clone();
        let mut evm = new_evm_with_inspector(db, env, inspector);
        evm.journaled_state.depth = depth;
        alloy_evm::Evm::transact(&mut evm, tx).wrap_err("EVM error")
    }
}

impl FoundryEvmFactory for EthFoundryEvmFactory {
    fn inspect(
        &self,
        db: &mut dyn DatabaseExt,
        env: &mut Env,
        inspector: &mut dyn FoundryInspectorExt,
    ) -> eyre::Result<ResultAndState> {
        let stack = inspector
            .downcast_mut::<InspectorStack>()
            .ok_or_else(|| eyre::eyre!("EthFoundryEvmFactory::inspect requires InspectorStack"))?;
        let tx = env.tx.clone();
        let mut evm = new_evm_with_inspector(db, env.to_owned(), stack);
        let res = alloy_evm::Evm::transact(&mut evm, tx).wrap_err("EVM error")?;
        *env = Env::from(evm.cfg.clone(), evm.block.clone(), evm.tx.clone());
        Ok(res)
    }

    fn transact_with_depth(
        &self,
        db: &mut dyn DatabaseExt,
        env: Env,
        inspector: &mut dyn FoundryInspectorExt,
        depth: usize,
    ) -> eyre::Result<ResultAndState> {
        if let Some(stack) = inspector.downcast_mut::<InspectorStack>() {
            Self::run_with_depth(db, env, stack, depth)
        } else if let Some(cheats) = inspector.downcast_mut::<Cheatcodes>() {
            Self::run_with_depth(db, env, cheats, depth)
        } else if let Some(noop) = inspector.downcast_mut::<NoOpInspector>() {
            Self::run_with_depth(db, env, noop, depth)
        } else {
            eyre::bail!(
                "EthFoundryEvmFactory::transact_with_depth: unsupported inspector type \
                 (expected InspectorStack, Cheatcodes, or NoOpInspector)"
            )
        }
    }
}

/// Returns the default (Ethereum) EVM factory.
pub fn default_evm_factory() -> Arc<dyn FoundryEvmFactory> {
    Arc::new(EthFoundryEvmFactory)
}
