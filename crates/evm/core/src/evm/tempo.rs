//! Tempo on evm2, using Tempo's own evm2 integration (tempoxyz/tempo#7871).

use super::{
    EvmEnv, FoundryBlockExt, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork,
    FoundryTxResultExt, TransactionRequestFor, TxEnvelopeFor, TxPayload, placeholder_signature,
};
use crate::{
    constants::{CALLER, SYSTEM_PRECOMPILE_STUB, TEST_CONTRACT_ADDRESS},
    tempo::{PATH_USD_ADDRESS, initialize_tempo_test_genesis_inner},
};
use alloy_consensus::{SignableTransaction, transaction::Recovered};
use alloy_eips::{Decodable2718, Encodable2718, Typed2718};
use alloy_network::{
    AnyRpcTransaction, AnyTxEnvelope, NetworkTransactionBuilder, TransactionBuilder,
};
use alloy_primitives::{Address, Bytes};
use evm2::{Evm, TxResult, Version, evm::precompile::PrecompileProvider, registry::TxRegistry};
use foundry_evm_networks::TEMPO_PRECOMPILE_ADDRESSES;
use tempo_alloy::{TempoNetwork, primitives::TEMPO_TX_TYPE_ID};
use tempo_evm2::{
    TempoBlockExt, TempoEvmExt, TempoEvmFactory, TempoEvmTypes, TempoTxEnv, TempoTxResultExt,
    tempo_execution_config, tempo_tx_registry,
};
use tempo_hardfork_evm2::TempoHardfork;
use tempo_precompiles_evm2::{TempoPrecompiles, storage::StorageCtx};
use tempo_primitives_evm2::TempoTxEnvelope;

/// Tempo EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct TempoEvmNetwork;

impl FoundryBlockExt for TempoBlockExt {
    fn timestamp_millis_part(&self) -> u64 {
        self.timestamp_millis_part
    }

    fn set_timestamp_millis_part(&mut self, millis: u64) {
        self.timestamp_millis_part = millis;
    }
}

/// Tempo envelope of Foundry's RPC types.
type TempoNetworkTxEnvelope = TxEnvelopeFor<TempoEvmNetwork>;

impl FoundryEvmExt<TempoTxEnv> for TempoEvmExt {}

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
                .map_err(|_| super::unknown_transaction_type(tx, "Tempo")),
            AnyTxEnvelope::Unknown(unknown) if unknown.ty() == TEMPO_TX_TYPE_ID => {
                // Tempo AA transactions only deserialize through Tempo's own envelope.
                let rpc_tx = serde_json::from_value::<
                    alloy_rpc_types::Transaction<TempoNetworkTxEnvelope>,
                >(serde_json::to_value(tx)?)
                .map_err(|err| eyre::eyre!("cannot decode Tempo AA transaction: {err}"))?;
                Ok(rpc_tx.inner.into_inner())
            }
            AnyTxEnvelope::Unknown(_) => Err(super::unknown_transaction_type(tx, "Tempo")),
        }
    }
}

/// Converts a Tempo network envelope into Tempo's evm2 transaction.
///
/// Foundry's RPC types and Tempo's evm2 integration are pinned to different Tempo revisions, so the
/// envelope crosses over as its exact EIP-2718 encoding.
fn tempo_tx(
    envelope: TempoNetworkTxEnvelope,
    sender: Address,
    payload: TxPayload,
    key_id: Option<Address>,
) -> eyre::Result<Recovered<TempoTxEnv>> {
    let encoded = envelope.encoded_2718();
    let envelope = TempoTxEnvelope::decode_2718_exact(&encoded)?;
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
        *tempo_execution_config(spec, chain_id).version()
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
