//! Support for generating the state root for memdb storage

use alloy_primitives::{
    B256, U256, keccak256,
    map::{AddressMap, B256Map, HashSet, U256Map},
};
use alloy_rlp::Encodable;
use alloy_trie::{
    EMPTY_ROOT_HASH, HashBuilder, Nibbles, TrieMask,
    nodes::{BranchNodeRef, ExtensionNodeRef, LeafNodeRef, RlpNode},
};
use revm::{
    database::{AccountState, DbAccount},
    state::{Account, AccountInfo},
};
use std::{array, mem};

/// Incrementally maintains the state trie used for mined block headers.
///
/// The old state-root path rebuilt and sorted every account and storage trie after each block.
/// That made EIP-2935's block-hash storage contract turn mining into linear work as its ring was
/// populated. This cache keeps an in-memory Merkle Patricia trie and only rehashes paths changed
/// by the latest block.
#[derive(Debug, Default)]
pub struct StateRootCache {
    trie: Option<IncrementalStateTrie>,
    dirty: AddressMap<DirtyAccount>,
    /// Reused while encoding dirty trie nodes.
    rlp_buf: Vec<u8>,
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
        let Self { trie, dirty, rlp_buf } = self;
        if trie.is_none() {
            *trie = Some(IncrementalStateTrie::from_accounts(accounts, rlp_buf));
            dirty.clear();
            return trie.as_mut().unwrap().root(rlp_buf);
        }

        let trie = trie.as_mut().unwrap();
        for (address, dirty) in mem::take(dirty) {
            let hashed_address = keccak256(address);
            let Some(account) = accounts.get(&address) else {
                trie.accounts.remove(hashed_address);
                trie.storage.remove(&hashed_address);
                continue;
            };
            if account.account_state == AccountState::NotExisting {
                trie.accounts.remove(hashed_address);
                trie.storage.remove(&hashed_address);
                continue;
            }

            let storage_trie = if dirty.reset_storage {
                trie.storage
                    .entry(hashed_address)
                    .insert_entry(IncrementalTrie::from_storage(&account.storage))
                    .into_mut()
            } else {
                let storage_trie = trie.storage.entry(hashed_address).or_default();
                for slot in dirty.storage {
                    let key = keccak256(slot.to_be_bytes::<32>());
                    if let Some(value) = account.storage.get(&slot)
                        && !value.is_zero()
                    {
                        storage_trie.insert(key, alloy_rlp::encode(value));
                    } else {
                        storage_trie.remove(key);
                    }
                }
                storage_trie
            };
            let storage_root = storage_trie.root_with_buf(rlp_buf);
            trie.accounts.insert(
                hashed_address,
                trie_account_rlp_with_storage_root(&account.info, storage_root),
            );
        }

        trie.root(rlp_buf)
    }
}

#[derive(Debug, Default)]
struct IncrementalStateTrie {
    accounts: IncrementalTrie,
    storage: B256Map<IncrementalTrie>,
}

impl IncrementalStateTrie {
    fn from_accounts(accounts: &AddressMap<DbAccount>, rlp_buf: &mut Vec<u8>) -> Self {
        let mut trie = Self::default();
        for (address, account) in accounts {
            if account.account_state == AccountState::NotExisting {
                continue;
            }
            let hashed_address = keccak256(address);
            let mut storage_trie = IncrementalTrie::from_storage(&account.storage);
            let storage_root = storage_trie.root_with_buf(rlp_buf);
            trie.accounts.insert(
                hashed_address,
                trie_account_rlp_with_storage_root(&account.info, storage_root),
            );
            trie.storage.insert(hashed_address, storage_trie);
        }
        trie
    }

    fn root(&mut self, rlp_buf: &mut Vec<u8>) -> B256 {
        self.accounts.root_with_buf(rlp_buf)
    }
}

/// A mutable Merkle Patricia trie that caches the RLP reference for every unchanged node.
#[derive(Debug, Default)]
struct IncrementalTrie {
    root: TrieNode,
}

impl IncrementalTrie {
    fn from_storage(storage: &U256Map<U256>) -> Self {
        let mut trie = Self::default();
        for (slot, value) in storage {
            if value.is_zero() {
                continue;
            }
            trie.insert(keccak256(slot.to_be_bytes::<32>()), alloy_rlp::encode(value));
        }
        trie
    }

    fn insert(&mut self, key: B256, value: Vec<u8>) {
        self.root.insert(Nibbles::unpack(key), value);
    }

