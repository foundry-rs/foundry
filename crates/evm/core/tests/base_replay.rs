use alloy_consensus::{
    Sealable,
    transaction::{Recovered, SignerRecoverable},
};
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_evm::{Evm, EvmEnv, FromRecoveredTx};
use alloy_network::{AnyRpcTransaction, AnyTxEnvelope, UnknownTxEnvelope};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, hex};
use alloy_serde::WithOtherFields;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use base_common_consensus::{
    BaseTransactionInfo, BaseTxEnvelope, Eip8130Signed, Predeploys, TxDeposit, TxEip8130,
};
use base_common_evm::{
    BaseEvmFactory, BaseHaltReason, BaseSpecId, BaseTransaction, BaseUpgrade, Eip8130ExecutionMode,
    L1BlockInfo,
};
use base_common_rpc_types::Transaction as BaseRpcTransaction;
use foundry_evm_core::{
    FoundryBlock, FoundryTransaction, FromAnyRpcTransaction,
    backend::Backend,
    evm::{BaseEvmNetwork, FoundryEvmFactory},
    fork::MultiFork,
    utils::get_blob_base_fee_update_fraction,
};
use revm::{
    Database,
    context::{BlockEnv, CfgEnv, TxEnv},
    inspector::NoOpInspector,
    state::{AccountInfo, Bytecode},
};
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
    l1_gas_used: String,
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

fn in_memory_base_backend() -> Backend<BaseEvmNetwork> {
    let (forks, _fork_handler) = MultiFork::new();
    Backend::new(forks, None).expect("in-memory Base backend")
}

fn seed_fixture_backend(fixture: &ReplayFixture) -> Backend<BaseEvmNetwork> {
    let mut db = in_memory_base_backend();
    let caller = fixture.transaction.from.parse().unwrap();
    let recipient = fixture.transaction.to.parse().unwrap();

    db.insert_account_info(
        caller,
        AccountInfo {
            balance: u256(&fixture.parent_state.caller_balance),
            nonce: u64_hex(&fixture.parent_state.caller_nonce),
            ..Default::default()
        },
    );
    assert_eq!(fixture.parent_state.recipient_code, "0x");
    db.insert_account_info(
        recipient,
        AccountInfo {
            balance: u256(&fixture.parent_state.recipient_balance),
            ..Default::default()
        },
    );
    db.insert_account_info(Predeploys::L1_BLOCK_INFO, AccountInfo::default());
    for (slot, value) in &fixture.parent_state.l1_block_info_storage {
        db.insert_account_storage(Predeploys::L1_BLOCK_INFO, u256(slot), u256(value))
            .expect("insert L1BlockInfo storage");
    }
    db
}

fn fixture_env(fixture: &ReplayFixture, upgrade: BaseUpgrade) -> EvmEnv<BaseSpecId, BlockEnv> {
    let chain_id = u64_hex(&fixture.chain_id);
    let timestamp = u64_hex(&fixture.block.timestamp);
    let mut cfg = CfgEnv::new_with_spec(BaseSpecId::new(upgrade));
    cfg.chain_id = chain_id;

    let mut block = BlockEnv::default();
    block.set_number(u256(&fixture.block.number));
    block.set_timestamp(U256::from(timestamp));
    block.set_gas_limit(u64_hex(&fixture.block.gas_limit));
    block.set_basefee(u64_hex(&fixture.block.base_fee_per_gas));
    block.set_beneficiary(fixture.block.beneficiary.parse().unwrap());
    block.set_prevrandao(Some(fixture.block.prevrandao.parse().unwrap()));
    block.set_blob_excess_gas_and_price(
        u64_hex(&fixture.block.excess_blob_gas),
        get_blob_base_fee_update_fraction(chain_id, timestamp),
    );
    EvmEnv::new(cfg, block)
}

fn decode_fixture_transaction(
    fixture: &ReplayFixture,
) -> (Bytes, BaseTxEnvelope, BaseTransaction<TxEnv>) {
    let raw = bytes(&fixture.transaction.raw);
    let envelope =
        BaseTxEnvelope::decode_2718(&mut raw.as_ref()).expect("decode Base transaction envelope");
    assert_eq!(envelope.encoded_2718(), raw);
    assert_eq!(*envelope.tx_hash(), fixture.transaction.hash.parse::<B256>().unwrap());

    let signer = envelope.recover_signer().expect("recover Base transaction signer");
    assert_eq!(signer, fixture.transaction.from.parse::<Address>().unwrap());
    let tx = BaseTransaction::from_recovered_tx(&envelope, signer);
    assert_eq!(tx.enveloped_tx(), Some(&raw));
    (raw, envelope, tx)
}

