//! Tempo T5 payment-lane classification shared by anvil and cast.

use alloy_eips::eip2718::Decodable2718;
use serde::{Deserialize, Serialize};
use tempo_primitives::TempoTxEnvelope;

/// Classifies a raw EIP-2718 encoded transaction with Tempo's T5 payment-lane classifier.
///
/// Bytes that do not decode as a Tempo envelope (e.g. EIP-4844 or Optimism deposit
/// transactions) classify as [`PaymentLaneReason::UnsupportedTransactionType`]; callers that
/// need to reject invalid transactions should validate the bytes beforehand.
pub fn classify_payment_lane(mut raw: &[u8]) -> PaymentLaneClassification {
    let Ok(tx) = TempoTxEnvelope::decode_2718(&mut raw) else {
        return PaymentLaneClassification::general(PaymentLaneReason::UnsupportedTransactionType);
    };

    if tx.is_payment_v2() {
        PaymentLaneClassification::payment()
    } else {
        PaymentLaneClassification::general(PaymentLaneReason::NotPaymentLane)
    }
}

/// Structured T5 payment-lane classification for Foundry-facing APIs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentLaneClassification {
    /// The classified lane.
    pub lane: PaymentLane,
    /// Convenience boolean for consumers that only need the lane predicate.
    pub payment: bool,
    /// Structured reason for general-lane classification, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<PaymentLaneReason>,
}

impl PaymentLaneClassification {
    /// Constructs a payment-lane classification.
    pub const fn payment() -> Self {
        Self { lane: PaymentLane::Payment, payment: true, reason: None }
    }

    /// Constructs a general-lane classification with a structured reason.
    pub const fn general(reason: PaymentLaneReason) -> Self {
        Self { lane: PaymentLane::General, payment: false, reason: Some(reason) }
    }
}

/// Payment-lane classifier output lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentLane {
    Payment,
    General,
}

/// Stable Foundry-facing reasons for general-lane classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentLaneReason {
    /// The active network is not Tempo.
    NotTempo,
    /// Tempo is active but the T5 classifier is not active.
    T5NotActive,
    /// The transaction type cannot be classified by Tempo's payment-lane classifier.
    UnsupportedTransactionType,
    /// Tempo's T5 classifier classified the transaction as general.
    NotPaymentLane,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Signed, TxEip1559};
    use alloy_eips::eip2718::Encodable2718;
    use alloy_primitives::{Address, Signature, TxKind, U256, address};
    use alloy_sol_types::SolCall;
    use tempo_alloy::contracts::precompiles::ITIP20;

    const PAYMENT_TOKEN: Address = address!("20c0000000000000000000000000000000000001");

    fn encode_eip1559(to: Address, input: Vec<u8>) -> Vec<u8> {
        let tx = TxEip1559 { to: TxKind::Call(to), input: input.into(), ..Default::default() };
        TempoTxEnvelope::Eip1559(Signed::new_unhashed(tx, Signature::test_signature()))
            .encoded_2718()
    }

    #[test]
    fn classifies_tip20_transfer_as_payment() {
        let input = ITIP20::transferCall { to: Address::ZERO, amount: U256::from(1) }.abi_encode();
        let raw = encode_eip1559(PAYMENT_TOKEN, input);

        assert_eq!(classify_payment_lane(&raw), PaymentLaneClassification::payment());
    }

    #[test]
    fn classifies_non_payment_call_as_general() {
        let raw = encode_eip1559(address!("1234567890123456789012345678901234567890"), vec![]);

        assert_eq!(
            classify_payment_lane(&raw),
            PaymentLaneClassification::general(PaymentLaneReason::NotPaymentLane)
        );
    }

    #[test]
    fn classifies_non_tempo_transaction_type_as_unsupported() {
        // EIP-4844 (0x03) and Optimism deposit (0x7e) have no Tempo envelope variant.
        for type_byte in [0x03_u8, 0x7e] {
            let raw = [type_byte, 0xc0];

            assert_eq!(
                classify_payment_lane(&raw),
                PaymentLaneClassification::general(PaymentLaneReason::UnsupportedTransactionType)
            );
        }
    }

    #[test]
    fn serializes_stable_json_shape() {
        let payment = serde_json::to_value(PaymentLaneClassification::payment()).unwrap();
        assert_eq!(payment, serde_json::json!({ "lane": "payment", "payment": true }));

        let general =
            serde_json::to_value(PaymentLaneClassification::general(PaymentLaneReason::NotTempo))
                .unwrap();
        assert_eq!(
            general,
            serde_json::json!({ "lane": "general", "payment": false, "reason": "not_tempo" })
        );
    }
}