    fn remove(&mut self, key: B256) {
        self.root.remove(Nibbles::unpack(key));
    }

    #[cfg(test)]
    fn root(&mut self) -> B256 {
        self.root_with_buf(&mut Vec::new())
    }

    fn root_with_buf(&mut self, rlp_buf: &mut Vec<u8>) -> B256 {
        let Some(root) = self.root.rlp(rlp_buf) else { return EMPTY_ROOT_HASH };
        root.as_hash().unwrap_or_else(|| keccak256(root.as_ref()))
    }
}

#[derive(Debug, Default)]
struct TrieNode {
    kind: TrieNodeKind,
    rlp: Option<RlpNode>,
}

#[derive(Debug, Default)]
enum TrieNodeKind {
    #[default]
    Empty,
    Leaf {
        path: Nibbles,
        value: Vec<u8>,
    },
    Extension {
        path: Nibbles,
        child: Box<TrieNode>,
    },
    Branch {
        children: [Option<Box<TrieNode>>; 16],
    },
}

impl TrieNode {
    const fn leaf(path: Nibbles, value: Vec<u8>) -> Self {
        Self { kind: TrieNodeKind::Leaf { path, value }, rlp: None }
    }

    fn extension(path: Nibbles, child: Self) -> Self {
        debug_assert!(!path.is_empty());
        Self { kind: TrieNodeKind::Extension { path, child: Box::new(child) }, rlp: None }
    }

    const fn branch(children: [Option<Box<Self>>; 16]) -> Self {
        Self { kind: TrieNodeKind::Branch { children }, rlp: None }
    }

    fn empty_children() -> [Option<Box<Self>>; 16] {
        Default::default()
    }

    fn insert(&mut self, key: Nibbles, value: Vec<u8>) {
        let kind = mem::take(&mut self.kind);
        self.rlp = None;
        *self = match kind {
            TrieNodeKind::Empty => Self::leaf(key, value),
            TrieNodeKind::Leaf { path, value: old_value } => {
                let common = path.common_prefix_length(&key);
                if common == path.len() {
                    debug_assert_eq!(common, key.len());
                    Self::leaf(path, value)
                } else {
                    let mut children = Self::empty_children();
                    let old_index = path.get(common).unwrap() as usize;
                    let new_index = key.get(common).unwrap() as usize;
                    children[old_index] =
                        Some(Box::new(Self::leaf(path.slice(common + 1..), old_value)));
                    children[new_index] =
                        Some(Box::new(Self::leaf(key.slice(common + 1..), value)));
                    let branch = Self::branch(children);
                    if common == 0 { branch } else { Self::extension(path.slice(..common), branch) }
                }
            }
            TrieNodeKind::Extension { path, mut child } => {
                let common = path.common_prefix_length(&key);
                if common == path.len() {
                    child.insert(key.slice(common..), value);
                    Self::extension(path, *child)
                } else {
                    let mut children = Self::empty_children();
                    let old_index = path.get(common).unwrap() as usize;
                    let old_path = path.slice(common + 1..);
                    let old_child = if old_path.is_empty() {
                        *child
                    } else {
                        Self::extension(old_path, *child)
                    };
                    children[old_index] = Some(Box::new(old_child));

                    let new_index = key.get(common).unwrap() as usize;
                    children[new_index] =
                        Some(Box::new(Self::leaf(key.slice(common + 1..), value)));
                    let branch = Self::branch(children);
                    if common == 0 { branch } else { Self::extension(path.slice(..common), branch) }
                }
            }
            TrieNodeKind::Branch { mut children } => {
                let index = key.first().expect("trie keys have equal lengths") as usize;
                children[index]
                    .get_or_insert_with(|| Box::new(Self::default()))
                    .insert(key.slice(1..), value);
                Self::branch(children)
            }
        };
    }

    fn remove(&mut self, key: Nibbles) {
        let kind = mem::take(&mut self.kind);
        self.rlp = None;
        *self = match kind {
            TrieNodeKind::Empty => Self::default(),
            TrieNodeKind::Leaf { path, value } => {
                if path == key {
                    Self::default()
                } else {
                    Self::leaf(path, value)
                }
            }
            TrieNodeKind::Extension { path, mut child } => {
                if key.starts_with(&path) {
                    child.remove(key.slice(path.len()..));
                    Self::normalize_extension(path, *child)
                } else {
                    Self::extension(path, *child)
                }
            }
            TrieNodeKind::Branch { mut children } => {
                let index = key.first().expect("trie keys have equal lengths") as usize;
                if let Some(child) = &mut children[index] {
                    child.remove(key.slice(1..));
                    if matches!(child.kind, TrieNodeKind::Empty) {
                        children[index] = None;
                    }
                }
                Self::normalize_branch(children)
            }
        };
    }

