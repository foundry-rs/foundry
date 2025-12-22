use alloy_consensus::{
    Sealed, Signed, TransactionEnvelope, TxEip1559, TxEip2930, TxEnvelope, TxLegacy, TxType,
    Typed2718,
    crypto::RecoveryError,
    transaction::{
        TxEip7702,
        eip4844::{TxEip4844Variant, TxEip4844WithSidecar},
    },
};
use alloy_evm::FromRecoveredTx;
use alloy_network::{AnyRpcTransaction, AnyTxEnvelope};
use alloy_primitives::{Address, B256};
use alloy_rlp::Encodable;
use alloy_rpc_types::ConversionError;
use alloy_serde::WithOtherFields;
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, OpTransaction as OpTransactionTrait, TxDeposit};
use op_revm::OpTransaction;
use revm::context::TxEnv;
use tempo_primitives::{AASigned, TempoTransaction};

//
/// Container type for signed, typed transactions.
// NOTE(onbjerg): Boxing `Tempo(AASigned)` breaks `TransactionEnvelope` derive macro trait bounds.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, TransactionEnvelope)]
#[envelope(
    tx_type_name = FoundryTxType,
    typed = FoundryTypedTx,
)]
pub enum FoundryTxEnvelope {
    /// Legacy transaction type
    #[envelope(ty = 0)]
    Legacy(Signed<TxLegacy>),
    /// [EIP-2930] transaction.
    ///
    /// [EIP-2930]: https://eips.ethereum.org/EIPS/eip-2930
    #[envelope(ty = 1)]
    Eip2930(Signed<TxEip2930>),
    /// [EIP-1559] transaction.
    ///
    /// [EIP-1559]: https://eips.ethereum.org/EIPS/eip-1559
    #[envelope(ty = 2)]
    Eip1559(Signed<TxEip1559>),
    /// [EIP-4844] transaction.
    ///
    /// [EIP-4844]: https://eips.ethereum.org/EIPS/eip-4844
    #[envelope(ty = 3)]
    Eip4844(Signed<TxEip4844Variant>),
    /// [EIP-7702] transaction.
    ///
    /// [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
    #[envelope(ty = 4)]
    Eip7702(Signed<TxEip7702>),
    /// OP stack deposit transaction.
    ///
    /// See <https://docs.optimism.io/op-stack/bridging/deposit-flow>.
    #[envelope(ty = 126)]
    Deposit(Sealed<TxDeposit>),
    /// Tempo transaction type.
    ///
    /// See <https://docs.tempo.xyz/protocol/transactions>.
    #[envelope(ty = 0x76, typed = TempoTransaction)]
    Tempo(AASigned),
}

impl FoundryTxEnvelope {
    /// Converts the transaction into an Ethereum [`TxEnvelope`].
    ///
    /// Returns an error if the transaction is not part of the standard Ethereum transaction types.
    pub fn try_into_eth(self) -> Result<TxEnvelope, Self> {
        match self {
            Self::Legacy(tx) => Ok(TxEnvelope::Legacy(tx)),
            Self::Eip2930(tx) => Ok(TxEnvelope::Eip2930(tx)),
            Self::Eip1559(tx) => Ok(TxEnvelope::Eip1559(tx)),
            Self::Eip4844(tx) => Ok(TxEnvelope::Eip4844(tx)),
            Self::Eip7702(tx) => Ok(TxEnvelope::Eip7702(tx)),
            Self::Deposit(_) => Err(self),
            Self::Tempo(_) => Err(self),
        }
    }

    pub fn sidecar(&self) -> Option<&TxEip4844WithSidecar> {
        match self {
            Self::Eip4844(signed_variant) => match signed_variant.tx() {
                TxEip4844Variant::TxEip4844WithSidecar(with_sidecar) => Some(with_sidecar),
                _ => None,
            },
            _ => None,
        }
    }

    /// Returns the hash of the transaction.
    ///
    /// # Note
    ///
    /// If this transaction has the Impersonated signature then this returns a modified unique
    /// hash. This allows us to treat impersonated transactions as unique.
    pub fn hash(&self) -> B256 {
        match self {
            Self::Legacy(t) => *t.hash(),
            Self::Eip2930(t) => *t.hash(),
            Self::Eip1559(t) => *t.hash(),
            Self::Eip4844(t) => *t.hash(),
            Self::Eip7702(t) => *t.hash(),
            Self::Deposit(t) => t.tx_hash(),
            Self::Tempo(t) => *t.hash(),
        }
    }

