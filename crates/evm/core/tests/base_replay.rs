//! Base replay over evm2: recorded mainnet transfers reproduce their wire fees and state deltas,
//! deposits mint and fail as on chain, and EIP-8130 transactions convert from fork RPC responses.

use alloy_consensus::{Sealable, transaction::Recovered};
use alloy_eips::eip2718::Encodable2718;
use alloy_network::{AnyRpcTransaction, AnyTxEnvelope, UnknownTxEnvelope};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, hex, keccak256};
use alloy_rpc_types::Transaction as RpcTransaction;
use alloy_serde::WithOtherFields;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use base_common_consensus::{
    BaseTransactionInfo, BaseTxEnvelope, Eip8130Signed, Predeploys, TxDeposit, TxEip8130,
};
use base_common_rpc_types::Transaction;
use evm2::{
    NoopInspector,
    bytecode::Bytecode,
    env::BlockEnv,
    evm::{AccountInfo, EmptyDB},
};
use foundry_evm_core::{
    backend::Backend,
    evm::{
        BaseEvmNetwork, EvmEnv, EvmEnvFor, FoundryEvmNetwork, FoundryTxResultExt, TxPayload,
        TypesFor,
    },
    utils::get_blob_base_fee_update_fraction,
};
use foundry_evm_hardforks::BaseUpgrade;
use serde::Deserialize;
use std::str::FromStr;

const AZUL_FIXTURE: &str = include_str!("fixtures/base/azul-transfer.json");
const BERYL_FIXTURE: &str = include_str!("fixtures/base/beryl-transfer.json");
const JOVIAN_FIXTURE: &str = include_str!("fixtures/base/jovian-transfer.json");

#[derive(Debug, Deserialize)]
struct ReplayFixture {
    chain_id: String,
    upgrade: String,
    block: FixtureBlock,
    transaction: FixtureTransaction,
    parent_state: FixtureParentState,
    expected: FixtureExpected,
}

#[derive(Debug, Deserialize)]
struct FixtureBlock {
    number: String,
    timestamp: String,
    gas_limit: String,
    base_fee_per_gas: String,
    beneficiary: String,
    prevrandao: String,
    excess_blob_gas: String,
    receipts_root: String,
    state_root: String,
}

