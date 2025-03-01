use crate::{SignableTransaction, Signed, Transaction, TxType};
use alloc::vec::Vec;
use alloy_eips::{
    eip2930::AccessList,
    eip7702::{constants::EIP7702_TX_TYPE_ID, SignedAuthorization},
    Typed2718,
};
use alloy_primitives::{
    Address, Bytes, ChainId, PrimitiveSignature as Signature, TxKind, B256, U256,
};
use alloy_rlp::{BufMut, Decodable, Encodable};
use core::mem;

use super::RlpEcdsaTx;

/// A transaction with a priority fee ([EIP-7702](https://eips.ethereum.org/EIPS/eip-7702)).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[doc(alias = "Eip7702Transaction", alias = "TransactionEip7702", alias = "Eip7702Tx")]
pub struct TxEip7702 {
    /// EIP-155: Simple replay attack protection
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub chain_id: ChainId,
    /// A scalar value equal to the number of transactions sent by the sender; formally Tn.
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub nonce: u64,
    /// A scalar value equal to the maximum
    /// amount of gas that should be used in executing
    /// this transaction. This is paid up-front, before any
    /// computation is done and may not be increased
    /// later; formally Tg.
    #[cfg_attr(
        feature = "serde",
        serde(with = "alloy_serde::quantity", rename = "gas", alias = "gasLimit")
    )]
    pub gas_limit: u64,
    /// A scalar value equal to the maximum
    /// amount of gas that should be used in executing
    /// this transaction. This is paid up-front, before any
    /// computation is done and may not be increased
    /// later; formally Tg.
    ///
    /// As ethereum circulation is around 120mil eth as of 2022 that is around
    /// 120000000000000000000000000 wei we are safe to use u128 as its max number is:
    /// 340282366920938463463374607431768211455
    ///
    /// This is also known as `GasFeeCap`
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub max_fee_per_gas: u128,
    /// Max Priority fee that transaction is paying
    ///
    /// As ethereum circulation is around 120mil eth as of 2022 that is around
    /// 120000000000000000000000000 wei we are safe to use u128 as its max number is:
    /// 340282366920938463463374607431768211455
    ///
    /// This is also known as `GasTipCap`
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub max_priority_fee_per_gas: u128,
    /// The 160-bit address of the message call’s recipient.
    pub to: Address,
    /// A scalar value equal to the number of Wei to
    /// be transferred to the message call’s recipient or,
    /// in the case of contract creation, as an endowment
    /// to the newly created account; formally Tv.
    pub value: U256,
    /// The accessList specifies a list of addresses and storage keys;
    /// these addresses and storage keys are added into the `accessed_addresses`
    /// and `accessed_storage_keys` global sets (introduced in EIP-2929).
    /// A gas cost is charged, though at a discount relative to the cost of
    /// accessing outside the list.
    pub access_list: AccessList,
    /// Authorizations are used to temporarily set the code of its signer to
    /// the code referenced by `address`. These also include a `chain_id` (which
    /// can be set to zero and not evaluated) as well as an optional `nonce`.
    pub authorization_list: Vec<SignedAuthorization>,
    /// Input has two uses depending if transaction is Create or Call (if `to` field is None or
    /// Some). pub init: An unlimited size byte array specifying the
    /// EVM-code for the account initialisation procedure CREATE,
    /// data: An unlimited size byte array specifying the
    /// input data of the message call, formally Td.
    pub input: Bytes,
}

impl TxEip7702 {
    /// Get the transaction type.
    #[doc(alias = "transaction_type")]
    pub const fn tx_type() -> TxType {
        TxType::Eip7702
    }

    /// Calculates a heuristic for the in-memory size of the [TxEip7702] transaction.
    #[inline]
    pub fn size(&self) -> usize {
        mem::size_of::<ChainId>() + // chain_id
        mem::size_of::<u64>() + // nonce
        mem::size_of::<u64>() + // gas_limit
        mem::size_of::<u128>() + // max_fee_per_gas
        mem::size_of::<u128>() + // max_priority_fee_per_gas
        mem::size_of::<Address>() + // to
        mem::size_of::<U256>() + // value
        self.access_list.size() + // access_list
        self.input.len() + // input
        self.authorization_list.capacity() * mem::size_of::<SignedAuthorization>() // authorization_list
    }
}

