use super::*;
use alloy_primitives::{Address, B256, TxKind, U256};
use evm2::{EvmFeatures, SpecId, evm::AccountInfo};

const GWEI: u128 = 1_000_000_000;
const ETHER: u128 = 1_000_000_000_000_000_000;

fn funded_db(accounts: &[(Address, u128)]) -> InMemoryDB {
    let mut db = InMemoryDB::default();
    for (address, balance) in accounts {
        db.insert_account_info(address, AccountInfo::default().with_balance(U256::from(*balance)));
    }
    db
}

fn balance<T: EvmTypes>(evm: &mut Evm<'_, T>, address: Address) -> U256 {
    evm.read_account_info(&address).unwrap().map(|info| info.balance).unwrap_or_default()
}

fn transfer<E: FoundryTxExt>(caller: Address, to: Address, value: u128) -> FoundryTxEnv<E> {
    FoundryTxEnv {
        tx_type: 2,
        caller,
        gas_limit: 50_000,
        gas_price: 10 * GWEI,
        gas_priority_fee: Some(2 * GWEI),
        kind: TxKind::Call(to),
        value: U256::from(value),
        chain_id: Some(1),
        ..Default::default()
    }
}

#[test]
fn eth_factory_executes_flat_transactions() {
    let (caller, to, coinbase) = (
        Address::with_last_byte(0xA1),
        Address::with_last_byte(0xA2),
        Address::with_last_byte(0xA3),
    );
    let factory = EthEvmFactory;
    let mut env =
        EvmEnv::new(SpecId::PRAGUE, factory.version(SpecId::PRAGUE, 1), Default::default());
    env.block.basefee = U256::from(5 * GWEI);
    env.block.beneficiary = coinbase;
    let mut evm = FoundryEvmFactory::create_evm(&factory, funded_db(&[(caller, ETHER)]), env);

    let tx = factory.build_tx(&transfer::<()>(caller, to, 7)).unwrap();
    let result = evm.transact(&tx).unwrap().commit();

    assert!(result.status);
    assert_eq!(result.tx_gas_used(), 21_000);
    // Effective price is min(max fee, base fee + priority) = 7 gwei; the coinbase earns only the
    // 2 gwei priority fee, so swapped fee fields would change both balances.
    assert_eq!(balance(&mut evm, caller), U256::from(ETHER - 7 - 21_000 * 7 * GWEI));
    assert_eq!(balance(&mut evm, to), U256::from(7));
    assert_eq!(balance(&mut evm, coinbase), U256::from(21_000 * 2 * GWEI));
}

#[test]
fn spec_changes_keep_foundry_policy() {
    let caller = Address::with_last_byte(0xA1);
    let factory = EthEvmFactory;
    let mut env =
        EvmEnv::new(SpecId::CANCUN, factory.version(SpecId::CANCUN, 31337), Default::default());
    env.set_feature(EvmFeatures::NONCE_CHECK, false);
    env.version.memory_limit = 1 << 20;
    let mut evm = FoundryEvmFactory::create_evm(&factory, funded_db(&[(caller, ETHER)]), env);

    factory.set_execution_spec(&mut evm, SpecId::PRAGUE);

    assert_eq!(evm.config_spec_id(), SpecId::PRAGUE);
    assert!(evm.feature(EvmFeatures::EIP7702), "fork features follow the new spec");
    assert!(!evm.feature(EvmFeatures::NONCE_CHECK), "disabled checks stay disabled");
    assert!(evm.feature(EvmFeatures::BALANCE_CHECK), "untouched checks stay enabled");
    assert_eq!(evm.version().chain_id, 31337);
    assert_eq!(evm.version().memory_limit, 1 << 20);

    let mut tx = transfer::<()>(caller, Address::with_last_byte(0xA2), 1);
    tx.chain_id = Some(31337);
    tx.nonce = 9;
    let result = evm.transact(&factory.build_tx(&tx).unwrap()).unwrap().commit();
    assert!(result.status, "a wrong nonce executes with the nonce check off");
}

