use crate::state::SharedAnvilState;
use alloy_primitives::{Address, B256, BlockNumber, Bytes, StorageKey, StorageValue};
use reth_ethereum::{
    primitives::{Account, Bytecode},
    storage::{
        AccountReader, BlockHashReader, BytecodeReader, HashedPostStateProvider,
        StateProofProvider, StateProvider, StateProviderBox, StateRootProvider,
        StorageRootProvider, errors::provider::ProviderResult,
    },
    trie::{
        AccountProof, DecodedMultiProofV2, ExecutionWitnessMode, HashedPostState, HashedStorage,
        MultiProof, MultiProofTargets, MultiProofTargetsV2, StorageMultiProof, StorageProof,
        TrieInput, updates::TrieUpdates,
    },
};
use revm::database::BundleState;
use std::fmt::{self, Debug, Formatter};

/// State provider that serves anvil state writes on top of the chain state.
pub struct AnvilStateProvider {
    state: SharedAnvilState,
    inner: StateProviderBox,
}

impl Debug for AnvilStateProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilStateProvider").finish_non_exhaustive()
    }
}

impl AnvilStateProvider {
    /// Wraps the given state provider.
    pub const fn new(state: SharedAnvilState, inner: StateProviderBox) -> Self {
        Self { state, inner }
    }
}

impl AccountReader for AnvilStateProvider {
    fn basic_account(&self, address: &Address) -> ProviderResult<Option<Account>> {
        let state = self.state.read();
        let Some(account_override) = state.account(address) else {
            return self.inner.basic_account(address);
        };

        let mut account = self.inner.basic_account(address)?.unwrap_or_default();
        if let Some(balance) = account_override.balance() {
            account.balance = balance;
        }
        if let Some(nonce) = account_override.nonce() {
            account.nonce = nonce;
        }
        if let Some(code_hash) = account_override.code_hash() {
            account.bytecode_hash = Some(code_hash);
        }
        Ok(Some(account))
    }
}

impl BytecodeReader for AnvilStateProvider {
    fn bytecode_by_hash(&self, code_hash: &B256) -> ProviderResult<Option<Bytecode>> {
        if let Some(bytecode) = self.state.read().bytecode_by_hash(code_hash) {
            return Ok(Some(bytecode.clone()));
        }
        self.inner.bytecode_by_hash(code_hash)
    }
}

impl StateProvider for AnvilStateProvider {
    fn storage(
        &self,
        account: Address,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let Some(value) = self.state.read().storage(&account, &storage_key) {
            return Ok(Some(value));
        }
        self.inner.storage(account, storage_key)
    }
}

impl BlockHashReader for AnvilStateProvider {
    fn block_hash(&self, number: BlockNumber) -> ProviderResult<Option<B256>> {
        self.inner.block_hash(number)
    }

    fn canonical_hashes_range(
        &self,
        start: BlockNumber,
        end: BlockNumber,
    ) -> ProviderResult<Vec<B256>> {
        self.inner.canonical_hashes_range(start, end)
    }
}

impl StateRootProvider for AnvilStateProvider {
    fn state_root(&self, hashed_state: HashedPostState) -> ProviderResult<B256> {
        self.inner.state_root(hashed_state)
    }

    fn state_root_from_nodes(&self, input: TrieInput) -> ProviderResult<B256> {
        self.inner.state_root_from_nodes(input)
    }

    fn state_root_with_updates(
        &self,
        hashed_state: HashedPostState,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        self.inner.state_root_with_updates(hashed_state)
    }

    fn state_root_from_nodes_with_updates(
        &self,
        input: TrieInput,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        self.inner.state_root_from_nodes_with_updates(input)
    }
}

impl StorageRootProvider for AnvilStateProvider {
    fn storage_root(
        &self,
        address: Address,
        hashed_storage: HashedStorage,
    ) -> ProviderResult<B256> {
        self.inner.storage_root(address, hashed_storage)
    }

    fn storage_proof(
        &self,
        address: Address,
        slot: B256,
        hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageProof> {
        self.inner.storage_proof(address, slot, hashed_storage)
    }

    fn storage_multiproof(
        &self,
        address: Address,
        slots: &[B256],
        hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageMultiProof> {
        self.inner.storage_multiproof(address, slots, hashed_storage)
    }
}

impl StateProofProvider for AnvilStateProvider {
    fn proof(
        &self,
        input: TrieInput,
        address: Address,
        slots: &[B256],
    ) -> ProviderResult<AccountProof> {
        self.inner.proof(input, address, slots)
    }

    fn multiproof(
        &self,
        input: TrieInput,
        targets: MultiProofTargets,
    ) -> ProviderResult<MultiProof> {
        self.inner.multiproof(input, targets)
    }

    fn multiproof_v2(
        &self,
        input: TrieInput,
        targets: MultiProofTargetsV2,
    ) -> ProviderResult<DecodedMultiProofV2> {
        self.inner.multiproof_v2(input, targets)
    }

    fn witness(
        &self,
        input: TrieInput,
        target: HashedPostState,
        mode: ExecutionWitnessMode,
    ) -> ProviderResult<Vec<Bytes>> {
        self.inner.witness(input, target, mode)
    }
}

impl HashedPostStateProvider for AnvilStateProvider {
    fn hashed_post_state(&self, bundle_state: &BundleState) -> ProviderResult<HashedPostState> {
        self.inner.hashed_post_state(bundle_state)
    }
}
