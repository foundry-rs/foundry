//! Tempo execution over evm2: nested transactions keep their halt reason, keychain spending
//! limits settle storage credits as on chain, and spec changes keep Tempo's gas schedule.

use alloy_consensus::transaction::Recovered;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolCall;
use evm2::{
    Evm, SpecId, Version,
    bytecode::Bytecode,
    evm::{AccountInfo, InMemoryDB},
    interpreter::InstrStop,
};
use foundry_evm_core::{
    constants::TEST_CONTRACT_ADDRESS,
    evm::{
        EvmEnv, ExecutionLifecycle, FoundryEvmFactory, FoundryEvmNetwork, TempoEvmNetwork,
        TxEnvelopeFor, TxPayload,
    },
};
use tempo_alloy::primitives::{
    TempoSignature, TempoTransaction,
    transaction::{Call, KeychainSignature, PrimitiveSignature, calc_gas_balance_spending},
};
use tempo_evm::{TempoEvmFactory, TempoEvmTypes, TempoTxEnv, tempo_execution_config};
use tempo_hardfork::TempoHardfork;
use tempo_precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, DEFAULT_FEE_TOKEN,
    account_keychain::{
        KeyRestrictions, SignatureType as KeychainSignatureType, TokenLimit as KeychainTokenLimit,
        authorizeKeyCall,
    },
    storage::StorageCtx,
    storage_credits::StorageCredits,
    tip20::{ITIP20, TIP20Token},
};

const GAS_LIMIT: u64 = 500_000;
const GAS_PRICE: u128 = 1_000_000_000_000;
const TRANSFER_AMOUNT: u64 = 1_234;
const CHAIN_ID: u64 = 1;

fn tempo_env(spec: TempoHardfork) -> EvmEnv<TempoEvmTypes> {
    EvmEnv::new(spec, TempoEvmFactory::default().version(spec, CHAIN_ID), Default::default())
}

/// Funds `account` with enough of the default fee token to pay a full `GAS_LIMIT` at
/// `GAS_PRICE` and transfer `TRANSFER_AMOUNT`, and gives `recipient` a non-zero balance.
fn seed_fee_token_balances(evm: &mut Evm<'_, TempoEvmTypes>, account: Address, recipient: Address) {
    let amount = calc_gas_balance_spending(GAS_LIMIT, GAS_PRICE) + U256::from(TRANSFER_AMOUNT);
    StorageCtx::enter_evm(evm, || {
        let mut token = TIP20Token::from_address_unchecked(DEFAULT_FEE_TOKEN);
        token.mint(TEST_CONTRACT_ADDRESS, ITIP20::mintCall { to: account, amount }).unwrap();
        token
            .mint(TEST_CONTRACT_ADDRESS, ITIP20::mintCall { to: recipient, amount: U256::ONE })
            .unwrap();
    });
    evm.state_mut().commit_transaction();
}

fn storage_credit_balance(evm: &mut Evm<'_, TempoEvmTypes>, owner: Address) -> u64 {
    StorageCtx::enter_evm(evm, || StorageCredits::new().balance_of(owner).unwrap())
}

fn signed_tx(
    envelope: impl Into<TxEnvelopeFor<TempoEvmNetwork>>,
    sender: Address,
) -> Recovered<TempoTxEnv> {
    TempoEvmNetwork::evm_tx(envelope.into(), sender, TxPayload::Signed).unwrap()
}

fn primitive_tx(account: &PrivateKeySigner, tx: TempoTransaction) -> Recovered<TempoTxEnv> {
    let signature = account.sign_hash_sync(&tx.signature_hash()).unwrap();
    signed_tx(tx.into_signed(signature.into()), account.address())
}

#[test]
fn nested_tempo_execution_preserves_halt_reason() {
    let account = PrivateKeySigner::random();
    let target = Address::repeat_byte(0xee);

    for (opcode, expected) in [(0x0c, InstrStop::OpcodeNotFound), (0x50, InstrStop::StackUnderflow)]
    {
        let mut db = InMemoryDB::default();
        let mut info =
            AccountInfo::default().with_code(Bytecode::new_raw(Bytes::from(vec![opcode])));
        db.insert_contract(&mut info);
        db.insert_account_info(&target, info);
        let mut lifecycle =
            ExecutionLifecycle::new(TempoEvmFactory::default(), db, tempo_env(TempoHardfork::T11));
        let mut evm = lifecycle.create_evm();
        seed_fee_token_balances(&mut evm, account.address(), target);
        let tx = primitive_tx(
            &account,
            TempoTransaction {
                chain_id: CHAIN_ID,
                fee_token: Some(DEFAULT_FEE_TOKEN),
                gas_limit: GAS_LIMIT,
                calls: vec![Call {
                    to: TxKind::Call(target),
                    value: U256::ZERO,
                    input: Bytes::new(),
                }],
                ..Default::default()
            },
        );

        let executed = lifecycle
            .prepare_nested(&mut evm, tx, None, None)
            .execute(&mut evm, None, false)
            .expect("nested transaction executes");
        let result = lifecycle.store_executed(executed);
        assert!(!result.status);
        assert_eq!(result.stop, expected);
    }
}