#[test]
fn tempo_factory_seeds_local_genesis() {
    let factory = tempo_evm2::TempoEvmFactory::default();
    let spec = tempo_hardfork_evm2::TempoHardfork::default();
    let env = EvmEnv::new(spec, factory.version(spec, 31337), Default::default());
    let mut evm = FoundryEvmFactory::create_evm(&factory, InMemoryDB::default(), env);

    let code_hash = |evm: &mut Evm<'_, _>, address| {
        evm.read_account_info(&address).unwrap().map(|info| info.code_hash).unwrap_or_default()
    };
    for address in [crate::tempo::PATH_USD_ADDRESS, crate::tempo::TIP_FEE_MANAGER_ADDRESS] {
        let hash = code_hash(&mut evm, address);
        assert!(hash != B256::ZERO && hash != alloy_primitives::KECCAK256_EMPTY, "{address}");
    }
}

#[cfg(feature = "optimism")]
#[test]
fn op_factory_executes_deposits() {
    let (caller, to) = (Address::with_last_byte(0xA1), Address::with_last_byte(0xA2));
    let factory = op_evm2::OpEvmFactory;
    let spec = op_evm2::OpSpecId::default();
    let env = EvmEnv::new(spec, FoundryEvmFactory::version(&factory, spec, 10), Default::default());
    let mut evm = FoundryEvmFactory::create_evm(&factory, InMemoryDB::default(), env);

    let tx = FoundryTxEnv {
        tx_type: DEPOSIT_TX_TYPE,
        caller,
        gas_limit: 100_000,
        kind: TxKind::Call(to),
        value: U256::from(3),
        ext: DepositTxExt { mint: Some(10), ..Default::default() },
        ..Default::default()
    };
    let result = evm.transact(&factory.build_tx(&tx).unwrap()).unwrap().commit();

    assert!(result.status);
    assert!(!result.ext.is_failed_deposit());
    // The deposit mints to the unfunded caller, then transfers the value without charging fees.
    assert_eq!(balance(&mut evm, caller), U256::from(7));
    assert_eq!(balance(&mut evm, to), U256::from(3));
}

#[cfg(feature = "monad")]
#[test]
fn monad_factory_charges_the_full_gas_limit() {
    let (caller, to) = (Address::with_last_byte(0xA1), Address::with_last_byte(0xA2));
    let factory = monad_evm2::MonadEvmFactory;
    let spec = monad_evm2::MonadHardfork::default();
    let mut env =
        EvmEnv::new(spec, FoundryEvmFactory::version(&factory, spec, 1), Default::default());
    env.block.basefee = U256::from(5 * GWEI);
    let mut evm = FoundryEvmFactory::create_evm(&factory, funded_db(&[(caller, ETHER)]), env);

    let result = evm.transact(&factory.build_tx(&transfer::<()>(caller, to, 7)).unwrap()).unwrap();
    assert!(result.commit().status);
    // Monad charges the whole 50k gas limit, not the 21k used.
    assert_eq!(balance(&mut evm, caller), U256::from(ETHER - 7 - 50_000 * 7 * GWEI));
}

#[cfg(feature = "base")]
#[test]
fn base_factory_plants_precompile_sentinels() {
    let factory = base_evm2::BaseEvmFactory::default();
    let spec = base_evm2::BaseSpecId::new(base_evm2::BaseUpgrade::Beryl);
    let env =
        EvmEnv::new(spec, FoundryEvmFactory::version(&factory, spec, 8453), Default::default());
    let mut evm = FoundryEvmFactory::create_evm(&factory, InMemoryDB::default(), env);

    let mut planted = 0;
    for address in foundry_evm_networks::BASE_CODE_SENTINEL_ADDRESSES {
        let info = evm.read_account_info(address).unwrap();
        let active = foundry_evm_networks::is_base_precompile_active_at(*address, spec.upgrade());
        let has_code = info.is_some_and(|info| info.code_hash != alloy_primitives::KECCAK256_EMPTY);
        assert_eq!(has_code, active, "{address}");
        planted += usize::from(has_code);
    }
    assert!(planted > 0);
}
