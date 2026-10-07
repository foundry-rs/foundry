//! Tempo precompile storage over a buffer of writes, for code that runs Tempo's precompile logic
//! outside of a transaction: the genesis of a Tempo dev chain, and the anvil methods that set
//! fee tokens, fee token balances, and Fee AMM liquidity.

use alloy_primitives::{Address, B256, LogData, U256};
use reth_ethereum::storage::StateProvider;
use revm::{
    context::{BlockEnv, journaled_state::JournalCheckpoint},
    state::{AccountInfo, Bytecode},
};
use std::collections::{BTreeMap, HashMap};
use tempo_hardfork::TempoHardfork;
use tempo_precompiles::{error::TempoPrecompileError, storage::PrecompileStorageProvider};
use tempo_primitives::TempoBlockEnv;

/// The writes to one account.
#[derive(Debug, Default)]
pub struct AccountWrites {
    /// The new code, if set.
    pub code: Option<Bytecode>,
    /// The written storage slots, zero values included.
    pub storage: BTreeMap<U256, U256>,
}

/// Tempo precompile storage that buffers its writes over an optional state to read from.
pub struct TempoStorage<'a> {
    base: Option<&'a dyn StateProvider>,
    writes: BTreeMap<Address, AccountWrites>,
    chain_id: u64,
    block_env: TempoBlockEnv,
    hardfork: TempoHardfork,
    transient: HashMap<(Address, U256), U256>,
    gas_used: u64,
    gas_refunded: i64,
}

impl<'a> TempoStorage<'a> {
    /// Creates the storage over `base`, or over an empty state without one, at the given block.
    pub fn new(
        base: Option<&'a dyn StateProvider>,
        chain_id: u64,
        block_number: u64,
        timestamp: u64,
        hardfork: TempoHardfork,
    ) -> Self {
        Self {
            base,
            writes: BTreeMap::new(),
            chain_id,
            block_env: TempoBlockEnv {
                inner: BlockEnv {
                    number: U256::from(block_number),
                    timestamp: U256::from(timestamp),
                    ..Default::default()
                },
                ..Default::default()
            },
            hardfork,
            transient: HashMap::new(),
            gas_used: 0,
            gas_refunded: 0,
        }
    }

    /// Returns the buffered writes.
    pub fn into_writes(self) -> BTreeMap<Address, AccountWrites> {
        self.writes
    }

    /// Reads the account from the writes, then from the base state.
    fn account_info(&self, address: Address) -> Result<Option<AccountInfo>, TempoPrecompileError> {
        let written_code = self.writes.get(&address).and_then(|writes| writes.code.clone());
        let base = match self.base {
            Some(base) => base.basic_account(&address).map_err(fatal)?,
            None => None,
        };
        Ok(match (base, written_code) {
            (None, None) => None,
            (Some(account), None) => {
                let code = match account.bytecode_hash {
                    Some(hash) => self
                        .base
                        .map(|base| base.bytecode_by_hash(&hash))
                        .transpose()
                        .map_err(fatal)?
                        .flatten()
                        .map(|code| code.0),
                    None => None,
                };
                Some(AccountInfo {
                    balance: account.balance,
                    nonce: account.nonce,
                    code_hash: account.get_bytecode_hash(),
                    code,
                    ..Default::default()
                })
            }
            (base, Some(code)) => {
                let (balance, nonce) =
                    base.map(|account| (account.balance, account.nonce)).unwrap_or_default();
                Some(AccountInfo {
                    balance,
                    nonce,
                    code_hash: code.hash_slow(),
                    code: Some(code),
                    ..Default::default()
                })
            }
        })
    }
}

/// Wraps an error as a fatal precompile error.
fn fatal(error: impl std::fmt::Display) -> TempoPrecompileError {
    TempoPrecompileError::Fatal(error.to_string())
}

impl PrecompileStorageProvider for TempoStorage<'_> {
    fn spec(&self) -> TempoHardfork {
        self.hardfork
    }

    fn chain_id(&self) -> u64 {
        self.chain_id
    }

    fn block_env(&self) -> &TempoBlockEnv {
        &self.block_env
    }

    fn set_code(&mut self, address: Address, code: Bytecode) -> Result<(), TempoPrecompileError> {
        self.writes.entry(address).or_default().code = Some(code);
        Ok(())
    }

    fn with_account_info(
        &mut self,
        address: Address,
        f: &mut dyn FnMut(&AccountInfo),
    ) -> Result<(), TempoPrecompileError> {
        // A missing account reads as empty, as in the EVM.
        f(&self.account_info(address)?.unwrap_or_default());
        Ok(())
    }

    fn account_code(&mut self, address: Address) -> Result<(B256, Bytecode), TempoPrecompileError> {
        Ok(self.account_info(address)?.map_or_else(
            || (B256::ZERO, Bytecode::default()),
            |info| (info.code_hash, info.code.unwrap_or_default()),
        ))
    }

    fn sload(&mut self, address: Address, key: U256) -> Result<U256, TempoPrecompileError> {
        if let Some(value) = self.writes.get(&address).and_then(|writes| writes.storage.get(&key)) {
            return Ok(*value);
        }
        match self.base {
            Some(base) => {
                Ok(base.storage(address, B256::from(key)).map_err(fatal)?.unwrap_or_default())
            }
            None => Ok(U256::ZERO),
        }
    }

    fn sstore(
        &mut self,
        address: Address,
        key: U256,
        value: U256,
    ) -> Result<(), TempoPrecompileError> {
        self.writes.entry(address).or_default().storage.insert(key, value);
        Ok(())
    }

    fn tstore(
        &mut self,
        address: Address,
        key: U256,
        value: U256,
    ) -> Result<(), TempoPrecompileError> {
        self.transient.insert((address, key), value);
        Ok(())
    }

    fn tload(&mut self, address: Address, key: U256) -> Result<U256, TempoPrecompileError> {
        Ok(self.transient.get(&(address, key)).copied().unwrap_or_default())
    }

    fn emit_event(
        &mut self,
        _address: Address,
        _event: LogData,
    ) -> Result<(), TempoPrecompileError> {
        Ok(())
    }

    fn deduct_gas(&mut self, gas: u64) -> Result<(), TempoPrecompileError> {
        self.gas_used = self.gas_used.saturating_add(gas);
        Ok(())
    }

    fn gas_used(&self) -> u64 {
        self.gas_used
    }

    fn state_gas_used(&self) -> u64 {
        0
    }

    fn state_gas_spilled(&self) -> u64 {
        0
    }

    fn gas_limit(&self) -> u64 {
        u64::MAX
    }

    fn gas_refunded(&self) -> i64 {
        self.gas_refunded
    }

    fn reservoir(&self) -> u64 {
        0
    }

    fn refund_gas(&mut self, gas: i64) {
        self.gas_refunded = self.gas_refunded.saturating_add(gas);
    }

    fn is_static(&self) -> bool {
        false
    }

    fn checkpoint(&mut self) -> JournalCheckpoint {
        JournalCheckpoint { log_i: 0, journal_i: 0, selfdestructed_i: 0 }
    }

    fn checkpoint_commit(&mut self, _checkpoint: JournalCheckpoint) {}

    fn checkpoint_revert(&mut self, _checkpoint: JournalCheckpoint) {}

    fn amsterdam_eip8037_enabled(&self) -> bool {
        false
    }

    // Gas is not charged outside of a transaction, so storage credits do not matter.
    fn set_tip1060_storage_credits(&mut self, _enabled: bool) {}
}
