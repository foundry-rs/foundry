//! Base-specific transaction conversions.

use alloy_primitives::{Address, Bytes};
use alloy_rpc_types::TransactionRequest;
use base_common_consensus::{BASE_TX_TYPE_ID as EIP8130_TRANSACTION_TYPE, TxBase};
use base_common_rpc_types::{BaseTransactionRequest, TxBaseRequestFields};
use serde::Serialize;

/// Projects the complete AA body into a simulation request without inventing a single call.
pub(super) fn simulation_request(
    tx: TxBase,
    from: Option<Address>,
    sender_auth: Option<Bytes>,
    payer_auth: Option<Bytes>,
) -> BaseTransactionRequest {
    let inner = TransactionRequest {
        transaction_type: Some(EIP8130_TRANSACTION_TYPE),
        chain_id: Some(tx.chain_id),
        from,
        nonce: Some(tx.nonce_sequence),
        gas: Some(tx.gas_limit),
        max_fee_per_gas: Some(tx.max_fee_per_gas),
        max_priority_fee_per_gas: Some(tx.max_priority_fee_per_gas),
        ..Default::default()
    };
    let fields = TxBaseRequestFields {
        nonce_key: Some(tx.nonce_key),
        account_changes: Some(tx.account_changes),
        calls: Some(tx.calls),
        valid_after: Some(tx.valid_after),
        valid_before: Some(tx.valid_before),
        metadata: Some(tx.metadata),
        sender: tx.sender,
        sender_auth,
        payer: tx.payer,
        payer_auth,
    };
    // The upstream request exposes no constructor for its private AA fields.
    #[derive(Serialize)]
    struct SimulationRequest {
        #[serde(flatten)]
        inner: TransactionRequest,
        #[serde(flatten)]
        fields: TxBaseRequestFields,
    }
    let value = serde_json::to_value(SimulationRequest { inner, fields })
        .expect("serializable Base simulation request");
    serde_json::from_value(value).expect("compatible Base simulation fields")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FoundryTransactionRequest, FoundryTxEnvelope, FoundryTypedTx};
    use alloy_network::NetworkTransactionBuilder;
    use alloy_primitives::U256;
    use base_common_consensus::{AccountChange, Call, Delegation, TxBaseSigned};

    #[test]
    fn eip8130_simulation_projection() {
        let body = TxBase {
            chain_id: 8453,
            sender: Some(Address::repeat_byte(1)),
            payer: Some(Address::repeat_byte(2)),
            nonce_key: U256::from(7),
            nonce_sequence: 9,
            gas_limit: 100_000,
            max_fee_per_gas: 20,
            max_priority_fee_per_gas: 3,
            valid_after: 100,
            valid_before: 200,
            calls: vec![vec![Call {
                to: Address::repeat_byte(3),
                value: U256::from(5),
                data: Bytes::from_static(b"call"),
            }]],
            account_changes: vec![AccountChange::Delegation(Delegation {
                target: Address::repeat_byte(4),
            })],
            metadata: Bytes::from_static(b"metadata"),
        };
        let signed = TxBaseSigned::new(
            body.clone(),
            Bytes::from_static(b"sender"),
            Bytes::from_static(b"payer"),
        );
        let envelope = FoundryTxEnvelope::Eip8130(signed.clone());
        for (request, from, sender_auth, payer_auth) in [
            (
                FoundryTransactionRequest::from(FoundryTypedTx::Eip8130(body.clone())),
                None,
                None,
                None,
            ),
            (
                FoundryTransactionRequest::from(envelope),
                body.sender,
                Some(signed.sender_auth().clone()),
                Some(signed.payer_auth().clone()),
            ),
        ] {
            assert!(!request.can_build());
            let base = request.as_base().unwrap();
            assert_eq!(
                base.as_base_tx(),
                Some(&TxBaseRequestFields {
                    nonce_key: Some(body.nonce_key),
                    account_changes: Some(body.account_changes.clone()),
                    calls: Some(body.calls.clone()),
                    valid_after: Some(body.valid_after),
                    valid_before: Some(body.valid_before),
                    metadata: Some(body.metadata.clone()),
                    sender: body.sender,
                    payer: body.payer,
                    sender_auth,
                    payer_auth,
                })
            );
            assert_eq!(
                base.as_ref(),
                &TransactionRequest {
                    transaction_type: Some(EIP8130_TRANSACTION_TYPE),
                    chain_id: Some(body.chain_id),
                    from,
                    nonce: Some(body.nonce_sequence),
                    gas: Some(body.gas_limit),
                    max_fee_per_gas: Some(body.max_fee_per_gas),
                    max_priority_fee_per_gas: Some(body.max_priority_fee_per_gas),
                    ..Default::default()
                }
            );
            let roundtrip: FoundryTransactionRequest =
                serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
            assert_eq!(request, roundtrip);
        }
    }
}
