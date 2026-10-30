//! Monad on evm2, using `monad-evm2`.

use super::{
    EvmEnv, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork, FoundryTxResultExt, SendableExt,
    TxPayload, eth_envelope_from_any_rpc, eth_tx, with_eip7702_delegations,
};
use alloy_consensus::{TxLegacy, transaction::Recovered};
use alloy_network::{AnyRpcTransaction, Ethereum};
use alloy_primitives::{
    Address, KECCAK256_EMPTY, U256,
    map::{AddressMap, HashSet},
};
use evm2::{
    Evm, ExecutionConfig, TxResult, Version,
    ethereum::TxEnvelope,
    evm::{AccountInfo, precompile::PrecompileProvider},
    registry::TxRegistry,
};
use monad_evm2::{
    MonadChainContext, MonadEvmExt, MonadEvmFactory, MonadEvmTypes, MonadHardfork,
    MonadPrecompiles, MonadTxResultExt, monad_memory_limit, monad_tx_registry, monad_version,
    protocol_system_call,
    reserve_balance::{reserve_account, sync_reserve_balance},
    staking::constants::SYSTEM_ADDRESS,
};

/// Monad EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct MonadEvmNetwork;

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
        // Protocol replay is opted into per trusted transaction.
        self.protocol_replay = false;
    }

    fn replace_while_executing(&mut self, ext: Self) {
        // The pooled memory offset counts the memory of the suspended frames, which each release
        // their share when they return.
        let memory_pool_offset = self.memory_pool_offset;
        *self = ext;
        self.memory_pool_offset = memory_pool_offset;
    }

    fn is_protocol_transaction(tx: &Recovered<TxEnvelope>) -> bool {
        matches!(protocol_system_call(tx), Ok(Some(_)))
    }

    fn set_protocol_replay(&mut self, enabled: bool) {
        self.protocol_replay = enabled;
    }

    fn set_preserve_transaction_state(&mut self, preserve: bool) {
        self.set_preserve_reserve_balance_tracker(preserve);
    }

    fn for_next_transaction(&mut self, tx: &Recovered<TxEnvelope>) {
        // The retained cursor is the current block's prefix before `current_tx_index`; anything
        // after it is a previous synthetic transaction's own entry, never canonical history.
        let chain = &mut self.chain;
        truncate_to_cursor(chain);
        chain.current_block_senders.push(tx.signer());
        chain.current_block_authorities.push(authorities(tx));
        // Protocol replay is opted into per trusted transaction.
        self.protocol_replay = false;
    }

    fn adopt_cursor(&mut self, cursor: &Self) {
        let running = running_entry(&self.chain);
        let mut chain = cursor.chain.clone();
        truncate_to_cursor(&mut chain);
        if let Some((sender, authorities)) = running {
            chain.current_block_senders.push(sender);
            chain.current_block_authorities.push(authorities);
        }
        self.chain = chain;
    }

    fn resume_context(&mut self, ext: Self) {
        // A context only contributes its block cursor; the reserve-balance tracker, journal sync
        // and memory pool belong to the running transaction.
        self.adopt_cursor(&ext);
    }
}

/// Drops entries after the retained cursor of the current block.
fn truncate_to_cursor(chain: &mut MonadChainContext) {
    chain.current_block_senders.truncate(chain.current_tx_index);
    chain.current_block_authorities.truncate(chain.current_tx_index);
}

/// Returns the running transaction's own sender and authorities, if it is positioned.
fn running_entry(chain: &MonadChainContext) -> Option<(Address, Vec<Address>)> {
    let sender = *chain.current_block_senders.get(chain.current_tx_index)?;
    let authorities =
        chain.current_block_authorities.get(chain.current_tx_index).cloned().unwrap_or_default();
    Some((sender, authorities))
}

/// Returns whether `tx`, attributed to the Monad system sender, is a canonical protocol system
/// transaction, whatever its signature recovers to.
pub fn is_protocol_system_legacy(tx: &TxLegacy) -> bool {
    let tx = Recovered::new_unchecked(TxEnvelope::Legacy(tx.clone()), SYSTEM_ADDRESS);
    matches!(protocol_system_call(&tx), Ok(Some(_)))
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
        with_eip7702_delegations(monad_version(spec, chain_id))
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

    fn sync_transaction(&self, evm: &mut Evm<'_, MonadEvmTypes>) {
        // A database error surfaces again when execution next reads the account.
        let _ = sync_reserve_balance(evm);
    }

    fn refresh_transaction(&self, evm: &mut Evm<'_, MonadEvmTypes>) {
        self.refresh_switched_transaction(evm, &AddressMap::default());
    }

    fn refresh_switched_transaction(
        &self,
        evm: &mut Evm<'_, MonadEvmTypes>,
        original: &AddressMap<Option<AccountInfo>>,
    ) {
        // Foundry's writes, such as `deal` rebasing earlier journal entries, are not reserve
        // balance events.
        let journal_len = evm.state().journal().len();
        evm.ext_mut().reserve_balance_sync.begin(journal_len);
        // Thresholds derive from the transaction-start balance, which a fork switch accepted into
        // the persistent accounts' new original.
        let tracked = evm.ext().reserve_balance.tracked_addresses().copied().collect::<Vec<_>>();
        let mut views = Vec::with_capacity(tracked.len());
        for address in tracked {
            // A database error surfaces again when execution next reads the account.
            let Ok(mut view) = reserve_account(evm, address) else { continue };
            if let Some(info) = original.get(&address) {
                view.original_balance = info.as_ref().map_or(U256::ZERO, |info| info.balance);
                view.original_code_hash =
                    info.as_ref().map_or(KECCAK256_EMPTY, |info| info.code_hash);
            }
            views.push((address, view));
        }
        let ext = evm.ext_mut();
        let chain = ext.chain.clone();
        ext.reserve_balance.rebase(&chain, |address| {
            views.iter().find(|(tracked, _)| *tracked == address).map(|(_, view)| *view)
        });
    }
}