    fn normalize_extension(path: Nibbles, child: Self) -> Self {
        match child.kind {
            TrieNodeKind::Empty => Self::default(),
            TrieNodeKind::Leaf { path: child_path, value } => {
                Self::leaf(path.join(&child_path), value)
            }
            TrieNodeKind::Extension { path: child_path, child } => {
                Self::extension(path.join(&child_path), *child)
            }
            TrieNodeKind::Branch { children } => Self::extension(path, Self::branch(children)),
        }
    }

    fn normalize_branch(mut children: [Option<Box<Self>>; 16]) -> Self {
        let mut indexes =
            children.iter().enumerate().filter_map(|(index, child)| child.as_ref().map(|_| index));
        let Some(index) = indexes.next() else { return Self::default() };
        if indexes.next().is_some() {
            return Self::branch(children);
        }

        let child = *children[index].take().unwrap();
        let prefix = Nibbles::from_nibbles([index as u8]);
        match child.kind {
            TrieNodeKind::Empty => unreachable!("empty branch children are removed"),
            TrieNodeKind::Leaf { path, value } => Self::leaf(prefix.join(&path), value),
            TrieNodeKind::Extension { path, child } => Self::extension(prefix.join(&path), *child),
            TrieNodeKind::Branch { children } => Self::extension(prefix, Self::branch(children)),
        }
    }

    fn rlp(&mut self, out: &mut Vec<u8>) -> Option<RlpNode> {
        if let Some(rlp) = &self.rlp {
            return Some(rlp.clone());
        }

        let rlp = match &mut self.kind {
            TrieNodeKind::Empty => return None,
            TrieNodeKind::Leaf { path, value } => {
                out.clear();
                LeafNodeRef::new(path, value).rlp(out)
            }
            TrieNodeKind::Extension { path, child } => {
                let child = child.rlp(out).expect("extension nodes have a child");
                out.clear();
                ExtensionNodeRef::new(path, child.as_ref()).rlp(out)
            }
            TrieNodeKind::Branch { children } => {
                let mut stack: [RlpNode; 16] = array::from_fn(|_| RlpNode::default());
                let mut stack_len = 0;
                let mut state_mask = TrieMask::default();
                for (index, child) in children.iter_mut().enumerate() {
                    if let Some(child) = child {
                        stack[stack_len] = child.rlp(out).expect("branch children are not empty");
                        stack_len += 1;
                        state_mask.set_bit(index as u8);
                    }
                }
                out.clear();
                BranchNodeRef::new(&stack[..stack_len], state_mask).rlp(out)
            }
        };
        self.rlp = Some(rlp.clone());
        Some(rlp)
    }
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
        .filter(|(_, value)| !value.is_zero())
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
        .filter(|(_, account)| account.account_state != AccountState::NotExisting)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rebuilt_root(values: &B256Map<Vec<u8>>) -> B256 {
        let mut leaves = values
            .iter()
            .map(|(key, value)| (Nibbles::unpack(*key), value.clone()))
            .collect::<Vec<_>>();
        leaves.sort_by_key(|(key, _)| *key);
        build_root(leaves)
    }

    #[test]
    fn incremental_trie_matches_full_rebuild() {
        let mut trie = IncrementalTrie::default();
        let mut values = B256Map::default();
        assert_eq!(trie.root(), EMPTY_ROOT_HASH);

        for index in 0..128 {
            let key = keccak256(U256::from(index).to_be_bytes::<32>());
            let value = alloy_rlp::encode(U256::from(index + 1));
            trie.insert(key, value.clone());
            values.insert(key, value);
            assert_eq!(trie.root(), rebuilt_root(&values));
        }

        for index in (0..128).step_by(3) {
            let key = keccak256(U256::from(index).to_be_bytes::<32>());
            let value = alloy_rlp::encode(U256::from(index + 1_000));
            trie.insert(key, value.clone());
            values.insert(key, value);
            assert_eq!(trie.root(), rebuilt_root(&values));
        }

        for index in (0..128).rev() {
            let key = keccak256(U256::from(index).to_be_bytes::<32>());
            trie.remove(key);
            values.remove(&key);
            assert_eq!(trie.root(), rebuilt_root(&values));
        }
    }
}
