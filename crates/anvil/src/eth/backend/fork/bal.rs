//! Prefills immutable fork state from a validated block access list.

use alloy_consensus::BlockHeader;
use alloy_eips::{
    BlockId,
    eip7928::{BlockAccessList, compute_block_access_list_hash, validate_block_access_list},
};
use alloy_network::AnyRpcBlock;
use alloy_primitives::{Address, B256, U256, map::U256Map};
use alloy_provider::Provider;
use eyre::{Result, WrapErr};
use foundry_common::provider::RetryProvider;
use foundry_evm::backend::BlockchainDb;
use revm::state::{AccountInfo, Bytecode};
use std::time::Duration;

/// Validated post-block values that can populate the remote cache without fetching accounts.
#[derive(Debug)]
pub(crate) struct PreparedBalSeed {
    block_hash: B256,
    accounts: Vec<(Address, AccountInfo)>,
    storage: Vec<(Address, Vec<(U256, U256)>)>,
}

/// Fetches an optional BAL within a bounded budget, including the provider's retries.
pub(crate) async fn fetch(
    provider: &RetryProvider,
    block: &AnyRpcBlock,
) -> Option<PreparedBalSeed> {
    let block_hash = block.header.hash;
    let bal = match tokio::time::timeout(
        Duration::from_millis(500),
        provider.get_block_access_list(BlockId::hash(block_hash)),
    )
    .await
    {
        Ok(Ok(Some(bal))) => bal,
        Ok(Ok(None)) => {
            debug!(target: "node", %block_hash, "fork BAL unavailable");
            return None;
        }
        Ok(Err(_)) => {
            debug!(target: "node", %block_hash, "fork BAL request failed");
            return None;
        }
        Err(_) => {
            debug!(target: "node", %block_hash, "fork BAL request timed out");
            return None;
        }
    };
    match PreparedBalSeed::new(
        bal,
        block_hash,
        block.transactions.len(),
        block.header.block_access_list_hash(),
    ) {
        Ok(seed) => Some(seed),
        Err(err) => {
            debug!(target: "node", %block_hash, %err, "ignoring invalid fork BAL");
            None
        }
    }
}

impl PreparedBalSeed {
    fn new(
        bal: BlockAccessList,
        block_hash: B256,
        transaction_count: usize,
        expected_hash: Option<B256>,
    ) -> Result<Self> {
        validate_block_access_list(&bal, transaction_count).wrap_err("invalid BAL structure")?;
        if let Some(expected_hash) = expected_hash {
            eyre::ensure!(
                compute_block_access_list_hash(&bal) == expected_hash,
                "BAL hash mismatch"
            );
        }

        let mut accounts = Vec::new();
        let mut storage = Vec::new();
        for account in bal {
            if !account.storage_changes.is_empty() {
                let mut slots = Vec::with_capacity(account.storage_changes.len());
                slots.extend(account.storage_post_states());
                storage.push((account.address, slots));
            }
            let balance = account.balance_post_state();
            let nonce = account.nonce_post_state();
            let mut code = None;
            // Validate code even when this account's remaining fields are incomplete.
            for change in account.code_changes {
                code =
                    Some(Bytecode::new_raw_checked(change.new_code).wrap_err("invalid BAL code")?);
            }
            if let (Some(balance), Some(nonce), Some(code)) = (balance, nonce, code) {
                accounts.push((
                    account.address,
                    AccountInfo {
                        balance,
                        nonce,
                        code_hash: code.hash_slow(),
                        code: Some(code),
                        account_id: None,
                    },
                ));
            }
        }
        Ok(Self { block_hash, accounts, storage })
    }

