//! Tests for block access list reads.

use crate::{
    backend::Backend,
    evm::{EthEvmNetwork, EvmEnv, FoundryEvmFactory, eth::EthEvmFactory},
};
use alloy_eips::eip7928::{AccountChanges, BalanceChange, SlotChanges, StorageChange};
use alloy_primitives::{Address, U256};
use evm2::evm::{
    AccountInfo, EmptyDB,
    bal::{Bal, BlockAccessIndex},
};
use std::sync::Arc;

#[test]
fn bal_reads_use_index_and_fall_back_to_parent_state() {
    let address = Address::with_last_byte(1);
    let slot = U256::ZERO;
    let spec = evm2::SpecId::AMSTERDAM;
    let env = EvmEnv::new(spec, EthEvmFactory::default().version(spec, 1), Default::default());
    let mut backend = Backend::<EthEvmNetwork>::with_db(EmptyDB::default(), env);
    backend.insert_account_info(address, AccountInfo::default().with_balance(U256::from(10)));
    backend.insert_account_storage(address, slot, U256::from(10));
    backend.insert_account_storage(address, U256::from(99), U256::from(77));
    let bal = Arc::new(
        Bal::try_from(alloy_eips::eip7928::BlockAccessList::from(vec![AccountChanges {
            balance_changes: vec![
                BalanceChange::new(BlockAccessIndex::new(0), U256::from(20)),
                BalanceChange::new(BlockAccessIndex::new(1), U256::from(30)),
            ],
            storage_changes: vec![SlotChanges::new(
                slot,
                vec![
                    StorageChange::new(BlockAccessIndex::new(0), U256::from(20)),
                    StorageChange::new(BlockAccessIndex::new(1), U256::from(30)),
                ],
            )],
            storage_reads: vec![U256::from(99)],
            ..AccountChanges::new(address)
        }]))
        .unwrap(),
    );

    let read = |backend: &Backend<EthEvmNetwork>| {
        let mut evm = backend.create_evm();
        let balance = evm.read_account_info(&address).unwrap().unwrap().balance;
        let state = evm.state_mut();
        let slots = [slot, U256::from(99)].map(|key| state.storage_slot_untracked(&address, &key));
        (balance, slots.map(Result::unwrap))
    };
    backend.set_bal(bal.clone(), BlockAccessIndex::new(1));
    assert_eq!(read(&backend), (U256::from(20), [U256::from(20), U256::from(77)]));
    backend.set_bal(bal, BlockAccessIndex::new(2));
    assert_eq!(read(&backend), (U256::from(30), [U256::from(30), U256::from(77)]));

    // Parking removes the list again.
    let evm = backend.create_evm();
    backend.park(evm);
    assert_eq!(read(&backend), (U256::from(10), [U256::from(10), U256::from(77)]));
}
