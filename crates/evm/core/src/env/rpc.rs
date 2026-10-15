//! RPC transaction conversion into concrete execution environments.

use alloy_chains::NamedChain;
use alloy_consensus::{Transaction as _, Typed2718};
use alloy_evm::FromRecoveredTx;
use alloy_network::{AnyRpcTransaction, AnyTxEnvelope, TransactionResponse};
use foundry_evm_networks::celo::CELO_DYNAMIC_FEE_TX_TYPE;
use revm::context::TxEnv;
use tempo_alloy::primitives::{TEMPO_TX_TYPE_ID, TempoTxEnvelope};
use tempo_revm::TempoTxEnv;

/// Trait for converting an [`AnyRpcTransaction`] into a specific `TxEnv`.
///
/// Ethereum envelopes delegate to [`FromRecoveredTx`]. Implementations may also explicitly
/// project compatible network-specific envelopes into their execution environment.
pub trait FromAnyRpcTransaction: Sized {
    /// Tries to convert an [`AnyRpcTransaction`] into `Self`.
    fn from_any_rpc_transaction(tx: &AnyRpcTransaction) -> eyre::Result<Self>;
}

/// Returns the error for a transaction whose envelope `target` cannot represent.
fn unknown_transaction_type(tx: &AnyRpcTransaction, target: &str) -> eyre::Report {
    eyre::eyre!("cannot convert unknown transaction type {:#04x} to {target}", tx.inner.inner.ty())
}

impl FromAnyRpcTransaction for TxEnv {
    fn from_any_rpc_transaction(tx: &AnyRpcTransaction) -> eyre::Result<Self> {
        if let Some(envelope) = tx.as_envelope() {
            return Ok(Self::from_recovered_tx(envelope, tx.from()));
        }

        // CIP-64 transactions have EIP-1559 execution fields plus a Celo-specific fee currency.
        // Foundry does not model fee payment in TxEnv, but can replay their EVM payload. Preserve
        // the custom type so revm does not compare the fee-currency price with the native-CELO
        // base fee. Keep this projection restricted to active Celo chains so an unrelated network
        // cannot silently acquire semantics for its own type 0x7b envelope.
        if let AnyTxEnvelope::Unknown(unknown) = &*tx.inner.inner
            && unknown.ty() == CELO_DYNAMIC_FEE_TX_TYPE
            && matches!(
                unknown.chain_id().and_then(NamedChain::from_chain_id),
                Some(NamedChain::Celo | NamedChain::CeloSepolia)
            )
        {
            return Ok(Self {
                tx_type: CELO_DYNAMIC_FEE_TX_TYPE,
                caller: tx.from(),
                gas_limit: unknown.gas_limit(),
                gas_price: unknown.max_fee_per_gas(),
                gas_priority_fee: unknown.max_priority_fee_per_gas(),
                kind: unknown.kind(),
                value: unknown.value(),
                data: unknown.input().clone(),
                nonce: unknown.nonce(),
                chain_id: unknown.chain_id(),
                access_list: unknown.access_list().cloned().unwrap_or_default(),
                ..Default::default()
            });
        }

        Err(unknown_transaction_type(tx, "TxEnv"))
    }
}

impl FromAnyRpcTransaction for TempoTxEnv {
    fn from_any_rpc_transaction(tx: &AnyRpcTransaction) -> eyre::Result<Self> {
        // Rebuild the signed Tempo envelope so Tempo's own conversion populates the transaction
        // hash and sender-scoped identifier for every type, and the batch calls, nonce key,
        // validity window, authorizations and fee payer for AA transactions.
        let envelope = match &*tx.inner.inner {
            AnyTxEnvelope::Ethereum(envelope) => TempoTxEnvelope::try_from(envelope.clone())
                .map_err(|_| unknown_transaction_type(tx, "TempoTxEnv"))?,
            AnyTxEnvelope::Unknown(unknown) if unknown.ty() == TEMPO_TX_TYPE_ID => {
                serde_json::from_value::<alloy_rpc_types::Transaction<TempoTxEnvelope>>(
                    serde_json::to_value(tx)?,
                )
                .map_err(|err| eyre::eyre!("cannot decode Tempo AA transaction: {err}"))?
                .inner
                .into_inner()
            }
            AnyTxEnvelope::Unknown(_) => return Err(unknown_transaction_type(tx, "TempoTxEnv")),
        };
        Ok(Self::from_recovered_tx(&envelope, tx.from()))
    }
}