    /// Returns the hash if the transaction is impersonated (using a fake signature)
    ///
    /// This appends the `address` before hashing it
    pub fn impersonated_hash(&self, sender: Address) -> B256 {
        let mut buffer = Vec::new();
        Encodable::encode(self, &mut buffer);
        buffer.extend_from_slice(sender.as_ref());
        B256::from_slice(alloy_primitives::utils::keccak256(&buffer).as_slice())
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, RecoveryError> {
        Ok(match self {
            Self::Legacy(tx) => tx.recover_signer()?,
            Self::Eip2930(tx) => tx.recover_signer()?,
            Self::Eip1559(tx) => tx.recover_signer()?,
            Self::Eip4844(tx) => tx.recover_signer()?,
            Self::Eip7702(tx) => tx.recover_signer()?,
            Self::Deposit(tx) => tx.from,
            Self::Tempo(tx) => tx.signature().recover_signer(&tx.signature_hash())?,
        })
    }
}

impl OpTransactionTrait for FoundryTxEnvelope {
    fn is_deposit(&self) -> bool {
        matches!(self, Self::Deposit(_))
    }

    fn as_deposit(&self) -> Option<&Sealed<TxDeposit>> {
        match self {
            Self::Deposit(tx) => Some(tx),
            _ => None,
        }
    }
}

impl TryFrom<FoundryTxEnvelope> for TxEnvelope {
    type Error = FoundryTxEnvelope;

    fn try_from(envelope: FoundryTxEnvelope) -> Result<Self, Self::Error> {
        envelope.try_into_eth()
    }
}

impl TryFrom<AnyRpcTransaction> for FoundryTxEnvelope {
    type Error = ConversionError;