    /// Seeds an exact fork block's remote cache, retaining already cached accounts and slots.
    ///
    /// The caller must apply this before exposing the staged database or applying local overrides.
    /// A seed for a different block is ignored without modifying the database.
    pub(crate) fn apply(self, db: &BlockchainDb) {
        if db.meta().read().fork_hash != Some(self.block_hash) {
            debug!(target: "node", block_hash=%self.block_hash, "ignoring fork BAL for a different cache block");
            return;
        }

        let mut accounts = db.accounts().write();
        let accounts_before = accounts.len();
        for (address, account) in self.accounts {
            accounts.entry(address).or_insert(account);
        }
        let inserted_accounts = accounts.len() - accounts_before;
        drop(accounts);

        let mut storage = db.storage().write();
        let mut inserted_slots = 0;
        for (address, slots) in self.storage {
            let cached_slots = storage.entry(address).or_insert_with(|| {
                U256Map::with_capacity_and_hasher(slots.len(), Default::default())
            });
            let slots_before = cached_slots.len();
            for (slot, value) in slots {
                cached_slots.entry(slot).or_insert(value);
            }
            inserted_slots += cached_slots.len() - slots_before;
        }
        debug!(target: "node", block_hash=%self.block_hash, inserted_accounts, inserted_slots, "prefilled fork cache from BAL");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_eips::eip7928::{
        AccountChanges, BalanceChange, BlockAccessIndex, CodeChange, NonceChange, SlotChanges,
        StorageChange,
    };
    use alloy_primitives::{Bytes, bytes};
    use foundry_evm::backend::BlockchainDbMeta;
    use revm::context::BlockEnv;

    fn database(hash: B256) -> BlockchainDb {
        BlockchainDb::new(
            BlockchainDbMeta::new(BlockEnv::default(), "http://localhost:8545".to_string())
                .with_fork_identity(hash, B256::ZERO),
            None,
        )
    }

    fn index(index: u64) -> BlockAccessIndex {
        BlockAccessIndex::new(index)
    }

    #[test]
    fn fork_bal_seed_keeps_final_zero_and_system_writes() {
        let hash = B256::repeat_byte(1);
        let address = Address::repeat_byte(1);
        let slot = U256::from(1);
        let system_slot = U256::from(2);
        let account = AccountChanges::new(address)
            .with_storage_change(SlotChanges::new(
                slot,
                vec![
                    StorageChange::new(index(0), U256::from(3)),
                    StorageChange::new(index(1), U256::from(4)),
                    StorageChange::new(index(3), U256::ZERO),
                ],
            ))
            .with_storage_change(SlotChanges::new(
                system_slot,
                vec![StorageChange::new(index(0), U256::from(5))],
            ));
        let db = database(hash);

        PreparedBalSeed::new(vec![account], hash, 2, None).unwrap().apply(&db);

        let storage = db.storage().read();
        assert_eq!(storage[&address][&slot], U256::ZERO);
        assert_eq!(storage[&address][&system_slot], U256::from(5));
        assert!(db.accounts().read().is_empty());
    }

    #[test]
    fn fork_bal_seed_leaves_partial_accounts_and_reads_unknown() {
        let hash = B256::repeat_byte(1);
        let address = Address::repeat_byte(1);
        let account = AccountChanges::new(address)
            .with_balance_change(BalanceChange::new(index(1), U256::from(42)))
            .with_storage_read(U256::from(2));
        let db = database(hash);

        PreparedBalSeed::new(vec![account], hash, 1, None).unwrap().apply(&db);

        assert!(db.accounts().read().is_empty());
        assert!(db.storage().read().is_empty());
    }

    fn complete_account(address: Address, code: Bytes) -> AccountChanges {
        AccountChanges::new(address)
            .with_balance_change(BalanceChange::new(index(1), U256::from(42)))
            .with_nonce_change(NonceChange::new(index(1), 3))
            .with_code_change(CodeChange::new(index(1), code))
    }

    #[test]
    fn fork_bal_seed_preserves_cached_values_and_merges_slots() {
        let hash = B256::repeat_byte(1);
        let address = Address::repeat_byte(1);
        let account = complete_account(address, Bytes::new())
            .with_storage_change(SlotChanges::new(
                U256::from(1),
                vec![StorageChange::new(index(1), U256::from(11))],
            ))
            .with_storage_change(SlotChanges::new(
                U256::from(2),
                vec![StorageChange::new(index(1), U256::from(22))],
            ));
        let db = database(hash);
        let cached_account = AccountInfo { balance: U256::from(99), ..Default::default() };
        db.accounts().write().insert(address, cached_account.clone());
        db.storage().write().insert(
            address,
            [(U256::from(1), U256::from(101)), (U256::from(3), U256::from(303))]
                .into_iter()
                .collect(),
        );

        PreparedBalSeed::new(vec![account], hash, 1, None).unwrap().apply(&db);

        assert_eq!(db.accounts().read()[&address], cached_account);
        assert_eq!(
            db.storage().read()[&address],
            [
                (U256::from(1), U256::from(101)),
                (U256::from(2), U256::from(22)),
                (U256::from(3), U256::from(303)),
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn fork_bal_seed_survives_disk_cache_reload() {
        let hash = B256::repeat_byte(1);
        let address = Address::repeat_byte(1);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("storage.json");
        let meta = BlockchainDbMeta::new(BlockEnv::default(), "http://localhost:8545".to_string())
            .with_fork_identity(hash, B256::ZERO);
        let db = BlockchainDb::new(meta.clone(), Some(path.clone()));
        db.storage().write().entry(address).or_default().insert(U256::from(9), U256::from(99));
        db.cache().flush();
        drop(db);

        let db = BlockchainDb::new(meta.clone(), Some(path.clone()));
        let code = bytes!("6000");
        let account = complete_account(address, code.clone()).with_storage_change(
            SlotChanges::new(U256::ONE, vec![StorageChange::new(index(1), U256::ZERO)]),
        );
        PreparedBalSeed::new(vec![account], hash, 1, None).unwrap().apply(&db);
        db.cache().flush();
        drop(db);

        let db = BlockchainDb::new(meta, Some(path));
        assert_eq!(db.storage().read()[&address][&U256::ONE], U256::ZERO);
        assert_eq!(db.storage().read()[&address][&U256::from(9)], U256::from(99));
        assert!(!db.storage().read()[&address].contains_key(&U256::from(2)));
        let accounts = db.accounts().read();
        let account = &accounts[&address];
        assert_eq!(account.balance, U256::from(42));
        assert_eq!(account.nonce, 3);
        assert_eq!(account.code.as_ref().unwrap().original_bytes(), code);
    }

    #[test]
    fn fork_bal_seed_preserves_delegation_code_and_final_clearing() {
        let hash = B256::repeat_byte(1);
        let authority = Address::repeat_byte(1);
        let cleared = Address::repeat_byte(2);
        let delegation = bytes!("ef01000000000000000000000000000000000000000042");
        let db = database(hash);
        let cleared_account = complete_account(cleared, delegation.clone())
            .with_balance_change(BalanceChange::new(index(2), U256::ZERO))
            .with_nonce_change(NonceChange::new(index(2), 0))
            .with_code_change(CodeChange::new(index(2), Bytes::new()));

        PreparedBalSeed::new(
            vec![complete_account(authority, delegation.clone()), cleared_account],
            hash,
            2,
            None,
        )
        .unwrap()
        .apply(&db);

        let accounts = db.accounts().read();
        let account = &accounts[&authority];
        assert_eq!(account.balance, U256::from(42));
        assert_eq!(account.nonce, 3);
        assert_eq!(account.code_hash, alloy_primitives::keccak256(&delegation));
        assert_eq!(account.code.as_ref().unwrap().original_bytes(), delegation);
        assert!(account.code.as_ref().unwrap().is_eip7702());
        let account = &accounts[&cleared];
        assert_eq!(account.balance, U256::ZERO);
        assert_eq!(account.nonce, 0);
        assert_eq!(account.code_hash, alloy_primitives::KECCAK256_EMPTY);
        assert!(account.code.as_ref().unwrap().is_empty());
    }

    #[test]
    fn fork_bal_seed_rejects_wrong_block_without_mutation() {
        let hash = B256::repeat_byte(1);
        let address = Address::repeat_byte(1);
        let account = complete_account(address, Bytes::new()).with_storage_change(
            SlotChanges::new(U256::from(1), vec![StorageChange::new(index(0), U256::from(42))]),
        );
        let db = database(B256::repeat_byte(2));

        PreparedBalSeed::new(vec![account], hash, 1, None).unwrap().apply(&db);

        assert!(db.accounts().read().is_empty());
        assert!(db.storage().read().is_empty());
    }

    #[test]
    fn fork_bal_seed_rejects_invalid_structure_hash_and_bytecode() {
        let hash = B256::repeat_byte(1);
        let address = Address::repeat_byte(1);
        let valid = vec![complete_account(address, Bytes::new())];
        let commitment = compute_block_access_list_hash(&valid);
        assert!(PreparedBalSeed::new(valid.clone(), hash, 1, Some(commitment)).is_ok());
        assert!(PreparedBalSeed::new(valid.clone(), hash, 1, Some(B256::ZERO)).is_err());

        let duplicate_accounts = vec![valid[0].clone(), valid[0].clone()];
        assert!(PreparedBalSeed::new(duplicate_accounts, hash, 1, None).is_err());
        let invalid_index = AccountChanges::new(address)
            .with_balance_change(BalanceChange::new(index(3), U256::ZERO));
        assert!(PreparedBalSeed::new(vec![invalid_index], hash, 1, None).is_err());
        let empty_changes = AccountChanges::new(address)
            .with_storage_change(SlotChanges::new(U256::from(1), vec![]));
        assert!(PreparedBalSeed::new(vec![empty_changes], hash, 1, None).is_err());

        // Invalid code must discard the seed even for an incomplete account or an earlier write.
        let invalid_code = AccountChanges::new(address)
            .with_code_change(CodeChange::new(index(0), bytes!("ef0100")))
            .with_code_change(CodeChange::new(index(1), Bytes::new()));
        assert!(PreparedBalSeed::new(vec![invalid_code], hash, 1, None).is_err());
    }

    #[test]
    fn fork_bal_seed_accepts_empty_block_with_post_execution_write() {
        let hash = B256::repeat_byte(1);
        let address = Address::repeat_byte(1);
        let slot = U256::from(1);
        let account = AccountChanges::new(address).with_storage_change(SlotChanges::new(
            slot,
            vec![StorageChange::new(index(1), U256::from(42))],
        ));
        let db = database(hash);

        PreparedBalSeed::new(vec![account], hash, 0, None).unwrap().apply(&db);

        assert_eq!(db.storage().read()[&address][&slot], U256::from(42));
    }
}