impl RlpEcdsaTx for TxEip7702 {
    const DEFAULT_TX_TYPE: u8 = { Self::tx_type() as u8 };

    /// Outputs the length of the transaction's fields, without a RLP header.
    #[doc(hidden)]
    fn rlp_encoded_fields_length(&self) -> usize {
        self.chain_id.length()
            + self.nonce.length()
            + self.max_priority_fee_per_gas.length()
            + self.max_fee_per_gas.length()
            + self.gas_limit.length()
            + self.to.length()
            + self.value.length()
            + self.input.0.length()
            + self.access_list.length()
            + self.authorization_list.length()
    }

    fn rlp_encode_fields(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.chain_id.encode(out);
        self.nonce.encode(out);
        self.max_priority_fee_per_gas.encode(out);
        self.max_fee_per_gas.encode(out);
        self.gas_limit.encode(out);
        self.to.encode(out);
        self.value.encode(out);
        self.input.0.encode(out);
        self.access_list.encode(out);
        self.authorization_list.encode(out);
    }

    fn rlp_decode_fields(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        Ok(Self {
            chain_id: Decodable::decode(buf)?,
            nonce: Decodable::decode(buf)?,
            max_priority_fee_per_gas: Decodable::decode(buf)?,
            max_fee_per_gas: Decodable::decode(buf)?,
            gas_limit: Decodable::decode(buf)?,
            to: Decodable::decode(buf)?,
            value: Decodable::decode(buf)?,
            input: Decodable::decode(buf)?,
            access_list: Decodable::decode(buf)?,
            authorization_list: Decodable::decode(buf)?,
        })
    }
}

impl Transaction for TxEip7702 {
    #[inline]
    fn chain_id(&self) -> Option<ChainId> {
        Some(self.chain_id)
    }

    #[inline]
    fn nonce(&self) -> u64 {
        self.nonce
    }

    #[inline]
    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    #[inline]
    fn gas_price(&self) -> Option<u128> {
        None
    }

    #[inline]
    fn max_fee_per_gas(&self) -> u128 {
        self.max_fee_per_gas
    }

    #[inline]
    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        Some(self.max_priority_fee_per_gas)
    }

    #[inline]
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        None
    }

    #[inline]
    fn priority_fee_or_price(&self) -> u128 {
        self.max_priority_fee_per_gas
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        base_fee.map_or(self.max_fee_per_gas, |base_fee| {
            // if the tip is greater than the max priority fee per gas, set it to the max
            // priority fee per gas + base fee
            let tip = self.max_fee_per_gas.saturating_sub(base_fee as u128);
            if tip > self.max_priority_fee_per_gas {
                self.max_priority_fee_per_gas + base_fee as u128
            } else {
                // otherwise return the max fee per gas
                self.max_fee_per_gas
            }
        })
    }

    #[inline]
    fn is_dynamic_fee(&self) -> bool {
        true
    }

    #[inline]
    fn kind(&self) -> TxKind {
        self.to.into()
    }

    #[inline]
    fn is_create(&self) -> bool {
        false
    }

    #[inline]
    fn value(&self) -> U256 {
        self.value
    }

    #[inline]
    fn input(&self) -> &Bytes {
        &self.input
    }

    #[inline]
    fn access_list(&self) -> Option<&AccessList> {
        Some(&self.access_list)
    }

    #[inline]
    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        None
    }

    #[inline]
    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        Some(&self.authorization_list)
    }
}

impl SignableTransaction<Signature> for TxEip7702 {
    fn set_chain_id(&mut self, chain_id: ChainId) {
        self.chain_id = chain_id;
    }

    fn encode_for_signing(&self, out: &mut dyn alloy_rlp::BufMut) {
        out.put_u8(EIP7702_TX_TYPE_ID);
        self.encode(out)
    }

    fn payload_len_for_signature(&self) -> usize {
        self.length() + 1
    }