    fn try_from(value: AnyRpcTransaction) -> Result<Self, Self::Error> {
        let WithOtherFields { inner, .. } = value.0;
        let from = inner.inner.signer();
        match inner.inner.into_inner() {
            AnyTxEnvelope::Ethereum(tx) => match tx {
                TxEnvelope::Legacy(tx) => Ok(Self::Legacy(tx)),
                TxEnvelope::Eip2930(tx) => Ok(Self::Eip2930(tx)),
                TxEnvelope::Eip1559(tx) => Ok(Self::Eip1559(tx)),
                TxEnvelope::Eip4844(tx) => Ok(Self::Eip4844(tx)),
                TxEnvelope::Eip7702(tx) => Ok(Self::Eip7702(tx)),
            },
            AnyTxEnvelope::Unknown(mut tx) => {
                // Try to convert to deposit transaction
                if tx.ty() == DEPOSIT_TX_TYPE_ID {
                    tx.inner.fields.insert("from".to_string(), serde_json::to_value(from).unwrap());
                    let deposit_tx =
                        tx.inner.fields.deserialize_into::<TxDeposit>().map_err(|e| {
                            ConversionError::Custom(format!(
                                "Failed to deserialize deposit tx: {e}"
                            ))
                        })?;

                    return Ok(Self::Deposit(Sealed::new(deposit_tx)));
                };

                Err(ConversionError::Custom("UnknownTxType".to_string()))
            }
        }
    }
}

impl FromRecoveredTx<FoundryTxEnvelope> for TxEnv {
    fn from_recovered_tx(tx: &FoundryTxEnvelope, caller: Address) -> Self {
        match tx {
            FoundryTxEnvelope::Legacy(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip2930(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip1559(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip4844(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip7702(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Deposit(sealed_tx) => {
                Self::from_recovered_tx(sealed_tx.inner(), caller)
            }
            FoundryTxEnvelope::Tempo(_) => panic!("unsupported tx type on ethereum"),
        }
    }
}

impl FromRecoveredTx<FoundryTxEnvelope> for OpTransaction<TxEnv> {
    fn from_recovered_tx(tx: &FoundryTxEnvelope, caller: Address) -> Self {
        match tx {
            FoundryTxEnvelope::Legacy(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip2930(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip1559(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip4844(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip7702(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Deposit(sealed_tx) => {
                Self::from_recovered_tx(sealed_tx.inner(), caller)
            }
            FoundryTxEnvelope::Tempo(_) => panic!("unsupported tx type on optimism"),
        }
    }
}

impl std::fmt::Display for FoundryTxType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Legacy => write!(f, "legacy"),
            Self::Eip2930 => write!(f, "eip2930"),
            Self::Eip1559 => write!(f, "eip1559"),
            Self::Eip4844 => write!(f, "eip4844"),
            Self::Eip7702 => write!(f, "eip7702"),
            Self::Deposit => write!(f, "deposit"),
            Self::Tempo => write!(f, "tempo"),
        }
    }
}

impl From<TxType> for FoundryTxType {
    fn from(tx: TxType) -> Self {
        match tx {
            TxType::Legacy => Self::Legacy,
            TxType::Eip2930 => Self::Eip2930,
            TxType::Eip1559 => Self::Eip1559,
            TxType::Eip4844 => Self::Eip4844,
            TxType::Eip7702 => Self::Eip7702,
        }
    }
}

impl From<FoundryTxEnvelope> for FoundryTypedTx {
    fn from(envelope: FoundryTxEnvelope) -> Self {
        match envelope {
            FoundryTxEnvelope::Legacy(signed_tx) => Self::Legacy(signed_tx.strip_signature()),
            FoundryTxEnvelope::Eip2930(signed_tx) => Self::Eip2930(signed_tx.strip_signature()),
            FoundryTxEnvelope::Eip1559(signed_tx) => Self::Eip1559(signed_tx.strip_signature()),
            FoundryTxEnvelope::Eip4844(signed_tx) => Self::Eip4844(signed_tx.strip_signature()),
            FoundryTxEnvelope::Eip7702(signed_tx) => Self::Eip7702(signed_tx.strip_signature()),
            FoundryTxEnvelope::Deposit(sealed_tx) => Self::Deposit(sealed_tx.into_inner()),
            FoundryTxEnvelope::Tempo(signed_tx) => Self::Tempo(signed_tx.strip_signature()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy_primitives::{Bytes, Signature, TxHash, TxKind, U256, b256, hex};
    use alloy_rlp::Decodable;

    use super::*;

    #[test]
    fn test_decode_call() {
        let bytes_first = &mut &hex::decode("f86b02843b9aca00830186a094d3e8763675e4c425df46cc3b5c0f6cbdac39604687038d7ea4c68000802ba00eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5aea03a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca18").unwrap()[..];
        let decoded = FoundryTxEnvelope::decode(&mut &bytes_first[..]).unwrap();

        let tx = TxLegacy {
            nonce: 2u64,
            gas_price: 1000000000u128,
            gas_limit: 100000,
            to: TxKind::Call(Address::from_slice(
                &hex::decode("d3e8763675e4c425df46cc3b5c0f6cbdac396046").unwrap()[..],
            )),
            value: U256::from(1000000000000000u64),
            input: Bytes::default(),
            chain_id: Some(4),
        };

        let signature = Signature::from_str("0eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5ae3a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca182b").unwrap();

        let tx = FoundryTxEnvelope::Legacy(Signed::new_unchecked(
            tx,
            signature,
            b256!("0xa517b206d2223278f860ea017d3626cacad4f52ff51030dc9a96b432f17f8d34"),
        ));

        assert_eq!(tx, decoded);
    }

    #[test]
    fn test_decode_create_goerli() {
        // test that an example create tx from goerli decodes properly
        let tx_bytes =
              hex::decode("02f901ee05228459682f008459682f11830209bf8080b90195608060405234801561001057600080fd5b50610175806100206000396000f3fe608060405234801561001057600080fd5b506004361061002b5760003560e01c80630c49c36c14610030575b600080fd5b61003861004e565b604051610045919061011d565b60405180910390f35b60606020600052600f6020527f68656c6c6f2073746174656d696e64000000000000000000000000000000000060405260406000f35b600081519050919050565b600082825260208201905092915050565b60005b838110156100be5780820151818401526020810190506100a3565b838111156100cd576000848401525b50505050565b6000601f19601f8301169050919050565b60006100ef82610084565b6100f9818561008f565b93506101098185602086016100a0565b610112816100d3565b840191505092915050565b6000602082019050818103600083015261013781846100e4565b90509291505056fea264697066735822122051449585839a4ea5ac23cae4552ef8a96b64ff59d0668f76bfac3796b2bdbb3664736f6c63430008090033c080a0136ebffaa8fc8b9fda9124de9ccb0b1f64e90fbd44251b4c4ac2501e60b104f9a07eb2999eec6d185ef57e91ed099afb0a926c5b536f0155dd67e537c7476e1471")
                  .unwrap();
        let _decoded = FoundryTxEnvelope::decode(&mut &tx_bytes[..]).unwrap();
    }

    #[test]
    fn can_recover_sender() {
        // random mainnet tx: https://etherscan.io/tx/0x86718885c4b4218c6af87d3d0b0d83e3cc465df2a05c048aa4db9f1a6f9de91f
        let bytes = hex::decode("02f872018307910d808507204d2cb1827d0094388c818ca8b9251b393131c08a736a67ccb19297880320d04823e2701c80c001a0cf024f4815304df2867a1a74e9d2707b6abda0337d2d54a4438d453f4160f190a07ac0e6b3bc9395b5b9c8b9e6d77204a236577a5b18467b9175c01de4faa208d9").unwrap();

        let Ok(FoundryTxEnvelope::Eip1559(tx)) = FoundryTxEnvelope::decode(&mut &bytes[..]) else {
            panic!("decoding FoundryTxEnvelope failed");
        };

        assert_eq!(
            tx.hash(),
            &"0x86718885c4b4218c6af87d3d0b0d83e3cc465df2a05c048aa4db9f1a6f9de91f"
                .parse::<B256>()
                .unwrap()
        );
        assert_eq!(
            tx.recover_signer().unwrap(),
            "0x95222290DD7278Aa3Ddd389Cc1E1d165CC4BAfe5".parse::<Address>().unwrap()
        );
    }

    // Test vector from https://sepolia.etherscan.io/tx/0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
    // Blobscan: https://sepolia.blobscan.com/tx/0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
    #[test]
    fn test_decode_live_4844_tx() {
        use alloy_primitives::{address, b256};

        // https://sepolia.etherscan.io/getRawTx?tx=0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
        let raw_tx = alloy_primitives::hex::decode("0x03f9011d83aa36a7820fa28477359400852e90edd0008252089411e9ca82a3a762b4b5bd264d4173a242e7a770648080c08504a817c800f8a5a0012ec3d6f66766bedb002a190126b3549fce0047de0d4c25cffce0dc1c57921aa00152d8e24762ff22b1cfd9f8c0683786a7ca63ba49973818b3d1e9512cd2cec4a0013b98c6c83e066d5b14af2b85199e3d4fc7d1e778dd53130d180f5077e2d1c7a001148b495d6e859114e670ca54fb6e2657f0cbae5b08063605093a4b3dc9f8f1a0011ac212f13c5dff2b2c6b600a79635103d6f580a4221079951181b25c7e654901a0c8de4cced43169f9aa3d36506363b2d2c44f6c49fc1fd91ea114c86f3757077ea01e11fdd0d1934eda0492606ee0bb80a7bf8f35cc5f86ec60fe5031ba48bfd544").unwrap();
        let res = FoundryTxEnvelope::decode(&mut raw_tx.as_slice()).unwrap();
        assert!(res.is_type(3));

        let tx = match res {
            FoundryTxEnvelope::Eip4844(tx) => tx,
            _ => unreachable!(),
        };

        assert_eq!(tx.tx().tx().to, address!("0x11E9CA82A3a762b4B5bd264d4173a242e7a77064"));

        assert_eq!(
            tx.tx().tx().blob_versioned_hashes,
            vec![
                b256!("0x012ec3d6f66766bedb002a190126b3549fce0047de0d4c25cffce0dc1c57921a"),
                b256!("0x0152d8e24762ff22b1cfd9f8c0683786a7ca63ba49973818b3d1e9512cd2cec4"),
                b256!("0x013b98c6c83e066d5b14af2b85199e3d4fc7d1e778dd53130d180f5077e2d1c7"),
                b256!("0x01148b495d6e859114e670ca54fb6e2657f0cbae5b08063605093a4b3dc9f8f1"),
                b256!("0x011ac212f13c5dff2b2c6b600a79635103d6f580a4221079951181b25c7e6549")
            ]
        );

        let from = tx.recover_signer().unwrap();
        assert_eq!(from, address!("0xA83C816D4f9b2783761a22BA6FADB0eB0606D7B2"));
    }

    #[test]
    fn test_decode_encode_deposit_tx() {
        // https://sepolia-optimism.etherscan.io/tx/0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7
        let tx_hash: TxHash = "0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7"
            .parse::<TxHash>()
            .unwrap();

        // https://sepolia-optimism.etherscan.io/getRawTx?tx=0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7
        let raw_tx = alloy_primitives::hex::decode(
            "7ef861a0dfd7ae78bf3c414cfaa77f13c0205c82eb9365e217b2daa3448c3156b69b27ac94778f2146f48179643473b82931c4cd7b8f153efd94778f2146f48179643473b82931c4cd7b8f153efd872386f26fc10000872386f26fc10000830186a08080",
        )
        .unwrap();
        let dep_tx = FoundryTxEnvelope::decode(&mut raw_tx.as_slice()).unwrap();

        let mut encoded = Vec::new();
        dep_tx.encode_2718(&mut encoded);

        assert_eq!(raw_tx, encoded);

        assert_eq!(tx_hash, dep_tx.hash());
    }

    #[test]
    fn can_recover_sender_not_normalized() {
        let bytes = hex::decode("f85f800182520894095e7baea6a6c7c4c2dfeb977efac326af552d870a801ba048b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353a0efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c804").unwrap();

        let Ok(FoundryTxEnvelope::Legacy(tx)) = FoundryTxEnvelope::decode(&mut &bytes[..]) else {
            panic!("decoding FoundryTxEnvelope failed");
        };

        assert_eq!(tx.tx().input, Bytes::from(b""));
        assert_eq!(tx.tx().gas_price, 1);
        assert_eq!(tx.tx().gas_limit, 21000);
        assert_eq!(tx.tx().nonce, 0);
        if let TxKind::Call(to) = tx.tx().to {
            assert_eq!(
                to,
                "0x095e7baea6a6c7c4c2dfeb977efac326af552d87".parse::<Address>().unwrap()
            );
        } else {
            panic!("expected a call transaction");
        }
        assert_eq!(tx.tx().value, U256::from(0x0au64));
        assert_eq!(
            tx.recover_signer().unwrap(),
            "0f65fe9276bc9a24ae7083ae28e2660ef72df99e".parse::<Address>().unwrap()
        );
    }

    #[test]
    fn deser_to_type_tx() {
        let tx = r#"
        {
            "type": "0x2",
            "chainId": "0x7a69",
            "nonce": "0x0",
            "gas": "0x5209",
            "maxFeePerGas": "0x77359401",
            "maxPriorityFeePerGas": "0x1",
            "to": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "value": "0x0",
            "accessList": [],
            "input": "0x",
            "r": "0x85c2794a580da137e24ccc823b45ae5cea99371ae23ee13860fcc6935f8305b0",
            "s": "0x41de7fa4121dab284af4453d30928241208bafa90cdb701fe9bc7054759fe3cd",
            "yParity": "0x0",
            "hash": "0x8c9b68e8947ace33028dba167354fde369ed7bbe34911b772d09b3c64b861515"
        }"#;

        let _typed_tx: FoundryTxEnvelope = serde_json::from_str(tx).unwrap();
    }

    #[test]
    fn test_from_recovered_tx_legacy() {
        let tx = r#"
        {
            "type": "0x0",
            "chainId": "0x1",
            "nonce": "0x0",
            "gas": "0x5208",
            "gasPrice": "0x1",
            "to": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "value": "0x1",
            "input": "0x",
            "r": "0x85c2794a580da137e24ccc823b45ae5cea99371ae23ee13860fcc6935f8305b0",
            "s": "0x41de7fa4121dab284af4453d30928241208bafa90cdb701fe9bc7054759fe3cd",
            "v": "0x1b",
            "hash": "0x8c9b68e8947ace33028dba167354fde369ed7bbe34911b772d09b3c64b861515"
        }"#;

        let typed_tx: FoundryTxEnvelope = serde_json::from_str(tx).unwrap();
        let sender = typed_tx.recover().unwrap();

        // Test TxEnv conversion via FromRecoveredTx trait
        let tx_env = TxEnv::from_recovered_tx(&typed_tx, sender);
        assert_eq!(tx_env.caller, sender);
        assert_eq!(tx_env.gas_limit, 0x5208);
        assert_eq!(tx_env.gas_price, 1);

        // Test OpTransaction<TxEnv> conversion via FromRecoveredTx trait
        let op_tx = OpTransaction::<TxEnv>::from_recovered_tx(&typed_tx, sender);
        assert_eq!(op_tx.base.caller, sender);
        assert_eq!(op_tx.base.gas_limit, 0x5208);
    }

    // Test vector from Tempo testnet:
    // https://explorer.testnet.tempo.xyz/tx/0x6d6d8c102064e6dee44abad2024a8b1d37959230baab80e70efbf9b0c739c4fd
    #[test]
    fn test_decode_encode_tempo_tx() {
        use alloy_primitives::address;
        use tempo_primitives::TEMPO_TX_TYPE_ID;

        let tx_hash: TxHash = "0x6d6d8c102064e6dee44abad2024a8b1d37959230baab80e70efbf9b0c739c4fd"
            .parse::<TxHash>()
            .unwrap();

        // Raw transaction from Tempo testnet via eth_getRawTransactionByHash
        let raw_tx = hex::decode(
            "76f9025e82a5bd808502cb4178008302d178f8fcf85c9420c000000000000000000000000000000000000080b844095ea7b3000000000000000000000000dec00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000989680f89c94dec000000000000000000000000000000000000080b884f8856c0f00000000000000000000000020c000000000000000000000000000000000000000000000000000000000000020c00000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000989680000000000000000000000000000000000000000000000000000000000097d330c0808080809420c000000000000000000000000000000000000180c0b90133027b98b7a8e6c68d7eac741a52e6fdae0560ce3c16ef5427ad46d7a54d0ed86dd41d000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a2238453071464a7a50585167546e645473643649456659457776323173516e626966374c4741776e4b43626b222c226f726967696e223a2268747470733a2f2f74656d706f2d6465782e76657263656c2e617070222c2263726f73734f726967696e223a66616c73657dcfd45c3b19745a42f80b134dcb02a8ba099a0e4e7be1984da54734aa81d8f29f74bb9170ae6d25bd510c83fe35895ee5712efe13980a5edc8094c534e23af85eaacc80b21e45fb11f349424dce3a2f23547f60c0ff2f8bcaede2a247545ce8dd87abf0dbb7a5c9507efae2e43833356651b45ac576c2e61cec4e9c0f41fcbf6e",
        )
        .unwrap();

        let tempo_tx = FoundryTxEnvelope::decode(&mut raw_tx.as_slice()).unwrap();

        // Verify it's a Tempo transaction (type 0x76)
        assert!(tempo_tx.is_type(TEMPO_TX_TYPE_ID));

        let FoundryTxEnvelope::Tempo(ref aa_signed) = tempo_tx else {
            panic!("Expected Tempo transaction");
        };

        // Verify the chain ID
        assert_eq!(aa_signed.tx().chain_id, 42429);

        // Verify the fee token
        assert_eq!(
            aa_signed.tx().fee_token,
            Some(address!("0x20C0000000000000000000000000000000000001"))
        );

        // Verify gas limit
        assert_eq!(aa_signed.tx().gas_limit, 184696);

        // Verify we have 2 calls
        assert_eq!(aa_signed.tx().calls.len(), 2);

        // Verify the hash
        assert_eq!(tx_hash, tempo_tx.hash());

        // Verify round-trip encoding
        let mut encoded = Vec::new();
        tempo_tx.encode_2718(&mut encoded);
        assert_eq!(raw_tx, encoded);

        // Verify sender recovery (WebAuthn signature)
        let sender = tempo_tx.recover().unwrap();
        assert_eq!(sender, address!("0x566Ff0f4a6114F8072ecDC8A7A8A13d8d0C6B45F"));
    }
}
