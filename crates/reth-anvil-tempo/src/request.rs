//! Adapters for Tempo's RPC requests and transaction-pool keys.

use crate::Tempo;
use alloy_consensus::Transaction;
use alloy_primitives::{Address, U256};
use alloy_rpc_types_eth::state::StateOverride;
use jsonrpsee::core::RpcResult;
use reth_anvil::{
    api::{CallBatch, NonceLane},
    txpool::TxPoolKey,
};
use tempo_alloy::rpc::TempoTransactionRequest;
use tempo_precompiles::{NONCE_PRECOMPILE_ADDRESS, nonce::NonceManager};
use tempo_primitives::{TempoTxEnvelope, transaction::TEMPO_EXPIRING_NONCE_KEY};

impl CallBatch<Tempo> for TempoTransactionRequest {
    fn validate_type(&self) -> RpcResult<()> {
        // Tempo's native builder and converter validate its transaction variants.
        Ok(())
    }

    fn call_state_overrides(
        &self,
        state_overrides: Option<StateOverride>,
    ) -> Option<StateOverride> {
        if let Some(nonce) = self.as_ref().nonce
            && let Some(from) = self.as_ref().from
        {
            let mut overrides = state_overrides.unwrap_or_default();
            match CallBatch::<Tempo>::nonce_lane(self, from) {
                NonceLane::Account => {
                    let account = overrides.entry(from).or_default();
                    account.nonce.get_or_insert(nonce);
                }
                // An expiring nonce has no lane state.
                NonceLane::Expiring => {}
                NonceLane::Storage(address, slot) => {
                    let account = overrides.entry(address).or_default();
                    account
                        .state_diff
                        .get_or_insert_default()
                        .entry(slot.into())
                        .or_insert(U256::from(nonce).into());
                }
            }
            return Some(overrides);
        }
        state_overrides
    }

    fn has_calls(&self) -> bool {
        !self.calls.is_empty()
    }

    fn signs_gas(&self) -> bool {
        self.fee_payer_signature.is_some()
    }

    fn nonce_lane(&self, from: Address) -> NonceLane {
        match self.nonce_key.filter(|key| !key.is_zero()) {
            None => NonceLane::Account,
            Some(TEMPO_EXPIRING_NONCE_KEY) => NonceLane::Expiring,
            Some(key) => NonceLane::Storage(
                NONCE_PRECOMPILE_ADDRESS,
                NonceManager::new().nonces[from][key].slot(),
            ),
        }
    }
}

impl TxPoolKey<Tempo> for TempoTxEnvelope {
    fn txpool_key(&self) -> String {
        match self.as_aa() {
            Some(tx) if !tx.tx().nonce_key.is_zero() => {
                format!("{}:{}", tx.tx().nonce_key, tx.tx().nonce)
            }
            _ => self.nonce().to_string(),
        }
    }
}