    fn into_signed(self, signature: Signature) -> Signed<Self> {
        let tx_hash = self.tx_hash(&signature);

        Signed::new_unchecked(self, signature, tx_hash)
    }
}

impl Typed2718 for TxEip7702 {
    fn ty(&self) -> u8 {
        TxType::Eip7702 as u8
    }
}

impl Encodable for TxEip7702 {
    fn encode(&self, out: &mut dyn BufMut) {
        self.rlp_encode(out);
    }

    fn length(&self) -> usize {
        self.rlp_encoded_length()
    }
}

impl Decodable for TxEip7702 {
    fn decode(data: &mut &[u8]) -> alloy_rlp::Result<Self> {
        Self::rlp_decode(data)
    }
}

/// Bincode-compatible [`TxEip7702`] serde implementation.
#[cfg(all(feature = "serde", feature = "serde-bincode-compat"))]
pub(super) mod serde_bincode_compat {
    use alloc::{borrow::Cow, vec::Vec};
    use alloy_eips::{eip2930::AccessList, eip7702::serde_bincode_compat::SignedAuthorization};
    use alloy_primitives::{Address, Bytes, ChainId, U256};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_with::{DeserializeAs, SerializeAs};

    /// Bincode-compatible [`super::TxEip7702`] serde implementation.
    ///
    /// Intended to use with the [`serde_with::serde_as`] macro in the following way:
    /// ```rust
    /// use alloy_consensus::{serde_bincode_compat, TxEip7702};
    /// use serde::{Deserialize, Serialize};
    /// use serde_with::serde_as;
    ///
    /// #[serde_as]
    /// #[derive(Serialize, Deserialize)]
    /// struct Data {
    ///     #[serde_as(as = "serde_bincode_compat::transaction::TxEip7702")]
    ///     transaction: TxEip7702,
    /// }
    /// ```
    #[derive(Debug, Serialize, Deserialize)]
    pub struct TxEip7702<'a> {
        chain_id: ChainId,
        nonce: u64,
        gas_limit: u64,
        max_fee_per_gas: u128,
        max_priority_fee_per_gas: u128,
        to: Address,
        value: U256,
        access_list: Cow<'a, AccessList>,
        authorization_list: Vec<SignedAuthorization<'a>>,
        input: Cow<'a, Bytes>,
    }

    impl<'a> From<&'a super::TxEip7702> for TxEip7702<'a> {
        fn from(value: &'a super::TxEip7702) -> Self {
            Self {
                chain_id: value.chain_id,
                nonce: value.nonce,
                gas_limit: value.gas_limit,
                max_fee_per_gas: value.max_fee_per_gas,
                max_priority_fee_per_gas: value.max_priority_fee_per_gas,
                to: value.to,
                value: value.value,
                access_list: Cow::Borrowed(&value.access_list),
                authorization_list: value.authorization_list.iter().map(Into::into).collect(),
                input: Cow::Borrowed(&value.input),
            }
        }
    }

    impl<'a> From<TxEip7702<'a>> for super::TxEip7702 {
        fn from(value: TxEip7702<'a>) -> Self {
            Self {
                chain_id: value.chain_id,
                nonce: value.nonce,
                gas_limit: value.gas_limit,
                max_fee_per_gas: value.max_fee_per_gas,
                max_priority_fee_per_gas: value.max_priority_fee_per_gas,
                to: value.to,
                value: value.value,
                access_list: value.access_list.into_owned(),
                authorization_list: value.authorization_list.into_iter().map(Into::into).collect(),
                input: value.input.into_owned(),
            }
        }
    }

    impl SerializeAs<super::TxEip7702> for TxEip7702<'_> {
        fn serialize_as<S>(source: &super::TxEip7702, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            TxEip7702::from(source).serialize(serializer)
        }
    }

    impl<'de> DeserializeAs<'de, super::TxEip7702> for TxEip7702<'de> {
        fn deserialize_as<D>(deserializer: D) -> Result<super::TxEip7702, D::Error>
        where
            D: Deserializer<'de>,
        {
            TxEip7702::deserialize(deserializer).map(Into::into)
        }
    }

    #[cfg(test)]
    mod tests {
        use arbitrary::Arbitrary;
        use rand::Rng;
        use serde::{Deserialize, Serialize};
        use serde_with::serde_as;

        use super::super::{serde_bincode_compat, TxEip7702};

        #[test]
        fn test_tx_eip7702_bincode_roundtrip() {
            #[serde_as]
            #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
            struct Data {
                #[serde_as(as = "serde_bincode_compat::TxEip7702")]
                transaction: TxEip7702,
            }

            let mut bytes = [0u8; 1024];
            rand::thread_rng().fill(bytes.as_mut_slice());
            let data = Data {
                transaction: TxEip7702::arbitrary(&mut arbitrary::Unstructured::new(&bytes))
                    .unwrap(),
            };

            let encoded = bincode::serialize(&data).unwrap();
            let decoded: Data = bincode::deserialize(&encoded).unwrap();
            assert_eq!(decoded, data);
        }
    }
}

