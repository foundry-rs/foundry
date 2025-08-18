//! This module provides an externalities builder for setting an environment with the Revive
//! runtime.
//!
//! It is heavily inspired from here: <https://github.com/paritytech/revive/blob/56aadce0a9554e68a08feb35cffb65574d7a618b/crates/runner/src/lib.rs#L69>
//!
//! THIS IS WORK IN PROGRESS. It is not yet complete and may change in the future.
#![allow(clippy::disallowed_macros)]
use polkadot_sdk::{
    frame_system, pallet_balances,
    pallet_revive::AddressMapper,
    polkadot_runtime_common::BuildStorage,
    sp_core::H160,
    sp_io,
    sp_keystore::{testing::MemoryKeystore, KeystoreExt},
    sp_runtime::AccountId32,
    sp_tracing,
};

pub use crate::runtime::{AccountId, Balance, Runtime, System};

mod runtime;

/// Externalities builder
#[derive(Default)]
pub struct ExtBuilder {
    // List of endowments at genesis
    balance_genesis_config: Vec<(AccountId32, Balance)>,
}

impl ExtBuilder {
    /// Set the balance of an account at genesis
    pub fn balance_genesis_config(self, value: Vec<(H160, Balance)>) -> Self {
        Self {
            balance_genesis_config: value
                .iter()
                .map(|(address, balance)| (AccountId::to_fallback_account_id(address), *balance))
                .collect(),
        }
    }

    /// Build the externalities
    pub fn build(self) -> sp_io::TestExternalities {
        sp_tracing::try_init_simple();
        let mut t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
        pallet_balances::GenesisConfig::<Runtime> {
            balances: self.balance_genesis_config,
            dev_accounts: None,
        }
        .assimilate_storage(&mut t)
        .unwrap();
        let mut ext = sp_io::TestExternalities::new(t);
        ext.register_extension(KeystoreExt::new(MemoryKeystore::new()));
        ext.execute_with(|| System::set_block_number(1));

        ext
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_externalities_works() {
        let mut ext = ExtBuilder::default()
            .balance_genesis_config(vec![(H160::from_low_u64_be(1), 1000)])
            .build();
        ext.execute_with(|| {
            assert_eq!(
                pallet_balances::Pallet::<Runtime>::free_balance(
                    AccountId::to_fallback_account_id(&H160::from_low_u64_be(1))
                ),
                1000
            );
        });
    }

    #[test]
    fn test_changing_block_number() {
        let mut ext = ExtBuilder::default().build();
        ext.execute_with(|| {
            assert_eq!(System::block_number(), 1);
            System::set_block_number(5);
            assert_eq!(System::block_number(), 5);
        });
    }
}
