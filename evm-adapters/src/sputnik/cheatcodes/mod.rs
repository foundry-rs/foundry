pub mod memory_stackstate_owned;

pub mod cheatcode_handler;
use std::collections::HashMap;

pub use cheatcode_handler::CheatcodeHandler;

pub mod backend;

use ethers::types::{Address, H256, U256};
use sputnik::backend::{Backend, MemoryAccount, MemoryBackend};

#[derive(Clone, Debug, Default)]
/// Cheatcodes can be used to control the EVM context during setup or runtime,
/// which can be useful for simulations or specialized unit tests
pub struct Cheatcodes {
    pub block_number: Option<U256>,
    pub block_timestamp: Option<U256>,
    pub block_base_fee_per_gas: Option<U256>,
    pub accounts: HashMap<Address, MemoryAccount>,
}

pub trait BackendExt: Backend {
    fn set_storage(&mut self, address: Address, slot: H256, value: H256);
}

impl<'a> BackendExt for MemoryBackend<'a> {
    fn set_storage(&mut self, address: Address, slot: H256, value: H256) {
        let account = self.state_mut().entry(address).or_insert_with(Default::default);
        let slot = account.storage.entry(slot).or_insert_with(Default::default);
        *slot = value;
    }
}

ethers::contract::abigen!(
    HEVM,
    r#"[
            roll(uint256)
            warp(uint256)
            store(address,bytes32,bytes32)
            load(address,bytes32)(bytes32)
            ffi(string[])(bytes)
            addr(uint256)(address)
            sign(uint256,bytes32)(uint8,bytes32,bytes32)
            prank(address,address,bytes)(bool,bytes)
            deal(address,uint256)
    ]"#,
);
pub use hevm_mod::HEVMCalls;

ethers::contract::abigen!(
    HevmConsole,
    r#"[
            event log(string)
            event logs                   (bytes)
            event log_address            (address)
            event log_bytes32            (bytes32)
            event log_int                (int)
            event log_uint               (uint)
            event log_bytes              (bytes)
            event log_string             (string)
            event log_named_address      (string key, address val)
            event log_named_bytes32      (string key, bytes32 val)
            event log_named_decimal_int  (string key, int val, uint decimals)
            event log_named_decimal_uint (string key, uint val, uint decimals)
            event log_named_int          (string key, int val)
            event log_named_uint         (string key, uint val)
            event log_named_bytes        (string key, bytes val)
            event log_named_string       (string key, string val)
            ]"#,
);
