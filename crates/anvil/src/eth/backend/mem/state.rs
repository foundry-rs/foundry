//! Support for generating the state root for memdb storage

use alloy_primitives::{
    B256, U256, keccak256,
    map::{AddressMap, B256Map, HashSet, U256Map},
};
use alloy_rlp::Encodable;
use alloy_trie::{HashBuilder, Nibbles};
use reth_trie_sparse::{LeafUpdate, RevealableSparseTrie, SparseStateTrie};
use revm::{
    database::DbAccount,
    state::{Account, AccountInfo},
};
use std::mem;

/// Incrementally maintains the state trie used for mined block headers.
///
/// The old state-root path rebuilt and sorted every account and storage trie after each block.
/// That made EIP-2935's block-hash storage contract turn mining into linear work as its ring was
/// populated. This cache keeps a fully revealed in-memory trie and only rehashes paths changed by
/// the latest block.
#[derive(Debug, Default)]
pub struct StateRootCache {
    trie: Option<SparseStateTrie>,
    dirty: AddressMap<DirtyAccount>,
}

#[derive(Debug, Default)]
struct DirtyAccount {
    storage: HashSet<U256>,
    reset_storage: bool,
}

impl StateRootCache {
    /// Records changes that will be committed to the database.
    pub fn record_changes(&mut self, changes: &AddressMap<Account>) {
        for (address, account) in changes {
            if !account.is_touched() {
                continue;
            }

            let dirty = self.dirty.entry(*address).or_default();
            dirty.reset_storage |= account.is_created() || account.is_selfdestructed();
            dirty.storage.extend(account.changed_storage_slots().map(|(slot, _)| *slot));
        }
    }

    /// Records an account-info change or a database load caused by `basic`.
    pub fn record_account(&mut self, address: alloy_primitives::Address) {
        self.dirty.entry(address).or_default();
    }

    /// Records a storage change or a database load caused by `storage`.
    pub fn record_storage(&mut self, address: alloy_primitives::Address, slot: U256) {
        self.dirty.entry(address).or_default().storage.insert(slot);
    }

    /// Invalidates the trie after wholesale database replacement or clearing.
    pub fn invalidate(&mut self) {
        self.trie = None;
        self.dirty.clear();
    }

    /// Returns the current root, applying only changes recorded since the previous call.
    pub fn root(&mut self, accounts: &AddressMap<DbAccount>) -> B256 {
        if self.trie.is_none() {
            self.trie = Some(build_sparse_state_trie(accounts));
            self.dirty.clear();
            return self.trie.as_mut().unwrap().root().expect("state trie is revealed");
        }

        let trie = self.trie.as_mut().unwrap();
        for (address, dirty) in mem::take(&mut self.dirty) {
            let hashed_address = keccak256(address);
            let Some(account) = accounts.get(&address) else {
                update_leaf(trie.trie_mut(), hashed_address, Vec::new());
                continue;
            };

            let storage_trie = trie.get_or_create_storage_trie_mut(hashed_address);
            let storage = if dirty.reset_storage {
                storage_trie.wipe().expect("storage trie is revealed");
                account.storage.iter().map(|(slot, value)| (*slot, *value)).collect::<Vec<_>>()
            } else {
                dirty
                    .storage
                    .into_iter()
                    .map(|slot| (slot, account.storage.get(&slot).copied().unwrap_or_default()))
                    .collect::<Vec<_>>()
            };
            update_storage_leaves(storage_trie, storage);
            let storage_root = storage_trie.root().expect("storage trie is revealed");
            update_leaf(
                trie.trie_mut(),
                hashed_address,
                trie_account_rlp_with_storage_root(&account.info, storage_root),
            );
        }

        trie.root().expect("state trie is revealed")
    }
}

