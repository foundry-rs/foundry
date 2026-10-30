//! Base on evm2, using `base-evm2`.

use super::{
    EvmEnv, FoundryDatabase, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork,
    FoundryTxResultExt, SendableExt, TxPayload, with_eip7702_delegations,
};
use crate::constants::SYSTEM_PRECOMPILE_STUB;
use alloy_consensus::transaction::Recovered;
use alloy_network::AnyRpcTransaction;
use alloy_primitives::{Address, Bytes, KECCAK256_EMPTY};
use base_common_network::Base;
use base_evm2::{
    BaseEvmExt, BaseEvmFactory, BaseEvmTypes, BaseSpecId, BaseTxEnvelope, BaseTxResultExt,
    base_tx_registry, base_version, precompiles::activation_admin_address_for_upgrade_by_chain_id,
};
use evm2::{
    Evm, TxResult, Version, bytecode::Bytecode, evm::precompile::PrecompileProvider,
    registry::TxRegistry,
};
use foundry_evm_networks::{BASE_CODE_SENTINEL_ADDRESSES, is_base_precompile_active_at};

/// Base EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct BaseEvmNetwork;

// SAFETY: `BaseEvmExt` is `Send + Sync`, as asserted below, so clones share only thread-safe state.
unsafe impl SendableExt for BaseEvmExt {}

const _: fn() = || {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<BaseEvmExt>();
};

impl FoundryEvmExt<BaseTxEnvelope> for BaseEvmExt {
    fn replace_while_executing(&mut self, ext: Self) {
        // Only the block-level L1 info comes from `ext`; everything else, including the cached L1
        // cost, describes the running transaction, as when REVM kept this state in its context.
        let tx_l1_cost = self.l1_block_info.tx_l1_cost;
        self.l1_block_info = ext.l1_block_info;
        self.l1_block_info.tx_l1_cost = tx_l1_cost;
    }
}

impl FoundryTxResultExt for BaseTxResultExt {
    fn is_failed_deposit(&self) -> bool {
        self.failed_deposit
    }

    fn eip8130_phase_statuses(&self) -> Option<&[u8]> {
        self.eip8130_phase_statuses.as_deref()
    }
}

impl FoundryEvmNetwork for BaseEvmNetwork {
    type Network = Base;
    type EvmFactory = BaseEvmFactory;

    fn evm_tx(
        envelope: base_common_consensus::BaseTxEnvelope,
        sender: Address,
        payload: TxPayload,
    ) -> eyre::Result<Recovered<BaseTxEnvelope>> {
        let mut tx = BaseTxEnvelope::from(envelope);
        // The L1 data and operator fees are priced over the signed encoding; simulations of
        // unsigned requests price them at zero, like `eth_call`. An EIP-8130 simulation carries
        // placeholder authentication, so it runs without signature checks, nonce advancement or
        // fee settlement, as the sender's self-actor unless a caller supplies an actor hint.
        if payload.is_simulated() {
            tx.set_enveloped(Bytes::new());
            tx.set_eip8130_simulation(None);
        }
        Ok(tx.with_signer(sender))
    }

    fn envelope_from_any_rpc(
        tx: &AnyRpcTransaction,
    ) -> eyre::Result<base_common_consensus::BaseTxEnvelope> {
        if let Ok(envelope) = base_common_consensus::BaseTxEnvelope::try_from(tx.clone()) {
            return Ok(envelope);
        }
        // EIP-8130 envelopes only deserialize through Base's own RPC transaction type.
        let rpc_tx =
            serde_json::from_value::<base_common_rpc_types::Transaction>(serde_json::to_value(tx)?)
                .map_err(|err| super::unknown_transaction_type(tx, "Base").wrap_err(err))?;
        Ok(rpc_tx.inner.into_inner())
    }
}

impl FoundryEvmFactory for BaseEvmFactory {
    type Types = BaseEvmTypes;

    fn version(&self, spec: BaseSpecId, chain_id: u64) -> Version {
        with_eip7702_delegations(base_version(spec, chain_id))
    }

    fn tx_registry(&self, spec: BaseSpecId) -> TxRegistry<BaseEvmTypes, TxResult<BaseEvmTypes>> {
        base_tx_registry(spec)
    }

    fn precompiles(
        &self,
        env: &EvmEnv<BaseEvmTypes>,
        _ext: &BaseEvmExt,
    ) -> Box<dyn PrecompileProvider<BaseEvmTypes>> {
        Box::new(Self::precompiles(self, env.spec))
    }

    fn configure_for<D: FoundryDatabase + ?Sized>(
        &self,
        db: &D,
        env: &EvmEnv<BaseEvmTypes>,
    ) -> Self {
        // Nested EVMs share the fork database, so execution chain-ID overrides must not change the
        // source chain's activation registry admin.
        let chain_id = db.active_fork_source_chain_id().unwrap_or(env.chain_id());
        let admin = self.activation_admin_address().or_else(|| {
            activation_admin_address_for_upgrade_by_chain_id(chain_id, env.spec.upgrade())
        });
        self.clone().with_activation_admin_address(admin)
    }

    fn initialize_evm(&self, evm: &mut Evm<'_, BaseEvmTypes>, _is_forked: bool) {
        // Base plants a one-byte sentinel on code-less stateful precompiles, so Solidity's
        // `extcodesize` check on high-level calls passes. Existing code, including fork code, wins.
        let upgrade = evm.config_spec_id().upgrade();
        let sentinel = Bytecode::new_legacy(Bytes::from_static(SYSTEM_PRECOMPILE_STUB));
        for address in BASE_CODE_SENTINEL_ADDRESSES
            .iter()
            .filter(|address| is_base_precompile_active_at(**address, upgrade))
        {
            if let Ok(mut account) = evm.state_mut().account(address)
                && account.code_hash() == KECCAK256_EMPTY
            {
                account.set_code_slow(sentinel.clone());
            }
        }
        evm.state_mut().commit_transaction();
    }
}
