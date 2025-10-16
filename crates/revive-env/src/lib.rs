//! This module provides an externalities builder for setting an environment with the Revive
//! runtime.
//!
//! It is heavily inspired from here: <https://github.com/paritytech/revive/blob/56aadce0a9554e68a08feb35cffb65574d7a618b/crates/runner/src/lib.rs#L69>
//!
//! THIS IS WORK IN PROGRESS. It is not yet complete and may change in the future.
#![allow(clippy::disallowed_macros)]
use polkadot_sdk::{
    frame_support::traits::{OnGenesis, fungible::Mutate},
    frame_system::{self, Pallet},
    pallet_balances,
    pallet_revive::{self, AddressMapper},
    polkadot_runtime_common::BuildStorage,
    sp_core::H160,
    sp_io,
    sp_keystore::{KeystoreExt, testing::MemoryKeystore},
    sp_runtime::AccountId32,
    sp_tracing,
};

pub use crate::runtime::{AccountId, Balance, Runtime, System, Timestamp};

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

        pallet_revive::GenesisConfig::<Runtime>::default().assimilate_storage(&mut t).unwrap();
        let mut ext = sp_io::TestExternalities::new(t);
        ext.register_extension(KeystoreExt::new(MemoryKeystore::new()));
        ext.execute_with(|| {
            Pallet::<Runtime>::on_genesis();
            System::set_block_number(0);

            // Set a large balance for pallet account to handle storage deposits during contract
            // migration Using a reasonable large value to avoid overflow when minting
            let pallet_account = pallet_revive::Pallet::<Runtime>::account_id();
            let large_balance: Balance = 1_000_000_000_000_000_000_000_000_000_u128;
            let _ = <Runtime as pallet_revive::Config>::Currency::mint_into(
                &pallet_account,
                large_balance,
            );

            let _ = pallet_revive::Pallet::<Runtime>::map_account(
                frame_system::RawOrigin::Signed(pallet_account).into(),
            );
        });
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
            assert_eq!(System::block_number(), 0);
            System::set_block_number(5);
            assert_eq!(System::block_number(), 5);
        });
    }
}