fn balance(db: &mut Backend<BaseEvmNetwork>, address: Address) -> U256 {
    db.basic(address).unwrap().unwrap_or_default().balance
}

fn nonce(db: &mut Backend<BaseEvmNetwork>, address: Address) -> u64 {
    db.basic(address).unwrap().unwrap_or_default().nonce
}

fn replay_transfer_fixture(fixture: ReplayFixture) {
    let upgrade = BaseUpgrade::from_str(&fixture.upgrade).expect("valid Base fixture upgrade");
    assert_ne!(fixture.block.receipts_root.parse::<B256>().unwrap(), B256::ZERO);
    assert_ne!(fixture.block.state_root.parse::<B256>().unwrap(), B256::ZERO);
    let (raw, _envelope, tx) = decode_fixture_transaction(&fixture);
    let mut db = seed_fixture_backend(&fixture);
    let spec = BaseSpecId::new(upgrade);
    let block_number = u256(&fixture.block.number);

    let mut l1_block_info =
        L1BlockInfo::try_fetch(&mut db, block_number, spec).expect("load L1BlockInfo");
    assert_eq!(l1_block_info.data_gas(&raw, spec), u256(&fixture.expected.l1_gas_used));
    assert_eq!(l1_block_info.calculate_tx_l1_cost(&raw, spec), u256(&fixture.expected.l1_fee));

    let mut evm = BaseEvmFactory::default().create_foundry_evm_with_inspector(
        &mut db,
        fixture_env(&fixture, upgrade),
        L1BlockInfo::default(),
        NoOpInspector,
    );
    let result = evm.transact_commit(tx).expect("replay Base transfer");
    assert_eq!(result.is_success(), fixture.expected.status);
    assert_eq!(result.tx_gas_used(), u64_hex(&fixture.expected.gas_used));
    assert_eq!(result.logs().len(), fixture.expected.log_count);
    drop(evm);

    let caller = fixture.transaction.from.parse().unwrap();
    let recipient = fixture.transaction.to.parse().unwrap();
    let gas_fee = u256(&fixture.expected.effective_gas_price) * U256::from(result.tx_gas_used());
    let expected_caller = u256(&fixture.parent_state.caller_balance)
        - u256(&fixture.transaction.value)
        - gas_fee
        - u256(&fixture.expected.l1_fee);
    assert_eq!(balance(&mut db, caller), expected_caller);
    assert_eq!(
        balance(&mut db, recipient),
        u256(&fixture.parent_state.recipient_balance) + u256(&fixture.transaction.value)
    );
    assert_eq!(nonce(&mut db, caller), u64_hex(&fixture.parent_state.caller_nonce) + 1);
    assert_eq!(balance(&mut db, Predeploys::L1_FEE_VAULT), u256(&fixture.expected.l1_fee));
    assert_eq!(
        balance(&mut db, Predeploys::BASE_FEE_VAULT),
        u256(&fixture.block.base_fee_per_gas) * U256::from(result.tx_gas_used())
    );
    assert_eq!(balance(&mut db, Predeploys::OPERATOR_FEE_VAULT), U256::ZERO);
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

#[test]
fn replays_beryl_operator_fee_charge_and_refund() {
    let fixture = fixture(BERYL_FIXTURE);
    let (_raw, _envelope, tx) = decode_fixture_transaction(&fixture);
    let mut db = seed_fixture_backend(&fixture);
    let mut evm = BaseEvmFactory::default().create_foundry_evm_with_inspector(
        &mut db,
        fixture_env(&fixture, BaseUpgrade::Beryl),
        L1BlockInfo::default(),
        NoOpInspector,
    );
    evm.ctx_mut().chain = L1BlockInfo {
        l2_block: Some(u256(&fixture.block.number)),
        operator_fee_scalar: Some(U256::from(2)),
        operator_fee_constant: Some(U256::from(5)),
        ..Default::default()
    };

    let result = evm.transact_commit(tx).expect("replay Base transfer with operator fee");
    assert!(result.is_success());
    let operator_fee = U256::from(result.tx_gas_used()) * U256::from(200) + U256::from(5);
    drop(evm);

    let caller = fixture.transaction.from.parse().unwrap();
    let expected_caller = u256(&fixture.parent_state.caller_balance)
        - u256(&fixture.transaction.value)
        - u256(&fixture.expected.effective_gas_price) * U256::from(result.tx_gas_used())
        - operator_fee;
    assert_eq!(balance(&mut db, caller), expected_caller);
    assert_eq!(balance(&mut db, Predeploys::OPERATOR_FEE_VAULT), operator_fee);
    assert_eq!(balance(&mut db, Predeploys::L1_FEE_VAULT), U256::ZERO);
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

fn simple_base_env(upgrade: BaseUpgrade) -> EvmEnv<BaseSpecId, BlockEnv> {
    let mut cfg = CfgEnv::new_with_spec(BaseSpecId::new(upgrade));
    cfg.chain_id = 8453;
    let mut block = BlockEnv::default();
    block.set_number(U256::ONE);
    block.set_timestamp(U256::ONE);
    block.set_gas_limit(30_000_000);
    block.set_basefee(1_000_000);
    EvmEnv::new(cfg, block)
}

#[test]
fn replays_successful_and_failed_base_deposits() {
    let sender = Address::repeat_byte(0xaa);
    let recipient = Address::repeat_byte(0xbb);
    let envelope = deposit_envelope(recipient);
    let raw: Bytes = envelope.encoded_2718().into();
    assert_eq!(raw[0], 0x7e);
    assert_eq!(
        L1BlockInfo::default().calculate_tx_l1_cost(&raw, BaseSpecId::new(BaseUpgrade::Beryl)),
        U256::ZERO
    );

    let mut db = in_memory_base_backend();
    let tx = BaseTransaction::from_recovered_tx(&envelope, sender);
    assert_eq!(tx.enveloped_tx(), Some(&raw));
    let mut evm = BaseEvmFactory::default().create_foundry_evm_with_inspector(
        &mut db,
        simple_base_env(BaseUpgrade::Beryl),
        L1BlockInfo::default(),
        NoOpInspector,
    );
    evm.ctx_mut().chain.l2_block = Some(U256::ONE);
    let result = evm.transact_commit(tx).expect("execute Base deposit");
    assert!(result.is_success());
    assert_eq!(result.tx_gas_used(), 21_000);
    drop(evm);
    assert_eq!(balance(&mut db, sender), U256::from(400));
    assert_eq!(balance(&mut db, recipient), U256::from(600));
    assert_eq!(balance(&mut db, Predeploys::L1_FEE_VAULT), U256::ZERO);

    let reverter = Address::repeat_byte(0xcc);
    let mut db = in_memory_base_backend();
    db.insert_account_info(
        reverter,
        AccountInfo::default().with_code(Bytecode::new_raw(Bytes::from_static(&[0xfe]))),
    );
    let envelope = deposit_envelope(reverter);
    let tx = BaseTransaction::from_recovered_tx(&envelope, sender);
    let mut evm = BaseEvmFactory::default().create_foundry_evm_with_inspector(
        &mut db,
        simple_base_env(BaseUpgrade::Beryl),
        L1BlockInfo::default(),
        NoOpInspector,
    );
    evm.ctx_mut().chain.l2_block = Some(U256::ONE);
    let result = evm.transact_commit(tx).expect("execute failed Base deposit");
    assert!(matches!(
        result,
        revm::context::result::ExecutionResult::Halt { reason: BaseHaltReason::FailedDeposit, .. }
    ));
    assert_eq!(result.tx_gas_used(), 100_000);
    drop(evm);
    assert_eq!(balance(&mut db, sender), U256::from(1_000));
    assert_eq!(balance(&mut db, reverter), U256::ZERO);
    assert_eq!(nonce(&mut db, sender), 1);
}

fn eip8130_transaction(signer: &PrivateKeySigner) -> (BaseTxEnvelope, BaseTransaction<TxEnv>) {
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
    let signed = Eip8130Signed::new(tx, signature.as_bytes().to_vec().into(), Bytes::new());
    let envelope = BaseTxEnvelope::Eip8130(signed);
    let base_tx = BaseTransaction::from_recovered_tx(&envelope, signer.address());
    (envelope, base_tx)
}

fn eip8130_backend(sender: Address) -> Backend<BaseEvmNetwork> {
    let mut db = in_memory_base_backend();
    db.insert_account_info(
        sender,
        AccountInfo { balance: U256::from(10u64).pow(U256::from(18)), ..Default::default() },
    );
    db
}

#[test]
fn simulates_and_commits_eip8130_without_placeholder_txenv() {
    let signer = PrivateKeySigner::from_bytes(&B256::with_last_byte(1)).unwrap();
    let sender = signer.address();
    let (envelope, tx) = eip8130_transaction(&signer);
    let raw: Bytes = envelope.encoded_2718().into();
    assert_eq!(raw[0], 0x79);
    assert_eq!(tx.enveloped_tx(), Some(&raw));
    assert!(tx.eip8130.is_some());

    let mut simulation_tx = tx.clone();
    simulation_tx.eip8130.as_mut().unwrap().mode = Eip8130ExecutionMode::Simulate;
    let mut simulation_db = eip8130_backend(sender);
    let initial_balance = balance(&mut simulation_db, sender);
    let mut simulation_evm = BaseEvmFactory::default().create_foundry_evm_with_inspector(
        &mut simulation_db,
        simple_base_env(BaseUpgrade::Cobalt),
        L1BlockInfo::default(),
        NoOpInspector,
    );
    simulation_evm.ctx_mut().chain.l2_block = Some(U256::ONE);
    let simulation_result =
        simulation_evm.transact_commit(simulation_tx).expect("simulate EIP-8130");
    assert!(simulation_result.is_success());
    assert!(simulation_result.tx_gas_used() > 0);
    drop(simulation_evm);
    assert_eq!(balance(&mut simulation_db, sender), initial_balance);
    assert_eq!(nonce(&mut simulation_db, sender), 0);

    let mut db = eip8130_backend(sender);
    let mut evm = BaseEvmFactory::default().create_foundry_evm_with_inspector(
        &mut db,
        simple_base_env(BaseUpgrade::Cobalt),
        L1BlockInfo::default(),
        NoOpInspector,
    );
    evm.ctx_mut().chain.l2_block = Some(U256::ONE);
    let result = evm.transact_commit(tx.clone()).expect("commit EIP-8130");
    assert!(result.is_success());
    assert!(result.tx_gas_used() > 0);
    drop(evm);
    assert!(balance(&mut db, sender) < initial_balance);

    let mut replay_evm = BaseEvmFactory::default().create_foundry_evm_with_inspector(
        &mut db,
        simple_base_env(BaseUpgrade::Cobalt),
        L1BlockInfo::default(),
        NoOpInspector,
    );
    replay_evm.ctx_mut().chain.l2_block = Some(U256::ONE);
    assert!(replay_evm.transact_commit(tx).is_err(), "protocol nonce replay must fail");
}

#[test]
fn converts_eip8130_from_any_rpc_transaction() {
    let signer = PrivateKeySigner::from_bytes(&B256::with_last_byte(2)).unwrap();
    let (envelope, expected) = eip8130_transaction(&signer);
    let hash = *envelope.tx_hash();
    let rpc_tx = BaseRpcTransaction::from_transaction(
        Recovered::new_unchecked(envelope, signer.address()),
        BaseTransactionInfo::default(),
    );
    let mut rpc_value = serde_json::to_value(rpc_tx).unwrap();
    rpc_value
        .as_object_mut()
        .unwrap()
        .insert("hash".to_string(), serde_json::to_value(hash).unwrap());
    let unknown = serde_json::from_value::<UnknownTxEnvelope>(rpc_value).unwrap();
    let any_tx = alloy_rpc_types::Transaction::from_transaction(
        Recovered::new_unchecked(AnyTxEnvelope::Unknown(unknown), signer.address()),
        Default::default(),
    );
    let any = AnyRpcTransaction::new(WithOtherFields::new(any_tx));

    let converted = BaseTransaction::<TxEnv>::from_any_rpc_transaction(&any).unwrap();
    assert_eq!(converted.enveloped_tx(), expected.enveloped_tx());
    assert_eq!(converted.eip8130, expected.eip8130);
}
