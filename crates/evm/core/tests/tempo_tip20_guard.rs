//! Integration tests for the TIP-1047 (Tempo T5) prefix guard helpers.

use alloy_primitives::{Address, Bytes, U256};
use foundry_evm_core::{
    env::FoundryTransaction,
    tempo::{
        Tip1047CreateGuard, is_tip20_prefix_for_guard, mask_tip20_prefixed_eth_authorizations,
        predicted_create_address, test_seam::OverrideGuard, tip20_prefix_revert,
    },
};
use revm::{
    context::{
        TxEnv,
        either::Either,
        transaction::{
            Authorization, RecoveredAuthority, RecoveredAuthorization, SignedAuthorization,
        },
    },
    context_interface::CreateScheme,
    interpreter::{CreateInputs, InstructionResult},
};
use tempo_contracts::precompiles::PATH_USD_ADDRESS;

const fn create_inputs(scheme: CreateScheme) -> CreateInputs {
    CreateInputs::new(
        Address::repeat_byte(0x11),
        scheme,
        U256::ZERO,
        Bytes::from_static(b""),
        1_000_000,
        0,
    )
}

#[test]
fn predicted_address_handles_each_scheme() {
    let create = create_inputs(CreateScheme::Create);
    assert!(predicted_create_address(&create, 0).is_some());

    let create2 = create_inputs(CreateScheme::Create2 { salt: U256::from(42u64) });
    assert!(predicted_create_address(&create2, 0).is_some());

    let custom = create_inputs(CreateScheme::Custom { address: Address::ZERO });
    assert!(predicted_create_address(&custom, 0).is_none());
}

#[test]
fn tip20_prefix_revert_preserves_gas_and_reservoir() {
    let mut inputs = create_inputs(CreateScheme::Create);
    inputs.set_gas_limit(123_456);
    inputs.set_reservoir(7_890);

    let outcome = tip20_prefix_revert(&inputs, Address::ZERO);
    assert_eq!(outcome.result.result, InstructionResult::Revert);
    assert!(outcome.address.is_none());
    assert_eq!(outcome.result.gas.limit(), 123_456);
    assert_eq!(outcome.result.gas.total_gas_spent(), 0);
    assert_eq!(outcome.result.gas.reservoir(), 7_890);
    assert!(outcome.result.output.starts_with(b"TIP-20 prefix create address forbidden"));
}

#[test]
fn is_tip20_prefix_for_guard_uses_override_when_set() {
    let custom_prefix = [0xAB; 12];
    let _g = OverrideGuard::new(custom_prefix);

    let matching = {
        let mut a = [0u8; 20];
        a[..12].copy_from_slice(&custom_prefix);
        Address::from(a)
    };

    assert!(is_tip20_prefix_for_guard(matching));
    assert!(!is_tip20_prefix_for_guard(Address::ZERO));
    // Override replaces the canonical prefix.
    assert!(!is_tip20_prefix_for_guard(PATH_USD_ADDRESS));
}

#[test]
fn is_tip20_prefix_for_guard_falls_back_to_canonical_without_override() {
    assert!(is_tip20_prefix_for_guard(PATH_USD_ADDRESS));
    assert!(!is_tip20_prefix_for_guard(Address::ZERO));
}

fn signed_auth(addr: Address) -> SignedAuthorization {
    SignedAuthorization::new_unchecked(
        Authorization { chain_id: U256::from(1u64), address: addr, nonce: 0 },
        0,
        U256::ZERO,
        U256::ZERO,
    )
}

fn recovered_valid_for(authority: Address) -> RecoveredAuthorization {
    RecoveredAuthorization::new_unchecked(
        Authorization { chain_id: U256::from(1u64), address: Address::ZERO, nonce: 0 },
        RecoveredAuthority::Valid(authority),
    )
}

#[test]
fn mask_eth_authorizations_preserves_length_and_marks_invalid() {
    let custom_prefix = [0xCD; 12];
    let _g = OverrideGuard::new(custom_prefix);

    let prefix_authority = {
        let mut a = [0u8; 20];
        a[..12].copy_from_slice(&custom_prefix);
        a[12..].copy_from_slice(&[1u8; 8]);
        Address::from(a)
    };
    let plain_authority = Address::repeat_byte(0x42);

    let mut auths: Vec<Either<SignedAuthorization, RecoveredAuthorization>> = vec![
        Either::Right(recovered_valid_for(plain_authority)),
        Either::Right(recovered_valid_for(prefix_authority)),
        Either::Left(signed_auth(Address::repeat_byte(0x77))),
    ];

    mask_tip20_prefixed_eth_authorizations(&mut auths);

    assert_eq!(auths.len(), 3);
    match &auths[0] {
        Either::Right(r) => assert_eq!(r.authority(), Some(plain_authority)),
        _ => panic!("plain entry must stay Either::Right"),
    }
    match &auths[1] {
        Either::Right(r) => assert!(r.authority().is_none()),
        _ => panic!("prefix entry must stay Either::Right"),
    }
    assert!(matches!(&auths[2], Either::Left(_)));
}

#[test]
fn foundry_transaction_mask_on_eth_tx_env() {
    let custom_prefix = [0xEE; 12];
    let _g = OverrideGuard::new(custom_prefix);

    let prefix_authority = {
        let mut a = [0u8; 20];
        a[..12].copy_from_slice(&custom_prefix);
        a[12..].copy_from_slice(&[1u8; 8]);
        Address::from(a)
    };

    let mut tx = TxEnv {
        authorization_list: vec![Either::Right(recovered_valid_for(prefix_authority))],
        ..Default::default()
    };
    tx.mask_tip20_prefixed_authorizations();
    match &tx.authorization_list[0] {
        Either::Right(r) => assert!(r.authority().is_none()),
        _ => panic!("prefix entry must stay Either::Right"),
    }
}

#[test]
fn tip1047_create_guard_wrapper_constructs() {
    // Smoke test: ensure the wrapper type compiles and constructs against a
    // no-op inspector. Behavioral coverage is in `foundry-evm` integration tests.
    let mut inner = revm::inspector::NoOpInspector;
    let _guard = Tip1047CreateGuard::new(&mut inner);
}