#[derive(Debug, Deserialize)]
struct FixtureTransaction {
    hash: String,
    raw: String,
    from: String,
    to: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct FixtureParentState {
    caller_balance: String,
    caller_nonce: String,
    recipient_balance: String,
    recipient_code: String,
    l1_block_info_storage: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
struct FixtureExpected {
    status: bool,
    gas_used: String,
    effective_gas_price: String,
    l1_fee: String,
    log_count: usize,
}

fn fixture(data: &str) -> ReplayFixture {
    serde_json::from_str(data).expect("valid Base replay fixture")
}

fn u256(value: &str) -> U256 {
    U256::from_str(value).expect("valid U256 fixture value")
}

fn u64_hex(value: &str) -> u64 {
    u64::from_str_radix(value.trim_start_matches("0x"), 16).expect("valid u64 fixture value")
}

fn bytes(value: &str) -> Bytes {
    hex::decode(value.trim_start_matches("0x")).expect("valid byte fixture").into()
}

fn env(
    chain_id: u64,
    upgrade: BaseUpgrade,
    block: BlockEnv<TypesFor<BaseEvmNetwork>>,
) -> EvmEnvFor<BaseEvmNetwork> {
    let factory = <BaseEvmNetwork as FoundryEvmNetwork>::EvmFactory::default();
    let spec = base_evm2::BaseSpecId::new(upgrade);
    EvmEnv::new(spec, factory.version(spec, chain_id), block)
}

fn fixture_env(fixture: &ReplayFixture, upgrade: BaseUpgrade) -> EvmEnvFor<BaseEvmNetwork> {
    let chain_id = u64_hex(&fixture.chain_id);
    let timestamp = u64_hex(&fixture.block.timestamp);
    let block = BlockEnv::<TypesFor<BaseEvmNetwork>> {
        number: u256(&fixture.block.number),
        timestamp: U256::from(timestamp),
        gas_limit: U256::from(u64_hex(&fixture.block.gas_limit)),
        basefee: U256::from(u64_hex(&fixture.block.base_fee_per_gas)),
        beneficiary: fixture.block.beneficiary.parse().unwrap(),
        prevrandao: fixture.block.prevrandao.parse().unwrap(),
        ..Default::default()
    };
    let mut env = env(chain_id, upgrade, block);
    env.set_blob_excess_gas(
        u64_hex(&fixture.block.excess_blob_gas),
        get_blob_base_fee_update_fraction(chain_id, timestamp),
    );
    env
}

fn seed_fixture_backend(fixture: &ReplayFixture, upgrade: BaseUpgrade) -> Backend<BaseEvmNetwork> {
    let mut backend = Backend::with_db(EmptyDB::default(), fixture_env(fixture, upgrade));
    let caller = fixture.transaction.from.parse().unwrap();
    let recipient = fixture.transaction.to.parse().unwrap();
    backend.insert_account_info(
        caller,
        AccountInfo::default()
            .with_balance(u256(&fixture.parent_state.caller_balance))
            .with_nonce(u64_hex(&fixture.parent_state.caller_nonce)),
    );
    assert_eq!(fixture.parent_state.recipient_code, "0x");
    backend.insert_account_info(
        recipient,
        AccountInfo::default().with_balance(u256(&fixture.parent_state.recipient_balance)),
    );
    backend.insert_account_info(Predeploys::L1_BLOCK_INFO, AccountInfo::default());
    for (slot, value) in &fixture.parent_state.l1_block_info_storage {
        backend.insert_account_storage(Predeploys::L1_BLOCK_INFO, u256(slot), u256(value));
    }
    backend
}

fn balance(backend: &Backend<BaseEvmNetwork>, address: Address) -> U256 {
    backend.basic(address).unwrap().unwrap_or_default().balance
}

fn nonce(backend: &Backend<BaseEvmNetwork>, address: Address) -> u64 {
    backend.basic(address).unwrap().unwrap_or_default().nonce
}

fn replay_transfer_fixture(fixture: ReplayFixture) {
    let upgrade = BaseUpgrade::from_str(&fixture.upgrade).expect("valid Base fixture upgrade");
    assert_ne!(fixture.block.receipts_root.parse::<B256>().unwrap(), B256::ZERO);
    assert_ne!(fixture.block.state_root.parse::<B256>().unwrap(), B256::ZERO);
    let raw = bytes(&fixture.transaction.raw);
    assert_eq!(keccak256(&raw), fixture.transaction.hash.parse::<B256>().unwrap());
    let tx = BaseEvmNetwork::decode_tx(&raw).expect("decode Base transaction");
    assert_eq!(tx.signer(), fixture.transaction.from.parse::<Address>().unwrap());
    let mut backend = seed_fixture_backend(&fixture, upgrade);

    let result =
        backend.inspect_commit(&tx, NoopInspector::default()).expect("replay Base transfer");
    assert_eq!(result.status, fixture.expected.status);
    assert_eq!(result.tx_gas_used(), u64_hex(&fixture.expected.gas_used));
    assert_eq!(result.logs.len(), fixture.expected.log_count);

    let caller = fixture.transaction.from.parse().unwrap();
    let recipient = fixture.transaction.to.parse().unwrap();
    let gas_fee = u256(&fixture.expected.effective_gas_price) * U256::from(result.tx_gas_used());
    let expected_caller = u256(&fixture.parent_state.caller_balance)
        - u256(&fixture.transaction.value)
        - gas_fee
        - u256(&fixture.expected.l1_fee);
    assert_eq!(balance(&backend, caller), expected_caller);
    assert_eq!(
        balance(&backend, recipient),
        u256(&fixture.parent_state.recipient_balance) + u256(&fixture.transaction.value)
    );
    assert_eq!(nonce(&backend, caller), u64_hex(&fixture.parent_state.caller_nonce) + 1);
    assert_eq!(balance(&backend, Predeploys::L1_FEE_VAULT), u256(&fixture.expected.l1_fee));
    assert_eq!(
        balance(&backend, Predeploys::BASE_FEE_VAULT),
        u256(&fixture.block.base_fee_per_gas) * U256::from(result.tx_gas_used())
    );
    assert_eq!(balance(&backend, Predeploys::OPERATOR_FEE_VAULT), U256::ZERO);
}

#[test]
fn replays_azul_transfer_with_wire_fees_and_state_deltas() {
    replay_transfer_fixture(fixture(AZUL_FIXTURE));
}

#[test]
fn replays_jovian_transfer_with_wire_fees_and_state_deltas() {
    replay_transfer_fixture(fixture(JOVIAN_FIXTURE));
}

#[test]
fn replays_beryl_transfer_with_wire_fees_and_state_deltas() {
    replay_transfer_fixture(fixture(BERYL_FIXTURE));
}

fn deposit_envelope(to: Address) -> BaseTxEnvelope {
    BaseTxEnvelope::Deposit(
        TxDeposit {
            source_hash: B256::repeat_byte(0x11),
            from: Address::repeat_byte(0xaa),
            to: TxKind::Call(to),
            mint: 1_000,
            value: U256::from(600),
            gas_limit: 100_000,
            is_system_transaction: false,
            input: Bytes::new(),
        }
        .seal_slow(),
    )
}

fn simple_backend() -> Backend<BaseEvmNetwork> {
    let block = BlockEnv::<TypesFor<BaseEvmNetwork>> {
        number: U256::ONE,
        timestamp: U256::ONE,
        gas_limit: U256::from(30_000_000),
        basefee: U256::from(1_000_000),
        ..Default::default()
    };
    Backend::with_db(EmptyDB::default(), env(8453, BaseUpgrade::Beryl, block))
}

#[test]
fn replays_successful_and_failed_base_deposits() {
    let sender = Address::repeat_byte(0xaa);
    let recipient = Address::repeat_byte(0xbb);
    let envelope = deposit_envelope(recipient);
    assert_eq!(envelope.encoded_2718()[0], 0x7e);

    let mut backend = simple_backend();
    let tx = BaseEvmNetwork::evm_tx(envelope, sender, TxPayload::Signed).unwrap();
    let result = backend.inspect_commit(&tx, NoopInspector::default()).expect("execute deposit");
    assert!(result.status);
    assert_eq!(result.tx_gas_used(), 21_000);
    assert_eq!(balance(&backend, sender), U256::from(400));
    assert_eq!(balance(&backend, recipient), U256::from(600));
    assert_eq!(balance(&backend, Predeploys::L1_FEE_VAULT), U256::ZERO);

    let reverter = Address::repeat_byte(0xcc);
    let mut backend = simple_backend();
    backend.insert_account_info(
        reverter,
        AccountInfo::default().with_code(Bytecode::new_raw(Bytes::from_static(&[0xfe]))),
    );
    let tx = BaseEvmNetwork::evm_tx(deposit_envelope(reverter), sender, TxPayload::Signed).unwrap();
    let result =
        backend.inspect_commit(&tx, NoopInspector::default()).expect("execute failed deposit");
    assert!(!result.status && result.ext.is_failed_deposit());
    assert_eq!(result.tx_gas_used(), 100_000);
    assert_eq!(balance(&backend, sender), U256::from(1_000));
    assert_eq!(balance(&backend, reverter), U256::ZERO);
    assert_eq!(nonce(&backend, sender), 1);
}

fn eip8130_envelope(signer: &PrivateKeySigner) -> BaseTxEnvelope {
    let tx = TxEip8130 {
        chain_id: 8453,
        sender: None,
        nonce_key: U256::ZERO,
        nonce_sequence: 0,
        valid_after: 0,
        valid_before: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 1_000_000_000,
        gas_limit: 200_000,
        account_changes: Vec::new(),
        calls: Vec::new(),
        metadata: Bytes::new(),
        payer: None,
    };
    let signature = signer.sign_hash_sync(&tx.sender_signature_hash()).unwrap();
    BaseTxEnvelope::Eip8130(Eip8130Signed::new(tx, signature.as_bytes().into(), Bytes::new()))
}

#[test]
fn converts_eip8130_from_any_rpc_transaction() {
    let signer = PrivateKeySigner::from_bytes(&B256::with_last_byte(2)).unwrap();
    let envelope = eip8130_envelope(&signer);
    let hash = *envelope.tx_hash();
    let rpc_tx = Transaction::from_transaction(
        Recovered::new_unchecked(envelope.clone(), signer.address()),
        BaseTransactionInfo::default(),
    );
    let mut rpc_value = serde_json::to_value(rpc_tx).unwrap();
    rpc_value
        .as_object_mut()
        .unwrap()
        .insert("hash".to_string(), serde_json::to_value(hash).unwrap());
    let unknown = serde_json::from_value::<UnknownTxEnvelope>(rpc_value).unwrap();
    let any_tx = RpcTransaction::from_transaction(
        Recovered::new_unchecked(AnyTxEnvelope::Unknown(unknown), signer.address()),
        Default::default(),
    );
    let any = AnyRpcTransaction::new(WithOtherFields::new(any_tx));

    let converted = BaseEvmNetwork::envelope_from_any_rpc(&any).unwrap();
    assert_eq!(converted.encoded_2718(), envelope.encoded_2718());
    assert_eq!(BaseEvmNetwork::tx_from_any_rpc(&any).unwrap().signer(), signer.address());
}