fn keychain_spend_setup() -> (Address, Address, Recovered<TempoTxEnv>, Recovered<TempoTxEnv>) {
    let root = PrivateKeySigner::random();
    let access_key = PrivateKeySigner::random();
    let recipient = Address::repeat_byte(0xee);
    let spending_limit =
        calc_gas_balance_spending(GAS_LIMIT, GAS_PRICE) + U256::from(TRANSFER_AMOUNT);

    let authorize = authorizeKeyCall {
        keyId: access_key.address(),
        signatureType: KeychainSignatureType::Secp256k1,
        config: KeyRestrictions {
            expiry: u64::MAX,
            enforceLimits: true,
            limits: vec![KeychainTokenLimit {
                token: DEFAULT_FEE_TOKEN,
                amount: spending_limit,
                period: 0,
            }],
            allowAnyCalls: true,
            allowedCalls: vec![],
        },
    };
    let auth_tx = primitive_tx(
        &root,
        TempoTransaction {
            chain_id: CHAIN_ID,
            fee_token: Some(DEFAULT_FEE_TOKEN),
            gas_limit: 2_000_000,
            calls: vec![Call {
                to: TxKind::Call(ACCOUNT_KEYCHAIN_ADDRESS),
                value: U256::ZERO,
                input: authorize.abi_encode().into(),
            }],
            ..Default::default()
        },
    );

    let spend_tx = TempoTransaction {
        chain_id: CHAIN_ID,
        fee_token: Some(DEFAULT_FEE_TOKEN),
        max_priority_fee_per_gas: GAS_PRICE,
        max_fee_per_gas: GAS_PRICE,
        gas_limit: GAS_LIMIT,
        calls: vec![Call {
            to: TxKind::Call(DEFAULT_FEE_TOKEN),
            value: U256::ZERO,
            input: ITIP20::transferCall { to: recipient, amount: U256::from(TRANSFER_AMOUNT) }
                .abi_encode()
                .into(),
        }],
        nonce: 1,
        ..Default::default()
    };
    let keychain_hash = KeychainSignature::signing_hash(spend_tx.signature_hash(), root.address());
    let access_signature = access_key.sign_hash_sync(&keychain_hash).unwrap();
    let spend_tx = signed_tx(
        spend_tx.into_signed(TempoSignature::Keychain(KeychainSignature::new(
            root.address(),
            PrimitiveSignature::Secp256k1(access_signature),
        ))),
        root.address(),
    );
    (root.address(), recipient, auth_tx, spend_tx)
}

#[test]
fn foundry_factory_keychain_limit_refund_does_not_leak_storage_credit() {
    let (account, recipient, auth_tx, spend_tx) = keychain_spend_setup();
    let factory = TempoEvmFactory::default();
    let mut evm = FoundryEvmFactory::create_evm(
        &factory,
        InMemoryDB::default(),
        tempo_env(TempoHardfork::T7),
    );
    seed_fee_token_balances(&mut evm, account, recipient);

    let auth_result = evm.transact(&auth_tx).expect("auth transaction executes").commit();
    assert!(auth_result.status, "auth transaction should succeed: {auth_result:?}");

    let spend_result = evm.transact(&spend_tx).expect("spend transaction executes").commit();
    assert!(spend_result.status, "keychain spend should succeed: {spend_result:?}");
    assert_eq!(
        storage_credit_balance(&mut evm, ACCOUNT_KEYCHAIN_ADDRESS),
        0,
        "post-tx fee refund recreates the keychain spending-limit slot, so the same-tx clear credit must be canceled",
    );
}

#[test]
fn tempo_execution_spec_preserves_network_gas() {
    let factory = TempoEvmFactory::default();
    let mut evm = FoundryEvmFactory::create_evm(
        &factory,
        InMemoryDB::default(),
        tempo_env(TempoHardfork::default()),
    );
    for spec in [TempoHardfork::T3, TempoHardfork::T7, TempoHardfork::T14] {
        let env = EvmEnv::new(spec, *evm.version(), *evm.block());
        factory.set_env(&mut evm, env);
        assert_eq!(evm.config_spec_id(), spec);
        assert_eq!(
            evm.version().gas_params,
            tempo_execution_config(spec, CHAIN_ID).version().gas_params
        );
        assert_ne!(evm.version().gas_params, Version::new(SpecId::OSAKA).gas_params);
    }
}