#[cfg(feature = "base")]
mod base {
    use super::*;
    use base_common_consensus::BaseTxEnvelope;
    use base_common_evm::{BaseTransaction, EIP8130_TRANSACTION_TYPE};
    use base_common_rpc_types::Transaction as BaseRpcTransaction;

    impl FromAnyRpcTransaction for BaseTransaction<TxEnv> {
        fn from_any_rpc_transaction(tx: &AnyRpcTransaction) -> eyre::Result<Self> {
            let envelope = match BaseTxEnvelope::try_from(tx.clone()) {
                Ok(envelope) => envelope,
                Err(_) if tx.ty() == EIP8130_TRANSACTION_TYPE => {
                    let rpc_tx =
                        serde_json::from_value::<BaseRpcTransaction>(serde_json::to_value(tx)?)
                            .map_err(|err| {
                                eyre::eyre!(
                                    "cannot convert RPC transaction to Base envelope: {err}"
                                )
                            })?;
                    rpc_tx.inner.into_inner()
                }
                Err(_) => eyre::bail!("cannot convert transaction to BaseTxEnvelope"),
            };
            Ok(Self::from_recovered_tx(&envelope, tx.from()))
        }
    }
}

#[cfg(feature = "optimism")]
mod optimism {
    use super::*;
    use alloy_eips::eip2718::Encodable2718;
    use alloy_op_evm::OpTx;
    use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, TxDeposit};
    use op_revm::OpTransaction;

    impl FromAnyRpcTransaction for OpTx {
        fn from_any_rpc_transaction(tx: &AnyRpcTransaction) -> eyre::Result<Self> {
            if let Some(envelope) = tx.as_envelope() {
                return Ok(Self(OpTransaction::<TxEnv> {
                    base: TxEnv::from_recovered_tx(envelope, tx.from()),
                    // The L1 data fee is charged off these bytes, and op-revm rejects a
                    // non-deposit transaction that arrives without them.
                    enveloped_tx: Some(envelope.encoded_2718().into()),
                    deposit: Default::default(),
                }));
            }

            // Handle OP deposit transactions from `Unknown` envelope variant.
            if let AnyTxEnvelope::Unknown(unknown) = &*tx.inner.inner
                && unknown.ty() == DEPOSIT_TX_TYPE_ID
            {
                let mut fields = unknown.inner.fields.clone();
                fields.insert("from".to_string(), serde_json::to_value(tx.from())?);
                let deposit_tx: TxDeposit = fields
                    .deserialize_into()
                    .map_err(|e| eyre::eyre!("failed to deserialize deposit tx: {e}"))?;
                return Ok(Self::from_recovered_tx(&deposit_tx, deposit_tx.from));
            }

            Err(unknown_transaction_type(tx, "OpTransaction"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Signed, TxEip1559, transaction::Recovered};
    use alloy_network::{AnyTxType, UnknownTxEnvelope, UnknownTypedTransaction};
    use alloy_primitives::{Address, B256, Bytes, Signature, U256, address, b256, bytes};
    use alloy_rpc_types::{Transaction as RpcTransaction, TransactionInfo};
    use alloy_serde::WithOtherFields;
    use alloy_signer::SignerSync;
    use alloy_signer_local::PrivateKeySigner;
    use revm::primitives::TxKind;
    use std::num::NonZeroU64;
    use tempo_alloy::primitives::{TempoSignature, TempoTransaction, transaction::Call};
    use tempo_revm::ExecutionContext;

    #[cfg(feature = "base")]
    use base_common_evm::BaseTransaction;

    fn make_signed_eip1559() -> Signed<TxEip1559> {
        Signed::new_unchecked(
            TxEip1559 {
                chain_id: 1,
                nonce: 42,
                gas_limit: 21001,
                to: TxKind::Call(Address::with_last_byte(0xBB)),
                value: U256::from(101),
                ..Default::default()
            },
            Signature::new(U256::ZERO, U256::ZERO, false),
            B256::ZERO,
        )
    }

    #[test]
    fn from_any_rpc_transaction_for_eth() {
        let from = Address::random();
        let any_tx = eth_rpc_transaction(from);
        let tx_env = TxEnv::from_any_rpc_transaction(&any_tx).unwrap();

        assert_eq!(tx_env.caller, from);
        assert_eq!(tx_env.nonce, 42);
        assert_eq!(tx_env.gas_limit, 21001);
        assert_eq!(tx_env.value, U256::from(101));
        assert_eq!(tx_env.kind, TxKind::Call(Address::with_last_byte(0xBB)));
    }

    #[cfg(feature = "base")]
    #[test]
    fn from_any_rpc_transaction_for_base_eth_envelope() {
        let from = Address::random();
        let any_tx = eth_rpc_transaction(from);

        let tx_env = BaseTransaction::<TxEnv>::from_any_rpc_transaction(&any_tx).unwrap();
        assert_eq!(tx_env.base.caller, from);
        assert_eq!(tx_env.base.nonce, 42);
        assert_eq!(tx_env.base.gas_limit, 21001);
        assert_eq!(tx_env.base.value, U256::from(101));
        assert!(tx_env.enveloped_tx.is_some());
    }

    #[test]
    fn from_any_rpc_transaction_unknown_envelope_errors() {
        let unknown = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
            hash: B256::ZERO,
            inner: UnknownTypedTransaction {
                ty: AnyTxType(0xFF),
                fields: Default::default(),
                memo: Default::default(),
            },
        });
        let from = Address::random();
        let any_tx = AnyRpcTransaction::new(WithOtherFields::new(RpcTransaction {
            inner: Recovered::new_unchecked(unknown, from),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            effective_gas_price: None,
            block_timestamp: None,
        }));

        let result = TxEnv::from_any_rpc_transaction(&any_tx).unwrap_err();
        assert_eq!(result.to_string(), "cannot convert unknown transaction type 0xff to TxEnv");
    }

    #[test]
    fn from_any_rpc_transaction_for_celo_dynamic_fee() {
        let from = Address::with_last_byte(0xAA);
        let to = Address::with_last_byte(0xBB);
        let fee_currency = Address::with_last_byte(0xCC);
        let json = serde_json::json!({
            "accessList": [],
            "blockHash": B256::ZERO,
            "blockNumber": "0x1",
            "chainId": "0xa4ec",
            "feeCurrency": fee_currency,
            "from": from,
            "gas": "0x5208",
            "gasPrice": "0x3",
            "hash": B256::ZERO,
            "input": "0x1234",
            "maxFeePerGas": "0x3",
            "maxPriorityFeePerGas": "0x1",
            "nonce": "0x2a",
            "r": B256::ZERO,
            "s": B256::ZERO,
            "to": to,
            "transactionIndex": "0x0",
            "type": "0x7b",
            "v": "0x0",
            "value": "0x65",
            "yParity": "0x0"
        });
        let mut non_celo_json = json.clone();
        non_celo_json["chainId"] = serde_json::json!("0x1");
        let non_celo_tx: AnyRpcTransaction = serde_json::from_value(non_celo_json).unwrap();
        assert!(TxEnv::from_any_rpc_transaction(&non_celo_tx).is_err());

        let any_tx: AnyRpcTransaction = serde_json::from_value(json).unwrap();

        let tx_env = TxEnv::from_any_rpc_transaction(&any_tx).unwrap();

        assert_eq!(tx_env.tx_type, CELO_DYNAMIC_FEE_TX_TYPE);
        assert_eq!(tx_env.caller, from);
        assert_eq!(tx_env.nonce, 42);
        assert_eq!(tx_env.gas_limit, 21000);
        assert_eq!(tx_env.gas_price, 3);
        assert_eq!(tx_env.gas_priority_fee, Some(1));
        assert_eq!(tx_env.kind, TxKind::Call(to));
        assert_eq!(tx_env.value, U256::from(101));
        assert_eq!(tx_env.data, Bytes::from_static(&[0x12, 0x34]));
        assert_eq!(tx_env.chain_id, Some(42_220));
    }

    #[test]
    fn from_any_rpc_transaction_for_tempo_eth_envelope() {
        let from = Address::random();
        let signed_tx = make_signed_eip1559();
        let tempo_envelope = TempoTxEnvelope::Eip1559(signed_tx.clone());
        let any_tx = eth_rpc_transaction(from);

        let tx_env = TempoTxEnv::from_any_rpc_transaction(&any_tx).unwrap();
        assert_eq!(tx_env.inner.caller, from);
        assert_eq!(tx_env.inner.nonce, 42);
        assert_eq!(tx_env.inner.gas_limit, 21001);
        assert_eq!(tx_env.inner.value, U256::from(101));
        assert_eq!(tx_env.fee_token, None);
        assert_eq!(
            tx_env.execution_context(),
            ExecutionContext::Transaction { tx_hash: *signed_tx.hash() }
        );
        assert_eq!(tx_env.unique_tx_identifier(), Some(tempo_envelope.unique_tx_identifier(from)));
        assert!(tx_env.tempo_tx_env.is_none());
    }

    #[test]
    fn from_any_rpc_transaction_for_tempo_aa() {
        let sender = PrivateKeySigner::random();
        let sponsor = PrivateKeySigner::random();
        let fee_token = Some(Address::random());
        let calls = vec![
            Call {
                to: TxKind::Call(Address::with_last_byte(0x11)),
                value: U256::ZERO,
                input: Bytes::from_static(&[0xaa, 0xbb]),
            },
            Call {
                to: TxKind::Call(Address::with_last_byte(0x22)),
                value: U256::ZERO,
                input: Bytes::from_static(&[0xcc, 0xdd]),
            },
        ];
        let mut tempo_tx = TempoTransaction {
            chain_id: 42431,
            nonce: 42,
            gas_limit: 424242,
            fee_token,
            nonce_key: U256::from(4242),
            valid_after: NonZeroU64::new(1800000000),
            valid_before: NonZeroU64::new(1900000000),
            calls: calls.clone(),
            ..Default::default()
        };
        tempo_tx.fee_payer_signature = Some(
            sponsor.sign_hash_sync(&tempo_tx.fee_payer_signature_hash(sender.address())).unwrap(),
        );
        let signature = sender.sign_hash_sync(&tempo_tx.signature_hash()).unwrap();
        let aa_signed = tempo_tx.into_signed(signature.into());
        let tx_hash = *aa_signed.hash();
        let unique_tx_identifier = aa_signed.expiring_nonce_hash(sender.address());

        // Round-trip a Tempo RPC transaction through JSON, as `AnyNetwork` providers receive it.
        let rpc_tx = RpcTransaction::from_transaction(
            Recovered::new_unchecked(TempoTxEnvelope::AA(aa_signed), sender.address()),
            TransactionInfo::default(),
        );
        let any_tx: AnyRpcTransaction =
            serde_json::from_value(serde_json::to_value(&rpc_tx).unwrap()).unwrap();
        assert!((*any_tx.inner.inner).is_unknown());

        let tx_env = TempoTxEnv::from_any_rpc_transaction(&any_tx).unwrap();
        assert_eq!(tx_env.inner.tx_type, TEMPO_TX_TYPE_ID);
        assert_eq!(tx_env.inner.caller, sender.address());
        assert_eq!(tx_env.inner.nonce, 42);
        assert_eq!(tx_env.inner.gas_limit, 424242);
        assert_eq!(tx_env.inner.chain_id, Some(42431));
        assert_eq!(tx_env.fee_token, fee_token);
        assert_eq!(tx_env.fee_payer, Some(Some(sponsor.address())));
        assert_eq!(tx_env.execution_context(), ExecutionContext::Transaction { tx_hash });
        assert_eq!(tx_env.unique_tx_identifier(), Some(unique_tx_identifier));

        let aa = tx_env.tempo_tx_env.as_deref().unwrap();
        assert_eq!(aa.aa_calls, calls);
        assert_eq!(aa.nonce_key, U256::from(4242));
        assert_eq!(aa.valid_after, Some(1800000000));
        assert_eq!(aa.valid_before, Some(1900000000));
        assert_eq!(aa.tx_hash, tx_hash);
    }

    /// Decodes a captured Tempo `eth_getTransactionByHash` result.
    fn captured_tempo_aa(json: &str) -> TempoTxEnv {
        let any_tx: AnyRpcTransaction = serde_json::from_str(json).unwrap();
        let tx_env = TempoTxEnv::from_any_rpc_transaction(&any_tx).unwrap();
        let aa = tx_env.tempo_tx_env.as_deref().unwrap();
        // Recovery over the recomputed signing hash only succeeds if every signed field decoded.
        assert_eq!(aa.signature.recover_signer(&aa.signature_hash).unwrap(), tx_env.inner.caller);
        assert_eq!(tx_env.inner.tx_type, TEMPO_TX_TYPE_ID);
        tx_env
    }

    #[test]
    fn from_any_rpc_transaction_for_captured_sponsored_tempo_aa_batch() {
        let tx_env =
            captured_tempo_aa(include_str!("../../test-data/tempo-aa-sponsored-batch.json"));
        assert_eq!(tx_env.inner.caller, address!("0x0a0d9bc4dda3a659699ce05153ffe9298f99ce89"));
        assert_eq!(tx_env.inner.nonce, 0);
        assert_eq!(tx_env.inner.gas_limit, 1403789);
        assert_eq!(tx_env.inner.chain_id, Some(4217));
        assert_eq!(tx_env.fee_token, Some(address!("0x20c0000000000000000000006a37da5c996874be")));
        assert_eq!(
            tx_env.fee_payer,
            Some(Some(address!("0x58aa7ce42e1d13b2919e2ac7e006c4fbc171442c")))
        );
        assert_eq!(
            tx_env.execution_context(),
            ExecutionContext::Transaction {
                tx_hash: b256!(
                    "0xbb23551c35bc2c2539ac3637c7464a22b5fb38bbb9cf39fa1f533ca6c2c981a3"
                )
            }
        );
        // Expiring nonces are replay-protected by this sender-scoped identifier.
        assert_eq!(
            tx_env.channel_open_context_hash(),
            Some(b256!("0xadb35ee9830a691a8dbd8f42208a1553d9a242b8750a218d841cc78a0dea20ca"))
        );

        let aa = tx_env.tempo_tx_env.as_deref().unwrap();
        assert_eq!(
            aa.aa_calls,
            [
                Call {
                    to: TxKind::Call(address!("0x20c0000000000000000000000000000000000000")),
                    value: U256::ZERO,
                    input: bytes!(
                        "0x095ea7b300000000000000000000000083a1491f3e7f8daab8f787a631334b9ca7a870230000000000000000000000000000000000000000000000000000000077359400"
                    ),
                },
                Call {
                    to: TxKind::Call(address!("0x83a1491f3e7f8daab8f787a631334b9ca7a87023")),
                    value: U256::ZERO,
                    input: bytes!(
                        "0x6e553f6500000000000000000000000000000000000000000000000000000000773594000000000000000000000000000a0d9bc4dda3a659699ce05153ffe9298f99ce89"
                    ),
                },
            ]
        );
        assert_eq!(aa.nonce_key, U256::MAX);
        assert_eq!(aa.valid_after, Some(542204929));
        assert_eq!(aa.valid_before, Some(1790813692));
    }

    #[test]
    fn from_any_rpc_transaction_for_captured_nonce_lane_tempo_aa() {
        let tx_env = captured_tempo_aa(include_str!("../../test-data/tempo-aa-nonce-lane.json"));
        assert_eq!(tx_env.inner.caller, address!("0xfdd1c606b498f5fcaaf27bd318b14caf52e8f6c2"));
        assert_eq!(tx_env.inner.nonce, 22496);
        assert_eq!(tx_env.inner.gas_limit, 99408);
        assert_eq!(tx_env.inner.chain_id, Some(42431));
        assert_eq!(tx_env.fee_token, None);
        assert_eq!(tx_env.fee_payer, None);
        assert_eq!(
            tx_env.execution_context(),
            ExecutionContext::Transaction {
                tx_hash: b256!(
                    "0x4df0a9b9e859be6c91c00edf8478939d0b91f6f9eb39dd9c8a93946ac70ff740"
                )
            }
        );

        let aa = tx_env.tempo_tx_env.as_deref().unwrap();
        let [call] = aa.aa_calls.as_slice() else { panic!("expected one call") };
        assert_eq!(call.to, TxKind::Call(address!("0x5ad0000000000000000000000000000000000003")));
        assert_eq!(call.value, U256::ZERO);
        assert_eq!(call.input.len(), 868);
        assert_eq!(aa.nonce_key, U256::ONE);
        assert_eq!(aa.valid_after, None);
        assert_eq!(aa.valid_before, None);
    }

    #[test]
    fn from_any_rpc_transaction_for_captured_keychain_tempo_aa() {
        let tx_env = captured_tempo_aa(include_str!("../../test-data/tempo-aa-keychain.json"));
        let sender = address!("0x5f704c6c7075acd14ee36527f03b5b5dcb4a966f");
        let key_id = address!("0x240a31713e851acdc0c590fe45ef72baeff55f91");
        assert_eq!(tx_env.inner.caller, sender);
        assert_eq!(tx_env.inner.nonce, 0);
        assert_eq!(tx_env.inner.gas_limit, 322821);
        assert_eq!(tx_env.inner.chain_id, Some(42431));
        assert_eq!(tx_env.fee_token, None);
        assert_eq!(
            tx_env.fee_payer,
            Some(Some(address!("0x133d7736f290fa2758cf0d1e0862f5a93ae1dcbf")))
        );
        assert_eq!(
            tx_env.execution_context(),
            ExecutionContext::Transaction {
                tx_hash: b256!(
                    "0xd4d372ad5cfa4cdf476aeb25d4a76fb80fe2d6169eb9354c544500c0496a19d6"
                )
            }
        );
        // Expiring nonces are replay-protected by this sender-scoped identifier.
        assert_eq!(
            tx_env.channel_open_context_hash(),
            Some(b256!("0xdb2722c83d8893c1637bbabfe2ad924bcc044653f6c9fa91d472d70ddfa97de3"))
        );

        let aa = tx_env.tempo_tx_env.as_deref().unwrap();
        assert_eq!(
            aa.aa_calls,
            [Call { to: TxKind::Call(Address::ZERO), value: U256::ZERO, input: Bytes::new() }]
        );
        assert_eq!(aa.nonce_key, U256::MAX);
        assert_eq!(aa.valid_after, Some(949691873));
        assert_eq!(aa.valid_before, Some(1790811579));
        let TempoSignature::Keychain(keychain) = &aa.signature else { panic!("expected keychain") };
        assert_eq!(keychain.user_address, sender);
        assert_eq!(keychain.key_id(&aa.signature_hash).unwrap(), key_id);
        let key_authorization = aa.key_authorization.as_ref().unwrap();
        assert_eq!(key_authorization.key_id, key_id);
        assert_eq!(key_authorization.recover_signer().unwrap(), sender);
    }

    #[test]
    fn from_any_rpc_transaction_for_captured_authorization_list_tempo_aa() {
        let tx_env =
            captured_tempo_aa(include_str!("../../test-data/tempo-aa-authorization-list.json"));
        assert_eq!(tx_env.inner.caller, address!("0xd410bcea80de9214e0e49df7d19c4c5e3db60f2a"));
        assert_eq!(tx_env.inner.nonce, 0);
        assert_eq!(tx_env.inner.gas_limit, 2000000);
        assert_eq!(tx_env.inner.chain_id, Some(42431));
        assert_eq!(tx_env.fee_token, Some(address!("0x20c0000000000000000000000000000000000000")));
        assert_eq!(tx_env.fee_payer, None);
        assert_eq!(
            tx_env.execution_context(),
            ExecutionContext::Transaction {
                tx_hash: b256!(
                    "0xfff5a6fa344fa9d6d7410374f40b6f711735500892cccdf480670fd7c5adcf93"
                )
            }
        );

        let aa = tx_env.tempo_tx_env.as_deref().unwrap();
        let call = Call {
            to: TxKind::Call(address!("0xa98d41b22c17ae4f4a94420a29a3095464c037e3")),
            value: U256::ZERO,
            input: Bytes::new(),
        };
        assert_eq!(aa.aa_calls, [call.clone(), call]);
        assert_eq!(aa.nonce_key, U256::ZERO);
        // Secp256k1, P256 and WebAuthn authorizations, in that order.
        let delegate = address!("0xaaaaaaaa00000000000000000000000000000000");
        assert_eq!(
            aa.tempo_authorization_list
                .iter()
                .map(|auth| (auth.address, auth.authority()))
                .collect::<Vec<_>>(),
            [
                (delegate, Some(address!("0xd9aa283bc5643587f9623a0e9683df78588404a8"))),
                (delegate, Some(address!("0x883b644ccaa219845187fd0932a8ef6e7d484bd6"))),
                (delegate, Some(address!("0x732c32ec8e029d8989b9d1db9914376b17e20676"))),
            ]
        );
    }

    #[test]
    fn from_any_rpc_transaction_for_tempo_aa_without_signature_errors() {
        let mut json: serde_json::Value =
            serde_json::from_str(include_str!("../../test-data/tempo-aa-sponsored-batch.json"))
                .unwrap();
        json.as_object_mut().unwrap().remove("signature");
        let any_tx: AnyRpcTransaction = serde_json::from_value(json).unwrap();

        let err = TempoTxEnv::from_any_rpc_transaction(&any_tx).unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot decode Tempo AA transaction: missing field `signature`"
        );
    }

    #[cfg(feature = "optimism")]
    mod optimism {
        use super::*;
        use alloy_consensus::Sealed;
        use alloy_eips::eip2718::Encodable2718;
        use alloy_op_evm::OpTx;
        use op_alloy_consensus::{OpTxEnvelope, TxDeposit, transaction::OpTransactionInfo};
        use op_alloy_rpc_types::Transaction as OpRpcTransaction;

        #[test]
        fn from_any_rpc_transaction_for_op() {
            let from = Address::random();
            let any_tx = eth_rpc_transaction(from);
            let expected_base = TxEnv::from_any_rpc_transaction(&any_tx).unwrap();

            let op_tx_env = OpTx::from_any_rpc_transaction(&any_tx).unwrap();
            assert_eq!(op_tx_env.base, expected_base);
            // op-revm charges the L1 data fee off these bytes and rejects a non-deposit
            // transaction that arrives without them.
            assert_eq!(
                op_tx_env.enveloped_tx,
                Some(any_tx.as_envelope().unwrap().encoded_2718().into())
            );
        }

        #[test]
        fn from_any_rpc_transaction_for_op_deposit() {
            let from = Address::random();
            let source_hash = B256::random();
            let deposit = TxDeposit {
                source_hash,
                from,
                to: TxKind::Call(Address::with_last_byte(0xCC)),
                mint: 1111,
                value: U256::from(200),
                gas_limit: 21000,
                is_system_transaction: true,
                input: Default::default(),
            };

            // Build a concrete OpRpcTransaction, serialize to JSON, deserialize as
            // AnyRpcTransaction.
            let op_rpc_tx = OpRpcTransaction::from_transaction(
                Recovered::new_unchecked(OpTxEnvelope::Deposit(Sealed::new(deposit)), from),
                OpTransactionInfo::default(),
            );
            let json = serde_json::to_value(&op_rpc_tx).unwrap();
            let any_tx: AnyRpcTransaction = serde_json::from_value(json).unwrap();

            let op_tx_env = OpTx::from_any_rpc_transaction(&any_tx).unwrap();
            assert_eq!(op_tx_env.base.caller, from);
            assert_eq!(op_tx_env.base.kind, TxKind::Call(Address::with_last_byte(0xCC)));
            assert_eq!(op_tx_env.base.value, U256::from(200));
            assert_eq!(op_tx_env.base.gas_limit, 21000);
            assert_eq!(op_tx_env.deposit.source_hash, source_hash);
            assert_eq!(op_tx_env.deposit.mint, Some(1111));
            assert!(op_tx_env.deposit.is_system_transaction);
        }
    }

    fn eth_rpc_transaction(from: Address) -> AnyRpcTransaction {
        RpcTransaction::from_transaction(
            Recovered::new_unchecked(make_signed_eip1559().into(), from),
            TransactionInfo::default(),
        )
        .into()
    }
}
