//! CIP-64 transaction encoding. Execution-time fee currency accounting is not implemented.

use alloy_consensus::{
    SignableTransaction, Transaction, TxEip1559, Typed2718,
    transaction::{RlpEcdsaDecodableTx, RlpEcdsaEncodableTx},
};
use alloy_eips::{eip2718::IsTyped2718, eip2930::AccessList, eip7702::SignedAuthorization};
use alloy_primitives::{Address, B256, Bytes, ChainId, Signature, TxKind, U256};
use alloy_rlp::{BufMut, Decodable, Encodable};
use serde::{Deserialize, Serialize};

/// Celo dynamic fee transaction type introduced by CIP-64.
pub const CIP64_TX_TYPE: u8 = 0x7b;

/// An EIP-1559-shaped transaction with a signed fee currency field.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxCip64 {
    /// Ordinary EIP-1559 transaction fields.
    #[serde(flatten)]
    pub inner: TxEip1559,
    /// Currency used to pay transaction fees, or native CELO when absent.
    #[serde(default)]
    pub fee_currency: Option<Address>,
}

impl RlpEcdsaEncodableTx for TxCip64 {
    fn rlp_encoded_fields_length(&self) -> usize {
        self.inner.rlp_encoded_fields_length()
            + self.fee_currency.map_or(1, |address| address.length())
    }

    fn rlp_encode_fields(&self, out: &mut dyn BufMut) {
        self.inner.rlp_encode_fields(out);
        match self.fee_currency {
            Some(address) => address.encode(out),
            None => out.put_u8(alloy_rlp::EMPTY_STRING_CODE),
        }
    }
}

impl RlpEcdsaDecodableTx for TxCip64 {
    const DEFAULT_TX_TYPE: u8 = CIP64_TX_TYPE;

    fn rlp_decode_fields(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let inner = TxEip1559::rlp_decode_fields(buf)?;
        let fee_currency = match buf.first() {
            Some(&alloy_rlp::EMPTY_STRING_CODE) => {
                *buf = &buf[1..];
                None
            }
            _ => Some(Address::decode(buf)?),
        };
        Ok(Self { inner, fee_currency })
    }
}

impl Typed2718 for TxCip64 {
    fn ty(&self) -> u8 {
        CIP64_TX_TYPE
    }
}

impl IsTyped2718 for TxCip64 {
    fn is_type(type_id: u8) -> bool {
        type_id == CIP64_TX_TYPE
    }
}

impl SignableTransaction<Signature> for TxCip64 {
    fn set_chain_id(&mut self, chain_id: ChainId) {
        self.inner.chain_id = chain_id;
    }

    fn encode_for_signing(&self, out: &mut dyn BufMut) {
        out.put_u8(CIP64_TX_TYPE);
        self.rlp_encode(out);
    }

    fn payload_len_for_signature(&self) -> usize {
        self.rlp_encoded_length() + 1
    }
}

impl Transaction for TxCip64 {
    fn chain_id(&self) -> Option<ChainId> {
        self.inner.chain_id()
    }

    fn nonce(&self) -> u64 {
        self.inner.nonce()
    }

    fn gas_limit(&self) -> u64 {
        self.inner.gas_limit()
    }

    fn gas_price(&self) -> Option<u128> {
        self.inner.gas_price()
    }

    fn max_fee_per_gas(&self) -> u128 {
        self.inner.max_fee_per_gas()
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.inner.max_priority_fee_per_gas()
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.inner.max_fee_per_blob_gas()
    }

    fn priority_fee_or_price(&self) -> u128 {
        self.inner.priority_fee_or_price()
    }

    fn is_dynamic_fee(&self) -> bool {
        self.inner.is_dynamic_fee()
    }

    fn kind(&self) -> TxKind {
        self.inner.kind()
    }

    fn is_create(&self) -> bool {
        self.inner.is_create()
    }

