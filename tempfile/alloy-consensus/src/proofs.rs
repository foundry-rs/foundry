//! Helper function for calculating Merkle proofs and hashes.

use crate::EMPTY_OMMER_ROOT_HASH;
use alloc::vec::Vec;
use alloy_eips::{eip2718::Encodable2718, eip4895::Withdrawal};
use alloy_primitives::{keccak256, B256};
use alloy_rlp::Encodable;

#[doc(inline)]
pub use alloy_trie::root::{
    ordered_trie_root, ordered_trie_root_with_encoder, state_root, state_root_ref_unhashed,
    state_root_unhashed, state_root_unsorted, storage_root, storage_root_unhashed,
    storage_root_unsorted,
};

/// Calculate a transaction root.
///
/// `(rlp(index), encoded(tx))` pairs.
pub fn calculate_transaction_root<T>(transactions: &[T]) -> B256
where
    T: Encodable2718,
{
    ordered_trie_root_with_encoder(transactions, |tx: &T, buf| tx.encode_2718(buf))
}

/// Calculates the root hash of the withdrawals.
pub fn calculate_withdrawals_root(withdrawals: &[Withdrawal]) -> B256 {
    ordered_trie_root(withdrawals)
}

/// Calculates the root hash for ommer/uncle headers.
///
/// See [`Header`](crate::Header).
pub fn calculate_ommers_root<T>(ommers: &[T]) -> B256
where
    T: Encodable,
{
    // Check if `ommers` list is empty
    if ommers.is_empty() {
        return EMPTY_OMMER_ROOT_HASH;
    }
    // RLP Encode
    let mut ommers_rlp = Vec::new();
    alloy_rlp::encode_list(ommers, &mut ommers_rlp);
    keccak256(ommers_rlp)
}

/// Calculates the receipt root.
pub fn calculate_receipt_root<T>(receipts: &[T]) -> B256
where
    T: Encodable2718,
{
    ordered_trie_root_with_encoder(receipts, |r, buf| r.encode_2718(buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Eip2718EncodableReceipt, Eip658Value, Receipt, ReceiptWithBloom, RlpEncodableReceipt,
        TxType, Typed2718,
    };
    use alloy_primitives::{b256, bloom, Address, Log, LogData};

    struct TypedReceipt {
        ty: TxType,
        receipt: Receipt,
    }

    impl RlpEncodableReceipt for TypedReceipt {
        fn rlp_encoded_length_with_bloom(&self, bloom: &alloy_primitives::Bloom) -> usize {
            let mut payload_length = self.eip2718_encoded_length_with_bloom(bloom);

            if !self.ty.is_legacy() {
                payload_length += alloy_rlp::Header {
                    list: false,
                    payload_length: self.eip2718_encoded_length_with_bloom(bloom),
                }
                .length();
            }

            payload_length
        }

        fn rlp_encode_with_bloom(
            &self,
            bloom: &alloy_primitives::Bloom,
            out: &mut dyn alloy_rlp::BufMut,
        ) {
            if !self.ty.is_legacy() {
                alloy_rlp::Header {
                    list: false,
                    payload_length: self.eip2718_encoded_length_with_bloom(bloom),
                }
                .encode(out)
            }
            self.eip2718_encode_with_bloom(bloom, out);
        }
    }

    impl Eip2718EncodableReceipt for TypedReceipt {
        fn eip2718_encoded_length_with_bloom(&self, bloom: &alloy_primitives::Bloom) -> usize {
            self.receipt.rlp_encoded_length_with_bloom(bloom) + (!self.ty.is_legacy()) as usize
        }

        fn eip2718_encode_with_bloom(
            &self,
            bloom: &alloy_primitives::Bloom,
            out: &mut dyn alloy_rlp::BufMut,
        ) {
            if !self.ty.is_legacy() {
                out.put_u8(self.ty.ty());
            }
            self.receipt.rlp_encode_with_bloom(bloom, out);
        }
    }

    impl Typed2718 for TypedReceipt {
        fn ty(&self) -> u8 {
            self.ty.ty()
        }
    }

    #[test]
    fn check_receipt_root_optimism() {
        let logs = vec![Log {
            address: Address::ZERO,
            data: LogData::new_unchecked(vec![], Default::default()),
        }];
        let logs_bloom = bloom!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001");
        let receipt = ReceiptWithBloom {
            receipt: TypedReceipt {
                receipt: Receipt {
                    status: Eip658Value::success(),
                    cumulative_gas_used: 102068,
                    logs,
                },
                ty: TxType::Eip2930,
            },
            logs_bloom,
        };
        let receipt = vec![receipt];
        let root = calculate_receipt_root(&receipt);
        assert_eq!(root, b256!("fe70ae4a136d98944951b2123859698d59ad251a381abc9960fa81cae3d0d4a0"));
    }
}