#[cfg(all(test, feature = "k256"))]
mod tests {
    use super::*;
    use crate::SignableTransaction;
    use alloy_eips::eip2930::AccessList;
    use alloy_primitives::{address, b256, hex, Address, PrimitiveSignature as Signature, U256};

    #[test]
    fn encode_decode_eip7702() {
        let tx =  TxEip7702 {
            chain_id: 1,
            nonce: 0x42,
            gas_limit: 44386,
            to: address!("6069a6c32cf691f5982febae4faf8a6f3ab2f0f6"),
            value: U256::from(0_u64),
            input:  hex!("a22cb4650000000000000000000000005eee75727d804a2b13038928d36f8b188945a57a0000000000000000000000000000000000000000000000000000000000000000").into(),
            max_fee_per_gas: 0x4a817c800,
            max_priority_fee_per_gas: 0x3b9aca00,
            access_list: AccessList::default(),
            authorization_list: vec![],
        };

        let sig = Signature::from_scalars_and_parity(
            b256!("840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca90058565"),
            b256!("25e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d1"),
            false,
        );

        let mut buf = vec![];
        tx.rlp_encode_signed(&sig, &mut buf);
        let decoded = TxEip7702::rlp_decode_signed(&mut &buf[..]).unwrap();
        assert_eq!(decoded, tx.into_signed(sig));
    }

    #[test]
    fn test_decode_create() {
        // tests that a contract creation tx encodes and decodes properly
        let tx = TxEip7702 {
            chain_id: 1u64,
            nonce: 0,
            max_fee_per_gas: 0x4a817c800,
            max_priority_fee_per_gas: 0x3b9aca00,
            gas_limit: 2,
            to: Address::default(),
            value: U256::ZERO,
            input: vec![1, 2].into(),
            access_list: Default::default(),
            authorization_list: Default::default(),
        };
        let sig = Signature::from_scalars_and_parity(
            b256!("840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca90058565"),
            b256!("25e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d1"),
            false,
        );
        let mut buf = vec![];
        tx.rlp_encode_signed(&sig, &mut buf);
        let decoded = TxEip7702::rlp_decode_signed(&mut &buf[..]).unwrap();
        assert_eq!(decoded, tx.into_signed(sig));
    }

    #[test]
    fn test_decode_call() {
        let tx = TxEip7702 {
            chain_id: 1u64,
            nonce: 0,
            max_fee_per_gas: 0x4a817c800,
            max_priority_fee_per_gas: 0x3b9aca00,
            gas_limit: 2,
            to: Address::default(),
            value: U256::ZERO,
            input: vec![1, 2].into(),
            access_list: Default::default(),
            authorization_list: Default::default(),
        };

        let sig = Signature::from_scalars_and_parity(
            b256!("840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca90058565"),
            b256!("25e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d1"),
            false,
        );

        let mut buf = vec![];
        tx.rlp_encode_signed(&sig, &mut buf);
        let decoded = TxEip7702::rlp_decode_signed(&mut &buf[..]).unwrap();
        assert_eq!(decoded, tx.into_signed(sig));
    }
}