    fn value(&self) -> U256 {
        self.inner.value()
    }

    fn input(&self) -> &Bytes {
        self.inner.input()
    }

    fn access_list(&self) -> Option<&AccessList> {
        self.inner.access_list()
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.inner.blob_versioned_hashes()
    }

    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        self.inner.authorization_list()
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        self.inner.effective_gas_price(base_fee)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FoundryTransactionRequest, FoundryTxEnvelope, FoundryTypedTx};
    use alloy_eips::eip2718::{Decodable2718, Encodable2718};
    use alloy_primitives::{address, hex};
    use serde_json::json;

    #[test]
    fn cip64_encoding_and_signing_payload() {
        let tx = TxCip64 {
            inner: TxEip1559 {
                chain_id: 1,
                max_priority_fee_per_gas: 1,
                max_fee_per_gas: 2,
                gas_limit: 21_000,
                to: address!("1111111111111111111111111111111111111111").into(),
                ..Default::default()
            },
            fee_currency: Some(address!("2222222222222222222222222222222222222222")),
        };
        // CIP-64 appends feeCurrency after accessList, before the signature fields.
        assert_eq!(
            tx.encoded_for_signing(),
            hex!(
                "7bf4018001028252089411111111111111111111111111111111111111118080c0942222222222222222222222222222222222222222"
            )
        );
        let signing_hash = tx.signature_hash();
        let signed = tx.clone().into_signed(Signature::new(U256::ONE, U256::from(2), false));
        let envelope = FoundryTxEnvelope::Celo(signed);
        let encoded = envelope.encoded_2718();
        assert_eq!(
            encoded,
            hex!(
                "7bf7018001028252089411111111111111111111111111111111111111118080c0942222222222222222222222222222222222222222800102"
            )
        );
        assert_eq!(FoundryTxEnvelope::decode_2718(&mut encoded.as_slice()).unwrap(), envelope);
        let mut native = tx;
        native.fee_currency = None;
        assert_ne!(native.signature_hash(), signing_hash);
        let native = FoundryTxEnvelope::Celo(native.into_signed(Signature::new(
            U256::ONE,
            U256::from(2),
            false,
        )));
        assert_eq!(
            FoundryTxEnvelope::decode_2718(&mut native.encoded_2718().as_slice()).unwrap(),
            native
        );
    }

    #[test]
    fn cip64_request_classification_and_conflicts() {
        let currency = "0x2222222222222222222222222222222222222222";
        let request = json!({"feeCurrency":currency,"chainId":"0x1","nonce":"0x0","gas":"0x5208","maxFeePerGas":"0x2","maxPriorityFeePerGas":"0x1","to":"0x1111111111111111111111111111111111111111"});
        let parsed: FoundryTransactionRequest = serde_json::from_value(request.clone()).unwrap();
        let typed = parsed.build_typed_tx().unwrap();
        assert!(matches!(typed, FoundryTypedTx::Celo(_)));
        assert_eq!(
            serde_json::to_value(FoundryTransactionRequest::from(typed)).unwrap()["feeCurrency"],
            currency
        );
        for (field, value) in [
            ("type", json!("0x2")),
            ("gasPrice", json!("0x1")),
            ("authorizationList", json!([])),
            ("blobVersionedHashes", json!([])),
            ("feeToken", json!(currency)),
        ] {
            let mut invalid = request.clone();
            invalid[field] = value;
            assert!(
                serde_json::from_value::<FoundryTransactionRequest>(invalid).is_err(),
                "{field}"
            );
        }
        let native: FoundryTransactionRequest =
            serde_json::from_value(json!({"type":"0x7b"})).unwrap();
        assert!(matches!(native, FoundryTransactionRequest::Celo(_)));
        let ordinary: FoundryTransactionRequest =
            serde_json::from_value(json!({"feeCurrency":null})).unwrap();
        assert!(ordinary.is_ethereum());
    }
}
