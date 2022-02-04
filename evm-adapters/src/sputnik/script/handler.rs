//! The handler that sits in between and intercepts script calls

use crate::sputnik::cheatcodes::memory_stackstate_owned::MemoryStackStateOwned;
use sputnik::{
    backend::Backend,
    executor::stack::{PrecompileSet, StackExecutor},
    Capture, Context, CreateScheme, ExitError, ExitReason, Handler, Transfer,
};
use std::convert::Infallible;

use ethers::{
    abi::{RawLog, Token},
    contract::EthLogDecode,
    core::{abi::AbiDecode, k256::ecdsa::SigningKey, utils},
    signers::{LocalWallet, Signer},
    solc::{artifacts::CompactContractBytecode, ProjectPathsConfig},
    types::{Address, H160, H256, U256},
};

pub type ScriptStackState<'config, Backend> = MemoryStackStateOwned<'config, Backend>;

pub type ScriptStackExecutor<'a, 'b, B, P> = StackExecutor<'a, 'b, ScriptStackState<'a, B>, P>;

pub struct ScriptHandler<H> {
    pub handler: H,
    state: ScriptState,
}

// Delegates everything internally, except the `call_inner` call, which is hooked
// so that we can modify
impl<'a, 'b, B: Backend, P: PrecompileSet> Handler
    for ScriptHandler<ScriptStackExecutor<'a, 'b, B, P>>
{
    type CreateInterrupt = Infallible;
    type CreateFeedback = Infallible;
    type CallInterrupt = Infallible;
    type CallFeedback = Infallible;

    // Everything else is left the same
    fn balance(&self, address: H160) -> U256 {
        self.handler.balance(address)
    }

    fn code_size(&self, address: H160) -> U256 {
        self.handler.code_size(address)
    }

    fn code_hash(&self, address: H160) -> H256 {
        self.handler.code_hash(address)
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.handler.code(address)
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        self.handler.storage(address, index)
    }

    fn original_storage(&self, address: H160, index: H256) -> H256 {
        self.handler.original_storage(address, index)
    }

    fn gas_left(&self) -> U256 {
        Handler::gas_left(&self.handler)
    }

    fn gas_price(&self) -> U256 {
        self.handler.gas_price()
    }

    fn origin(&self) -> H160 {
        self.handler.origin()
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.handler.block_hash(number)
    }

    fn block_number(&self) -> U256 {
        self.handler.block_number()
    }

    fn block_coinbase(&self) -> H160 {
        self.handler.block_coinbase()
    }

    fn block_timestamp(&self) -> U256 {
        self.handler.block_timestamp()
    }

    fn block_difficulty(&self) -> U256 {
        self.handler.block_difficulty()
    }

    fn block_gas_limit(&self) -> U256 {
        self.handler.block_gas_limit()
    }

    fn block_base_fee_per_gas(&self) -> U256 {
        self.handler.block_base_fee_per_gas()
    }

    fn chain_id(&self) -> U256 {
        self.handler.chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.handler.exists(address)
    }

    fn deleted(&self, address: H160) -> bool {
        self.handler.deleted(address)
    }

    fn is_cold(&self, address: H160, index: Option<H256>) -> bool {
        self.handler.is_cold(address, index)
    }

    fn set_storage(&mut self, address: H160, index: H256, value: H256) -> Result<(), ExitError> {
        self.handler.set_storage(address, index, value)
    }

    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) -> Result<(), ExitError> {
        self.handler.log(address, topics, data)
    }

    fn mark_delete(&mut self, address: H160, target: H160) -> Result<(), ExitError> {
        self.handler.mark_delete(address, target)
    }

    fn create(
        &mut self,
        caller: H160,
        scheme: CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        target_gas: Option<u64>,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Self::CreateInterrupt> {
        self.handler.create(caller, scheme, value, init_code, target_gas)
    }

    fn call(
        &mut self,
        code_address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        target_gas: Option<u64>,
        is_static: bool,
        context: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Self::CallInterrupt> {
        todo!("override script calls")
    }

    fn pre_validate(
        &mut self,
        context: &Context,
        opcode: sputnik::Opcode,
        stack: &sputnik::Stack,
    ) -> Result<(), ExitError> {
        self.handler.pre_validate(context, opcode, stack)
    }
}

/// Tracks the state of the script that's currently being executed
#[derive(Debug)]
pub struct ScriptState {
    // TODO file handles etc
}
