//! Tempo on evm2, using Tempo's own evm2 integration (tempoxyz/tempo#7871).

use super::{
    EvmEnv, FoundryBlockExt, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork,
    FoundryTxResultExt, SendableExt, TransactionRequestFor, TxEnvelopeFor, TxPayload,
    placeholder_signature,
};
use crate::{
    constants::{CALLER, SYSTEM_PRECOMPILE_STUB, TEST_CONTRACT_ADDRESS},
    tempo::{PATH_USD_ADDRESS, initialize_tempo_test_genesis_inner},
};
use alloy_consensus::{SignableTransaction, transaction::Recovered};
use alloy_eips::Typed2718;
use alloy_network::{
    AnyRpcTransaction, AnyTxEnvelope, NetworkTransactionBuilder, TransactionBuilder,
};
use alloy_primitives::{Address, Bytes};
use evm2::{Evm, TxResult, Version, evm::precompile::PrecompileProvider, registry::TxRegistry};
use foundry_evm_networks::TEMPO_PRECOMPILE_ADDRESSES;
use std::sync::Arc;
use tempo_alloy::{TempoNetwork, primitives::TEMPO_TX_TYPE_ID};
use tempo_evm::{
    TempoBlockExt, TempoEvmExt, TempoEvmFactory, TempoEvmTypes, TempoTxEnv, TempoTxResultExt,
    tempo_execution_config, tempo_tx_registry,
};
use tempo_hardfork::TempoHardfork;
use tempo_precompiles::{
    TempoPrecompiles,
    storage::{StorageActions, StorageCtx},
};

/// Tempo EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct TempoEvmNetwork;

impl FoundryBlockExt for TempoBlockExt {}

/// Tempo envelope of Foundry's RPC types.
type TempoNetworkTxEnvelope = TxEnvelopeFor<TempoEvmNetwork>;

impl FoundryEvmExt<TempoTxEnv> for TempoEvmExt {}

// SAFETY: `detached` allocates fresh transaction-local state instead of cloning or reading the
// `Rc` handles; every other field is `Send + Sync`.
unsafe impl SendableExt for TempoEvmExt {
    fn detached(&self) -> Self {
        Self {
            fee_manager: Arc::clone(&self.fee_manager),
            actions: if self.actions.is_enabled() {
                StorageActions::enabled()
            } else {
                StorageActions::disabled()
            },
            skip_valid_after_check: self.skip_valid_after_check,
            skip_liquidity_check: self.skip_liquidity_check,
            resolved_fee_token: self.resolved_fee_token,
            key_expiry: self.key_expiry,
            ..Self::default()
        }
    }
}

impl FoundryTxResultExt for TempoTxResultExt {}

impl FoundryEvmNetwork for TempoEvmNetwork {
    type Network = TempoNetwork;
    type EvmFactory = TempoEvmFactory;

    fn evm_tx(
        envelope: TempoNetworkTxEnvelope,
        sender: Address,
        payload: TxPayload,
    ) -> eyre::Result<Recovered<TempoTxEnv>> {
        tempo_tx(envelope, sender, payload, None)
    }

    fn tx_from_request(
        request: TransactionRequestFor<Self>,
    ) -> eyre::Result<Recovered<TempoTxEnv>> {
        let sender =
            request.from().ok_or_else(|| eyre::eyre!("transaction request has no sender"))?;
        // The access key only prices simulations; the signed envelope carries its own key.
        let key_id = request.key_id;
        let unsigned =
            request.build_unsigned().map_err(|err| eyre::eyre!(err.error.to_string()))?;
        let envelope = TempoNetworkTxEnvelope::from(unsigned.into_signed(placeholder_signature()));
        tempo_tx(envelope, sender, TxPayload::Simulated, key_id)
    }

    fn envelope_from_any_rpc(tx: &AnyRpcTransaction) -> eyre::Result<TempoNetworkTxEnvelope> {
        match &*tx.inner.inner {
            AnyTxEnvelope::Ethereum(envelope) => TempoNetworkTxEnvelope::try_from(envelope.clone())
                .map_err(|_| super::unknown_transaction_type(tx, "TempoTxEnv")),
            AnyTxEnvelope::Unknown(unknown) if unknown.ty() == TEMPO_TX_TYPE_ID => {
                // Tempo AA transactions only deserialize through Tempo's own envelope.
                let rpc_tx = serde_json::from_value::<
                    alloy_rpc_types::Transaction<TempoNetworkTxEnvelope>,
                >(serde_json::to_value(tx)?)
                .map_err(|err| eyre::eyre!("cannot decode Tempo AA transaction: {err}"))?;
                Ok(rpc_tx.inner.into_inner())
            }
            AnyTxEnvelope::Unknown(_) => Err(super::unknown_transaction_type(tx, "TempoTxEnv")),
        }
    }
}

/// Converts a Tempo network envelope into Tempo's evm2 transaction.
fn tempo_tx(
    envelope: TempoNetworkTxEnvelope,
    sender: Address,
    payload: TxPayload,
    key_id: Option<Address>,
) -> eyre::Result<Recovered<TempoTxEnv>> {
    let mut tx = TempoTxEnv::from(Recovered::new_unchecked(envelope, sender));
    if payload.is_simulated() {
        let id = tx.unique_tx_identifier();
        tx = tx.with_simulation_overrides(id, None, key_id);
    }
    Ok(Recovered::new_unchecked(tx, sender))
}

impl FoundryEvmFactory for TempoEvmFactory {
    type Types = TempoEvmTypes;

    fn version(&self, spec: TempoHardfork, chain_id: u64) -> Version {
        let mut version = *tempo_execution_config(spec, chain_id).version();
        // Match Tempo's node: the execution config keeps Ethereum's cap, and Tempo raises it.
        version.tx_gas_limit_cap = spec.tx_gas_limit_cap().unwrap_or(u64::MAX);
        version
    }

    fn tx_registry(
        &self,
        spec: TempoHardfork,
    ) -> TxRegistry<TempoEvmTypes, TxResult<TempoEvmTypes>> {
        tempo_tx_registry(spec.into())
    }

    fn precompiles(
        &self,
        env: &EvmEnv<TempoEvmTypes>,
        ext: &TempoEvmExt,
    ) -> Box<dyn PrecompileProvider<TempoEvmTypes>> {
        Box::new(TempoPrecompiles::new(
            env.spec,
            ext.actions.clone(),
            ext.non_creditable_slots.clone(),
        ))
    }

    fn initialize_evm(&self, evm: &mut Evm<'_, TempoEvmTypes>, is_forked: bool) {
        let result = StorageCtx::enter_evm(evm, || {
            if is_forked {
                let mut ctx = StorageCtx;
                // Precompiles have no on-chain bytecode; seed sentinels so forks do not refetch
                // them. Failed loads are retried, and surface, on later access.
                for address in TEMPO_PRECOMPILE_ADDRESSES.iter().chain([&PATH_USD_ADDRESS]) {
                    let _ = ctx.set_code(*address, Bytes::from_static(SYSTEM_PRECOMPILE_STUB));
                }
                Ok(())
            } else {
                initialize_tempo_test_genesis_inner(TEST_CONTRACT_ADDRESS, CALLER)
            }
        });
        result.expect("tempo genesis initialization failed");
        evm.state_mut().commit_transaction();
    }
}
