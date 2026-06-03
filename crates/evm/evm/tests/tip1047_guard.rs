//! Integration tests for the TIP-1047 (Tempo T5) CREATE / CREATE2 prefix guard
//! and EIP-7702 / Tempo AA authorization masking, exercised through the
//! `foundry-evm` `Executor`.

use alloy_primitives::{Address, B256, Bytes, U256, hex};
use foundry_evm::{
    backend::Backend,
    executors::{Executor, ExecutorBuilder},
};
use foundry_evm_core::{
    constants::CALLER,
    env::FoundryTransaction,
    evm::{EvmEnvFor, TempoEvmNetwork, TxEnvFor},
    tempo::test_seam::OverrideGuard,
};
use foundry_evm_hardforks::TempoHardfork;
use revm::{
    DatabaseRef,
    context::{
        TxEnv,
        either::Either,
        transaction::{Authorization, RecoveredAuthority, RecoveredAuthorization},
    },
    primitives::KECCAK_EMPTY,
};
use tempo_revm::TempoTxEnv;

/// Init code that deploys a single-byte `STOP` runtime. `STOP` lives at offset 12.
const TINY_INIT_CODE: &[u8] = &hex!("6001600c60003960016000f300");

fn tempo_executor(spec: TempoHardfork) -> Executor<TempoEvmNetwork> {
    let backend = Backend::<TempoEvmNetwork>::spawn(None).unwrap();
    // Stay under Tempo's 30M per-tx gas cap.
    ExecutorBuilder::default().spec_id(spec).gas_limit(10_000_000).build(
        EvmEnvFor::<TempoEvmNetwork>::default(),
        TxEnvFor::<TempoEvmNetwork>::default(),
        backend,
    )
}

/// Override prefix matching the address `caller` would CREATE next.
fn override_for_create(caller: Address, nonce: u64) -> (OverrideGuard, Address) {
    let predicted = caller.create(nonce);
    let mut prefix = [0u8; 12];
    prefix.copy_from_slice(&predicted.as_slice()[..12]);
    (OverrideGuard::new(prefix), predicted)
}

fn seed_caller(
    executor: &mut Executor<TempoEvmNetwork>,
    caller: Address,
    balance: U256,
    nonce: u64,
) {
    executor.set_balance(caller, balance).unwrap();
    executor.set_nonce(caller, nonce).unwrap();
}

#[test]
fn t5_create_with_tip20_prefix_reverts_with_no_state_changes() {
    let mut executor = tempo_executor(TempoHardfork::T5);
    let initial_balance = U256::from(1_000_000_000_000_000_000u128);
    let nonce = 7u64;
    seed_caller(&mut executor, CALLER, initial_balance, nonce);

    let (_guard, predicted_address) = override_for_create(CALLER, nonce);

    // value=0 because Tempo rejects native value transfers at validation time.
    let res = executor.deploy(CALLER, Bytes::from_static(TINY_INIT_CODE), U256::ZERO, None);

    let err = res.expect_err("TIP-1047 guard should cause deploy to revert");
    let err_str = format!("{err:?}");
    // The guard's revert payload is hex-encoded in the error blob.
    let tip20_marker_hex = alloy_primitives::hex::encode("TIP-20 prefix create address forbidden");
    assert!(
        err_str.contains("TIP-20 prefix") || err_str.contains(&tip20_marker_hex),
        "expected a TIP-1047 guard revert, got: {err_str}",
    );

    // Tempo bumps `AccountInfo::nonce` only inside the CREATE frame, so
    // short-circuiting must leave the caller's account nonce unchanged.
    let post_nonce = executor.get_nonce(CALLER).unwrap();
    assert_eq!(post_nonce, nonce, "in-frame nonce bump must not happen");

    let post_predicted_balance = executor.get_balance(predicted_address).unwrap();
    assert_eq!(post_predicted_balance, U256::ZERO, "predicted address must have no balance");

    // `AccountInfo::code` is lazy; authoritative emptiness check is `code_hash`.
    let predicted_code_hash = executor
        .backend()
        .basic_ref(predicted_address)
        .unwrap()
        .map(|a| a.code_hash)
        .unwrap_or(KECCAK_EMPTY);
    assert!(
        predicted_code_hash == KECCAK_EMPTY || predicted_code_hash == B256::ZERO,
        "no code at predicted address; got {predicted_code_hash}",
    );
}

#[test]
fn pre_t5_create_with_same_prefix_succeeds() {
    // Same setup as the T5 test, but on T4 the guard must NOT fire.
    let mut executor = tempo_executor(TempoHardfork::T4);
    let initial_balance = U256::from(1_000_000_000_000_000_000u128);
    let nonce = 7u64;
    seed_caller(&mut executor, CALLER, initial_balance, nonce);

    let (_guard, predicted_address) = override_for_create(CALLER, nonce);

    let res = executor
        .deploy(CALLER, Bytes::from_static(TINY_INIT_CODE), U256::ZERO, None)
        .expect("pre-T5 deploy must succeed");
    assert_eq!(res.address, predicted_address);

    let deployed_code_hash = executor
        .backend()
        .basic_ref(predicted_address)
        .unwrap()
        .map(|a| a.code_hash)
        .unwrap_or(KECCAK_EMPTY);
    assert!(
        deployed_code_hash != KECCAK_EMPTY && deployed_code_hash != B256::ZERO,
        "runtime code must be installed; got {deployed_code_hash}",
    );

    assert_eq!(executor.get_nonce(CALLER).unwrap(), nonce + 1);
}

#[test]
fn t5_authorization_list_masks_prefixed_authority() {
    let custom_prefix = [0xEE; 12];
    let _g = OverrideGuard::new(custom_prefix);

    let prefix_authority = {
        let mut a = [0u8; 20];
        a[..12].copy_from_slice(&custom_prefix);
        a[12..].copy_from_slice(&[1u8; 8]);
        Address::from(a)
    };
    let plain_authority = Address::repeat_byte(0x42);

    let mut tx = TempoTxEnv::default();
    tx.inner.authorization_list = vec![
        Either::Right(RecoveredAuthorization::new_unchecked(
            Authorization { chain_id: U256::ONE, address: Address::ZERO, nonce: 0 },
            RecoveredAuthority::Valid(plain_authority),
        )),
        Either::Right(RecoveredAuthorization::new_unchecked(
            Authorization { chain_id: U256::ONE, address: Address::ZERO, nonce: 0 },
            RecoveredAuthority::Valid(prefix_authority),
        )),
    ];

    tx.mask_tip20_prefixed_authorizations();

    assert_eq!(tx.inner.authorization_list.len(), 2);
    match &tx.inner.authorization_list[0] {
        Either::Right(r) => assert_eq!(r.authority(), Some(plain_authority)),
        _ => panic!("plain entry must stay Either::Right"),
    }
    match &tx.inner.authorization_list[1] {
        Either::Right(r) => assert!(r.authority().is_none()),
        _ => panic!("prefix entry must stay Either::Right"),
    }

    // Default impl on plain `TxEnv` masks the standard list.
    let mut eth_tx = TxEnv {
        authorization_list: vec![Either::Right(RecoveredAuthorization::new_unchecked(
            Authorization { chain_id: U256::ONE, address: Address::ZERO, nonce: 0 },
            RecoveredAuthority::Valid(prefix_authority),
        ))],
        ..Default::default()
    };
    eth_tx.mask_tip20_prefixed_authorizations();
    match &eth_tx.authorization_list[0] {
        Either::Right(r) => assert!(r.authority().is_none()),
        _ => panic!("Eth tx must also mask prefixed entries"),
    }
}