fn build_sparse_state_trie(accounts: &AddressMap<DbAccount>) -> SparseStateTrie {
    let mut trie = SparseStateTrie::new();
    trie.set_accounts_trie(RevealableSparseTrie::revealed_empty());
    trie.set_default_storage_trie(RevealableSparseTrie::revealed_empty());

    let mut account_leaves = B256Map::default();
    for (address, account) in accounts {
        let hashed_address = keccak256(address);
        let storage_trie = trie.get_or_create_storage_trie_mut(hashed_address);
        update_storage_leaves(
            storage_trie,
            account.storage.iter().map(|(slot, value)| (*slot, *value)),
        );
        let storage_root = storage_trie.root().expect("storage trie is revealed");
        account_leaves.insert(
            hashed_address,
            LeafUpdate::Changed(trie_account_rlp_with_storage_root(&account.info, storage_root)),
        );
    }
    update_leaves(trie.trie_mut(), &mut account_leaves);
    trie
}

fn update_storage_leaves(
    trie: &mut RevealableSparseTrie,
    storage: impl IntoIterator<Item = (U256, U256)>,
) {
    let mut leaves = storage
        .into_iter()
        .map(|(slot, value)| {
            (keccak256(slot.to_be_bytes::<32>()), LeafUpdate::Changed(alloy_rlp::encode(value)))
        })
        .collect();
    update_leaves(trie, &mut leaves);
}

fn update_leaf(trie: &mut RevealableSparseTrie, key: B256, value: Vec<u8>) {
    let mut leaves = B256Map::from_iter([(key, LeafUpdate::Changed(value))]);
    update_leaves(trie, &mut leaves);
}

fn update_leaves(trie: &mut RevealableSparseTrie, leaves: &mut B256Map<LeafUpdate>) {
    trie.update_leaves(leaves, |_, _| unreachable!("the complete trie is always revealed"))
        .expect("state trie update succeeds");
    debug_assert!(leaves.is_empty());
}

pub fn build_root(values: impl IntoIterator<Item = (Nibbles, Vec<u8>)>) -> B256 {
    let mut builder = HashBuilder::default();
    for (key, value) in values {
        builder.add_leaf(key, value.as_ref());
    }
    builder.root()
}

/// Builds state root from the given accounts
pub fn state_root(accounts: &AddressMap<DbAccount>) -> B256 {
    build_root(trie_accounts(accounts))
}

/// Builds storage root from the given storage
pub fn storage_root(storage: &U256Map<U256>) -> B256 {
    build_root(trie_storage(storage))
}

/// Builds iterator over stored key-value pairs ready for storage trie root calculation.
pub fn trie_storage(storage: &U256Map<U256>) -> Vec<(Nibbles, Vec<u8>)> {
    let mut storage = storage
        .iter()
        .map(|(key, value)| {
            let data = alloy_rlp::encode(value);
            (Nibbles::unpack(keccak256(key.to_be_bytes::<32>())), data)
        })
        .collect::<Vec<_>>();
    storage.sort_by_key(|(key, _)| *key);

    storage
}

/// Builds iterator over stored key-value pairs ready for account trie root calculation.
pub fn trie_accounts(accounts: &AddressMap<DbAccount>) -> Vec<(Nibbles, Vec<u8>)> {
    let mut accounts: Vec<(Nibbles, Vec<u8>)> = accounts
        .iter()
        .map(|(address, account)| {
            let data = trie_account_rlp(&account.info, &account.storage);
            (Nibbles::unpack(keccak256(*address)), data)
        })
        .collect();
    accounts.sort_by_key(|(key, _)| *key);

    accounts
}

/// Returns the RLP for this account.
pub fn trie_account_rlp(info: &AccountInfo, storage: &U256Map<U256>) -> Vec<u8> {
    trie_account_rlp_with_storage_root(info, storage_root(storage))
}

/// Returns the RLP for this account with an already computed storage root.
fn trie_account_rlp_with_storage_root(info: &AccountInfo, storage_root: B256) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let list: [&dyn Encodable; 4] = [&info.nonce, &info.balance, &storage_root, &info.code_hash];

    alloy_rlp::encode_list::<_, dyn Encodable>(&list, &mut out);

    out
}
