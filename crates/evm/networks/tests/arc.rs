//! Blocklist conformance tests for Arc's native USDC precompiles.
//!
//! Seed storage independently of Forge's cheatcode inspector.

use alloy_evm::{EthEvmFactory, Evm, EvmEnv, EvmFactory};
use alloy_primitives::{Address, Bytes, TxKind, U256, address, keccak256};
use alloy_sol_types::{SolCall, SolValue, sol};
use foundry_evm_networks::arc::{self, NATIVE_COIN_AUTHORITY};
use revm::{
    context::{
        BlockEnv, CfgEnv, TxEnv,
        result::{ExecutionResult, ResultAndState},
    },
    database::InMemoryDB,
    primitives::hardfork::SpecId,
    state::AccountInfo,
};

const CONTROL: Address = address!("1800000000000000000000000000000000000001");
const USDC: Address = address!("3600000000000000000000000000000000000000");
const SENDER: Address = Address::with_last_byte(0x42);
const RECIPIENT: Address = Address::with_last_byte(0x43);

sol! {
    function transfer(address from, address to, uint256 amount) external returns (bool);
    function isBlocklisted(address account) external view returns (bool);
}

fn execute(target: Address, data: Bytes, blocked: Address) -> ResultAndState {
    let mut db = InMemoryDB::default();
    db.insert_account_info(SENDER, AccountInfo { balance: U256::from(100), ..Default::default() });
    db.insert_account_storage(
        CONTROL,
        keccak256((blocked, U256::from(2)).abi_encode()).into(),
        U256::ONE,
    )
    .unwrap();
    let env = EvmEnv::new(
        CfgEnv::new().with_chain_id(5_042_002).with_spec_and_mainnet_gas_params(SpecId::OSAKA),
        BlockEnv { timestamp: U256::from(1_800_000_000u64), ..Default::default() },
    );
    let mut evm = EthEvmFactory::default().create_evm(db, env);
    arc::apply_precompiles(evm.precompiles_mut(), 5_042_002);
    evm.transact(TxEnv {
        caller: USDC,
        chain_id: Some(5_042_002),
        kind: TxKind::Call(target),
        data,
        gas_limit: 100_000,
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn native_transfer_rejects_blocked_sender_and_recipient() {
    for blocked in [SENDER, RECIPIENT] {
        let output = execute(
            NATIVE_COIN_AUTHORITY,
            transferCall { from: SENDER, to: RECIPIENT, amount: U256::from(30) }
                .abi_encode()
                .into(),
            blocked,
        );
        assert!(matches!(&output.result, ExecutionResult::Revert { .. }));
        let mut expected = vec![0x08, 0xc3, 0x79, 0xa0];
        expected.extend("Blocked address".abi_encode());
        assert_eq!(output.result.output(), Some(&Bytes::from(expected)));
        assert_eq!(
            output.state.get(&SENDER).map_or(U256::from(100), |account| account.info.balance),
            U256::from(100)
        );
        assert_eq!(
            output.state.get(&RECIPIENT).map_or(U256::ZERO, |account| account.info.balance),
            U256::ZERO
        );
        assert!(output.result.logs().is_empty());
    }
}

#[test]
fn native_control_reads_blocklist_storage() {
    for account in [SENDER, RECIPIENT] {
        let output = execute(CONTROL, isBlocklistedCall { account }.abi_encode().into(), SENDER);
        assert!(output.result.is_success());
        assert_eq!(output.result.output(), Some(&Bytes::from((account == SENDER).abi_encode())));
        assert!(output.result.logs().is_empty());
    }
}
