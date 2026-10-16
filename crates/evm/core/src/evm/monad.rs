//! Monad on evm2, using `monad-evm2`.

use super::{
    EvmEnv, FoundryBlockExt, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork,
    FoundryTxResultExt, SendableExt, TxPayload, eth_envelope_from_any_rpc, eth_tx,
};
use alloy_consensus::transaction::Recovered;
use alloy_network::{AnyRpcTransaction, Ethereum};
use alloy_primitives::{Address, map::HashSet};
use evm2::{
    ExecutionConfig, TxResult, Version, ethereum::TxEnvelope, evm::precompile::PrecompileProvider,
    registry::TxRegistry,
};
use monad_evm2::{
    MonadBlockExt, MonadChainContext, MonadEvmExt, MonadEvmFactory, MonadEvmTypes, MonadHardfork,
    MonadPrecompiles, MonadTxResultExt, monad_memory_limit, monad_tx_registry, monad_version,
};

/// Monad EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct MonadEvmNetwork;

impl FoundryBlockExt for MonadBlockExt {}

// SAFETY: `MonadEvmExt` is `Send + Sync`, as asserted below, so clones share only thread-safe
// state.
unsafe impl SendableExt for MonadEvmExt {}

const _: fn() = || {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MonadEvmExt>();
};

impl FoundryEvmExt<TxEnvelope> for MonadEvmExt {
    fn for_transaction(&mut self, tx: &Recovered<TxEnvelope>) {
        // A synthetic transaction is alone in its block: its sender and EIP-7702 authorities are
        // the only ones the reserve-balance rules may see.
        self.for_block(&[], &[], std::slice::from_ref(tx), 0);
    }

    fn for_block(
        &mut self,
        grandparent: &[Recovered<TxEnvelope>],
        parent: &[Recovered<TxEnvelope>],
        current: &[Recovered<TxEnvelope>],
        index: usize,
    ) {
        self.chain = MonadChainContext {
            grandparent_senders_and_authorities: block_participants(grandparent),
            parent_senders_and_authorities: block_participants(parent),
            current_block_senders: current.iter().map(|tx| tx.signer()).collect(),
            current_block_authorities: current.iter().map(authorities).collect(),
            current_tx_index: index,
            ..Default::default()
        };
    }

    fn replace_while_executing(&mut self, ext: Self) {
        // The pooled memory offset counts the memory of the suspended frames, which each release
        // their share when they return.
        let memory_pool_offset = self.memory_pool_offset;
        *self = ext;
        self.memory_pool_offset = memory_pool_offset;
    }
}

/// Returns the EIP-7702 authorities of `tx`.
fn authorities(tx: &Recovered<TxEnvelope>) -> Vec<Address> {
    tx.as_eip7702()
        .map(|tx| tx.authorization_list.iter().filter_map(|auth| auth.authority()).collect())
        .unwrap_or_default()
}

/// Returns every sender and EIP-7702 authority of a block's transactions.
fn block_participants(transactions: &[Recovered<TxEnvelope>]) -> HashSet<Address> {
    transactions.iter().flat_map(|tx| std::iter::once(tx.signer()).chain(authorities(tx))).collect()
}

impl FoundryTxResultExt for MonadTxResultExt {}

impl FoundryEvmNetwork for MonadEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = MonadEvmFactory;

    fn evm_tx(
        envelope: alloy_consensus::TxEnvelope,
        sender: Address,
        _payload: TxPayload,
    ) -> eyre::Result<Recovered<TxEnvelope>> {
        Ok(Recovered::new_unchecked(eth_tx(envelope), sender))
    }

    fn envelope_from_any_rpc(tx: &AnyRpcTransaction) -> eyre::Result<alloy_consensus::TxEnvelope> {
        eth_envelope_from_any_rpc(tx, "Monad")
    }
}

impl FoundryEvmFactory for MonadEvmFactory {
    type Types = MonadEvmTypes;

    fn version(&self, spec: MonadHardfork, chain_id: u64) -> Version {
        monad_version(spec, chain_id)
    }

    fn execution_config(
        &self,
        spec: MonadHardfork,
        mut version: Version,
    ) -> ExecutionConfig<MonadEvmTypes> {
        // MIP-3 caps the pooled memory limit from MonadNine on, including a larger configured one.
        version.memory_limit = monad_memory_limit(spec, version.memory_limit);
        ExecutionConfig::for_spec_and_version(spec, version)
    }

    fn tx_registry(
        &self,
        spec: MonadHardfork,
    ) -> TxRegistry<MonadEvmTypes, TxResult<MonadEvmTypes>> {
        monad_tx_registry(spec)
    }

    fn precompiles(
        &self,
        env: &EvmEnv<MonadEvmTypes>,
        _ext: &MonadEvmExt,
    ) -> Box<dyn PrecompileProvider<MonadEvmTypes>> {
        Box::new(MonadPrecompiles::new(env.spec))
    }
}
