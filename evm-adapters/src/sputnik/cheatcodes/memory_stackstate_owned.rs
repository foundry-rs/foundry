use sputnik::{
    backend::{Backend, Basic},
    executor::stack::{MemoryStackSubstate, StackState, StackSubstateMetadata},
    ExitError, Transfer,
};

use ethers::types::{H160, H256, U256};

/// This struct implementation is copied from [upstream](https://github.com/rust-blockchain/evm/blob/5ecf36ce393380a89c6f1b09ef79f686fe043624/src/executor/stack/state.rs#L412) and modified to own the Backend type.
///
/// We had to copy it so that we can modify the Stack's internal backend, because
/// the upstream MemoryStackState only has an immutable reference to `Backend` which
/// does not allow us to do so.
#[derive(Clone)]
pub struct MemoryStackStateOwned<'config, B> {
    pub backend: B,
    pub substate: MemoryStackSubstate<'config>,
    pub expected_revert: Option<Vec<u8>>,
    pub next_msg_sender: Option<H160>,
    pub msg_sender: Option<(H160, H160, usize)>,
    pub all_logs: Vec<String>,
}

impl<'config, B: Backend> MemoryStackStateOwned<'config, B> {
    pub fn deposit(&mut self, address: H160, value: U256) {
        self.substate.deposit(address, value, &self.backend);
    }
}

impl<'config, B: Backend> MemoryStackStateOwned<'config, B> {
    pub fn new(metadata: StackSubstateMetadata<'config>, backend: B) -> Self {
        Self {
            backend,
            substate: MemoryStackSubstate::new(metadata),
            expected_revert: None,
            next_msg_sender: None,
            msg_sender: None,
            all_logs: Default::default(),
        }
    }
}

impl<'config, B: Backend> Backend for MemoryStackStateOwned<'config, B> {
    fn gas_price(&self) -> U256 {
        self.backend.gas_price()
    }
    fn origin(&self) -> H160 {
        self.backend.origin()
    }
    fn block_hash(&self, number: U256) -> H256 {
        self.backend.block_hash(number)
    }
    fn block_number(&self) -> U256 {
        self.backend.block_number()
    }
    fn block_coinbase(&self) -> H160 {
        self.backend.block_coinbase()
    }
    fn block_timestamp(&self) -> U256 {
        self.backend.block_timestamp()
    }
    fn block_difficulty(&self) -> U256 {
        self.backend.block_difficulty()
    }
    fn block_gas_limit(&self) -> U256 {
        self.backend.block_gas_limit()
    }
    fn block_base_fee_per_gas(&self) -> U256 {
        self.backend.block_base_fee_per_gas()
    }
    fn chain_id(&self) -> U256 {
        self.backend.chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.substate.known_account(address).is_some() || self.backend.exists(address)
    }

    fn basic(&self, address: H160) -> Basic {
        self.substate.known_basic(address).unwrap_or_else(|| self.backend.basic(address))
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.substate.known_code(address).unwrap_or_else(|| self.backend.code(address))
    }

    fn storage(&self, address: H160, key: H256) -> H256 {
        self.substate
            .known_storage(address, key)
            .unwrap_or_else(|| self.backend.storage(address, key))
    }

    fn original_storage(&self, address: H160, key: H256) -> Option<H256> {
        if let Some(value) = self.substate.known_original_storage(address, key) {
            return Some(value)
        }

        self.backend.original_storage(address, key)
    }
}

impl<'config, B: Backend> StackState<'config> for MemoryStackStateOwned<'config, B> {
    fn metadata(&self) -> &StackSubstateMetadata<'config> {
        self.substate.metadata()
    }

    fn metadata_mut(&mut self) -> &mut StackSubstateMetadata<'config> {
        self.substate.metadata_mut()
    }

    fn enter(&mut self, gas_limit: u64, is_static: bool) {
        self.substate.enter(gas_limit, is_static)
    }

    fn exit_commit(&mut self) -> Result<(), ExitError> {
        self.substate.exit_commit()
    }

    fn exit_revert(&mut self) -> Result<(), ExitError> {
        self.substate.exit_revert()
    }

    fn exit_discard(&mut self) -> Result<(), ExitError> {
        self.substate.exit_discard()
    }

    fn is_empty(&self, address: H160) -> bool {
        if let Some(known_empty) = self.substate.known_empty(address) {
            return known_empty
        }

        self.backend.basic(address).balance == U256::zero() &&
            self.backend.basic(address).nonce == U256::zero() &&
            self.backend.code(address).len() == 0
    }

    fn deleted(&self, address: H160) -> bool {
        self.substate.deleted(address)
    }

    fn is_cold(&self, address: H160) -> bool {
        self.substate.is_cold(address)
    }

    fn is_storage_cold(&self, address: H160, key: H256) -> bool {
        self.substate.is_storage_cold(address, key)
    }

    fn inc_nonce(&mut self, address: H160) {
        self.substate.inc_nonce(address, &self.backend);
    }

    fn set_storage(&mut self, address: H160, key: H256, value: H256) {
        self.substate.set_storage(address, key, value)
    }

    fn reset_storage(&mut self, address: H160) {
        self.substate.reset_storage(address, &self.backend);
    }

    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) {
        self.substate.log(address, topics, data);
    }

    fn set_deleted(&mut self, address: H160) {
        self.substate.set_deleted(address)
    }

    fn set_code(&mut self, address: H160, code: Vec<u8>) {
        self.substate.set_code(address, code, &self.backend)
    }

    fn transfer(&mut self, transfer: Transfer) -> Result<(), ExitError> {
        self.substate.transfer(transfer, &self.backend)
    }

    fn reset_balance(&mut self, address: H160) {
        self.substate.reset_balance(address, &self.backend)
    }

    fn touch(&mut self, address: H160) {
        self.substate.touch(address, &self.backend)
    }
}
