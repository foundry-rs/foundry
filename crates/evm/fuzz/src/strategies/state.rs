use crate::{BasicTxDetails, invariant::FuzzRunIdentifiedContracts};
use alloy_dyn_abi::{DynSolType, DynSolValue, EventExt, FunctionExt};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{
    Address, B256, Bytes, I256, Log, U256, keccak256,
    map::{AddressIndexSet, AddressMap, B256IndexSet, HashMap, IndexSet},
};
use foundry_common::{
    ignore_metadata_hash, mapping_slots::MappingSlots, slot_identifier::SlotIdentifier,
};
use foundry_compilers::{ProjectPathsConfig, artifacts::StorageLayout};
use foundry_config::FuzzDictionaryConfig;
use foundry_evm_core::{bytecode::InstIter, utils::StateChangeset};
use parking_lot::{RawRwLock, RwLock, lock_api::RwLockReadGuard};
use revm::{
    database::{CacheDB, DatabaseRef, DbAccount},
    state::AccountInfo,
};
use solar::{
    ast::{self, Visit},
    interface::source_map::FileName,
    sema::Compiler,
};
use std::{collections::BTreeMap, fmt, ops::ControlFlow, sync::Arc};

/// The maximum number of bytes we will look at in bytecodes to find push bytes (24 KiB).
///
/// This is to limit the performance impact of fuzz tests that might deploy arbitrarily sized
/// bytecode (as is the case with Solmate).
const PUSH_BYTE_ANALYSIS_LIMIT: usize = 24 * 1024;

/// A set of arbitrary 32 byte data from the VM used to generate values for the strategy.
///
/// Wrapped in a shareable container.
#[derive(Clone, Debug)]
pub struct EvmFuzzState {
    inner: Arc<RwLock<FuzzDictionary>>,
    /// Addresses of external libraries deployed in test setup, excluded from fuzz test inputs.
    pub deployed_libs: Vec<Address>,
    /// Records mapping accesses. Used to identify storage slots belonging to mappings and sampling
    /// the values in the [`FuzzDictionary`].
    ///
    /// Only needed when [`StorageLayout`] is available.
    pub(crate) mapping_slots: Option<AddressMap<MappingSlots>>,
}

impl EvmFuzzState {
    pub fn new<DB: DatabaseRef>(
        db: &CacheDB<DB>,
        config: FuzzDictionaryConfig,
        deployed_libs: &[Address],
        analysis: Option<&Arc<Compiler>>,
        paths_config: Option<&ProjectPathsConfig>,
    ) -> Self {
        // Sort accounts to ensure deterministic dictionary generation from the same setUp state.
        let mut accs = db.cache.accounts.iter().collect::<Vec<_>>();
        accs.sort_by_key(|(address, _)| *address);

        // Create fuzz dictionary and insert values from db state.
        let mut dictionary = FuzzDictionary::new(config);
        dictionary.insert_db_values(accs);

        // Seed dict with AST literals if analysis is available.
        if let Some(compiler) = analysis {
            let literals = LiteralsCollector::process(
                compiler,
                paths_config,
                config.max_fuzz_dictionary_literals,
            );
            dictionary.sample_values = literals.words;
            dictionary.string_literals = literals.strings;
            dictionary.byte_literals = literals.bytes;
            trace!("inserted AST literals into fuzz dictionary");
        }

        Self {
            inner: Arc::new(RwLock::new(dictionary)),
            deployed_libs: deployed_libs.to_vec(),
            mapping_slots: None,
        }
    }

    pub fn with_mapping_slots(mut self, mapping_slots: AddressMap<MappingSlots>) -> Self {
        self.mapping_slots = Some(mapping_slots);
        self
    }

    pub fn collect_values(&self, values: impl IntoIterator<Item = B256>) {
        let mut dict = self.inner.write();
        for value in values {
            dict.insert_value(value);
        }
    }

    /// Collects state changes from a [StateChangeset] and logs into an [EvmFuzzState] according to
    /// the given [FuzzDictionaryConfig].
    pub fn collect_values_from_call(
        &self,
        fuzzed_contracts: &FuzzRunIdentifiedContracts,
        tx: &BasicTxDetails,
        result: &Bytes,
        logs: &[Log],
        state_changeset: &StateChangeset,
        run_depth: u32,
    ) {
        let mut dict = self.inner.write();
        {
            let targets = fuzzed_contracts.targets.lock();
            let (target_abi, target_function) = targets.fuzzed_artifacts(tx);
            dict.insert_logs_values(target_abi, logs, run_depth);
            dict.insert_result_values(target_function, result, run_depth);
            // Get storage layouts for contracts in the state changeset
            let storage_layouts = targets.get_storage_layouts();
            dict.insert_new_state_values(
                state_changeset,
                &storage_layouts,
                self.mapping_slots.as_ref(),
            );
        }
    }

    /// Removes all newly added entries from the dictionary.
    ///
    /// Should be called between fuzz/invariant runs to avoid accumulating data derived from fuzz
    /// inputs.
    pub fn revert(&self) {
        self.inner.write().revert();
    }

    pub fn dictionary_read(&self) -> RwLockReadGuard<'_, RawRwLock, FuzzDictionary> {
        self.inner.read()
    }

    /// Logs stats about the current state.
    pub fn log_stats(&self) {
        self.inner.read().log_stats();
    }

    #[cfg(test)]
    /// Test-only helper to seed the dictionary with literal values.
    pub(crate) fn seed_literals(&self, map: LiteralMaps) {
        self.inner.write().seed_literals(map);
    }
}

// We're using `IndexSet` to have a stable element order when restoring persisted state, as well as
// for performance when iterating over the sets.
#[derive(Default)]
pub struct FuzzDictionary {
    /// Collected state values.
    state_values: B256IndexSet,
    /// Addresses that already had their PUSH bytes collected.
    addresses: AddressIndexSet,
    /// Configuration for the dictionary.
    config: FuzzDictionaryConfig,
    /// Number of state values initially collected from db.
    /// Used to revert new collected values at the end of each run.
    db_state_values: usize,
    /// Number of address values initially collected from db.
    /// Used to revert new collected addresses at the end of each run.
    db_addresses: usize,
    /// Typed runtime sample values persisted across invariant runs.
    /// Initially seeded with literal values collected from the source code.
    sample_values: HashMap<DynSolType, B256IndexSet>,
    /// String literals collected from source code. Never reverted.
    string_literals: IndexSet<String>,
    /// Byte literals (hex"...") collected from source code. Never reverted.
    byte_literals: IndexSet<Bytes>,

    misses: usize,
    hits: usize,
}

impl fmt::Debug for FuzzDictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FuzzDictionary")
            .field("state_values", &self.state_values.len())
            .field("addresses", &self.addresses)
            .finish()
    }
}

impl FuzzDictionary {
    pub fn new(config: FuzzDictionaryConfig) -> Self {
        let mut dictionary = Self { config, ..Default::default() };
        dictionary.prefill();
        dictionary
    }

    /// Insert common values into the dictionary at initialization.
    fn prefill(&mut self) {
        self.insert_value(B256::ZERO);
    }

    /// Insert values from initial db state into fuzz dictionary.
    /// These values are persisted across invariant runs.
    fn insert_db_values(&mut self, db_state: Vec<(&Address, &DbAccount)>) {
        for (address, account) in db_state {
            // Insert basic account information
            self.insert_value(address.into_word());
            // Insert push bytes
            self.insert_push_bytes_values(address, &account.info);
            // Insert storage values.
            if self.config.include_storage {
                // Sort storage values before inserting to ensure deterministic dictionary.
                let values = account.storage.iter().collect::<BTreeMap<_, _>>();
                for (slot, value) in values {
                    self.insert_storage_value(slot, value, None, None);
                }
            }
        }

        // We need at least some state data if DB is empty,
        // otherwise we can't select random data for state fuzzing.
        if self.values().is_empty() {
            // Prefill with a random address.
            self.insert_value(Address::random().into_word());
        }

        // Record number of values and addresses inserted from db to be used for reverting at the
        // end of each run.
        self.db_state_values = self.state_values.len();
        self.db_addresses = self.addresses.len();
    }

    /// Insert values collected from call result into fuzz dictionary.
    fn insert_result_values(
        &mut self,
        function: Option<&Function>,
        result: &Bytes,
        run_depth: u32,
    ) {
        if let Some(function) = function
            && !function.outputs.is_empty()
        {
            // Decode result and collect samples to be used in subsequent fuzz runs.
            if let Ok(decoded_result) = function.abi_decode_output(result) {
                self.insert_sample_values(decoded_result, run_depth);
            }
        }
    }

    /// Insert values from call log topics and data into fuzz dictionary.
    fn insert_logs_values(&mut self, abi: Option<&JsonAbi>, logs: &[Log], run_depth: u32) {
        let mut samples = Vec::new();
        // Decode logs with known events and collect samples from indexed fields and event body.
        for log in logs {
            let mut log_decoded = false;
            // Try to decode log with events from contract abi.
            if let Some(abi) = abi {
                for event in abi.events() {
                    if let Ok(decoded_event) = event.decode_log(log) {
                        samples.extend(decoded_event.indexed);
                        samples.extend(decoded_event.body);
                        log_decoded = true;
                        break;
                    }
                }
            }

            // If we weren't able to decode event then we insert raw data in fuzz dictionary.
            if !log_decoded {
                for &topic in log.topics() {
                    self.insert_value(topic);
                }
                let chunks = log.data.data.chunks_exact(32);
                let rem = chunks.remainder();
                for chunk in chunks {
                    self.insert_value(chunk.try_into().unwrap());
                }
                if !rem.is_empty() {
                    self.insert_value(B256::right_padding_from(rem));
                }
            }
        }

        // Insert samples collected from current call in fuzz dictionary.
        self.insert_sample_values(samples, run_depth);
    }

    /// Insert values from call state changeset into fuzz dictionary.
    /// These values are removed at the end of current run.
    fn insert_new_state_values(
        &mut self,
        state_changeset: &StateChangeset,
        storage_layouts: &HashMap<Address, Arc<StorageLayout>>,
        mapping_slots: Option<&AddressMap<MappingSlots>>,
    ) {
        for (address, account) in state_changeset {
            // Insert basic account information.
            self.insert_value(address.into_word());
            // Insert push bytes.
            self.insert_push_bytes_values(address, &account.info);
            // Insert storage values.
            if self.config.include_storage {
                let storage_layout = storage_layouts.get(address).cloned();
                trace!(
                    "{address:?} has mapping_slots {}",
                    mapping_slots.is_some_and(|m| m.contains_key(address))
                );
                let mapping_slots = mapping_slots.and_then(|m| m.get(address));
                for (slot, value) in &account.storage {
                    self.insert_storage_value(
                        slot,
                        &value.present_value,
                        storage_layout.as_deref(),
                        mapping_slots,
                    );
                }
            }
        }
    }

    /// Insert values from push bytes into fuzz dictionary.
    /// Values are collected only once for a given address.
    /// If values are newly collected then they are removed at the end of current run.
    fn insert_push_bytes_values(&mut self, address: &Address, account_info: &AccountInfo) {
        if self.config.include_push_bytes
            && !self.addresses.contains(address)
            && let Some(code) = &account_info.code
        {
            self.insert_address(*address);
            if !self.values_full() {
                self.collect_push_bytes(ignore_metadata_hash(code.original_byte_slice()));
            }
        }
    }

    fn collect_push_bytes(&mut self, code: &[u8]) {
        let len = code.len().min(PUSH_BYTE_ANALYSIS_LIMIT);
        let code = &code[..len];
        for inst in InstIter::new(code) {
            // Don't add 0 to the dictionary as it's already present.
            if !inst.immediate.is_empty()
                && let Some(push_value) = U256::try_from_be_slice(inst.immediate)
                && push_value != U256::ZERO
            {
                self.insert_value_u256(push_value);
            }
        }
    }

    /// Insert values from single storage slot and storage value into fuzz dictionary.
    /// Uses [`SlotIdentifier`] to identify storage slots types.
    fn insert_storage_value(
        &mut self,
        slot: &U256,
        value: &U256,
        layout: Option<&StorageLayout>,
        mapping_slots: Option<&MappingSlots>,
    ) {
        let slot = B256::from(*slot);
        let value = B256::from(*value);

        // Always insert the slot itself
        self.insert_value(slot);

        // If we have a storage layout, use SlotIdentifier for better type identification
        if let Some(slot_identifier) =
            layout.map(|l| SlotIdentifier::new(l.clone().into()))
            // Identify Slot Type
            && let Some(slot_info) = slot_identifier.identify(&slot, mapping_slots) && slot_info.decode(value).is_some()
        {
            trace!(?slot_info, "inserting typed storage value");
            self.sample_values.entry(slot_info.slot_type.dyn_sol_type).or_default().insert(value);
        } else {
            self.insert_value_u256(value.into());
        }
    }

    /// Insert address into fuzz dictionary.
    /// If address is newly collected then it is removed by index at the end of current run.
    fn insert_address(&mut self, address: Address) {
        if self.addresses.len() < self.config.max_fuzz_dictionary_addresses {
            self.addresses.insert(address);
        }
    }

    /// Insert raw value into fuzz dictionary.
    ///
    /// If value is newly collected then it is removed by index at the end of current run.
    ///
    /// Returns true if the value was inserted.
    fn insert_value(&mut self, value: B256) -> bool {
        let insert = !self.values_full();
        if insert {
            let new_value = self.state_values.insert(value);
            let counter = if new_value { &mut self.misses } else { &mut self.hits };
            *counter += 1;
        }
        insert
    }

    fn insert_value_u256(&mut self, value: U256) -> bool {
        // Also add the value below and above the push value to the dictionary.
        let one = U256::from(1);
        self.insert_value(value.into())
            | self.insert_value((value.wrapping_sub(one)).into())
            | self.insert_value((value.wrapping_add(one)).into())
    }

    fn values_full(&self) -> bool {
        self.state_values.len() >= self.config.max_fuzz_dictionary_values
    }

    /// Insert sample values that are reused across multiple runs.
    /// The number of samples is limited to invariant run depth.
    /// If collected samples limit is reached then values are inserted as regular values.
    pub fn insert_sample_values(
        &mut self,
        sample_values: impl IntoIterator<Item = DynSolValue>,
        limit: u32,
    ) {
        for sample in sample_values {
            if let (Some(sample_type), Some(sample_value)) = (sample.as_type(), sample.as_word()) {
                if let Some(values) = self.sample_values.get_mut(&sample_type) {
                    if values.len() < limit as usize {
                        values.insert(sample_value);
                    } else {
                        // Insert as state value (will be removed at the end of the run).
                        self.insert_value(sample_value);
                    }
                } else {
                    self.sample_values.entry(sample_type).or_default().insert(sample_value);
                }
            }
        }
    }

    pub fn values(&self) -> &B256IndexSet {
        &self.state_values
    }

    pub fn len(&self) -> usize {
        self.state_values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.state_values.is_empty()
    }

    #[inline]
    pub fn samples(&self, param_type: &DynSolType) -> Option<&B256IndexSet> {
        self.sample_values.get(param_type)
    }

    /// Returns the collected AST strings.
    #[inline]
    pub fn ast_strings(&self) -> &IndexSet<String> {
        &self.string_literals
    }

    /// Returns the collected AST bytes (hex literals).
    #[inline]
    pub fn ast_bytes(&self) -> &IndexSet<Bytes> {
        &self.byte_literals
    }

    #[inline]
    pub fn addresses(&self) -> &AddressIndexSet {
        &self.addresses
    }

    /// Revert values and addresses collected during the run by truncating to initial db len.
    pub fn revert(&mut self) {
        self.state_values.truncate(self.db_state_values);
        self.addresses.truncate(self.db_addresses);
    }

    pub fn log_stats(&self) {
        trace!(
            addresses.len = self.addresses.len(),
            ast_string.len = self.string_literals.len(),
            sample.len = self.sample_values.len(),
            state.len = self.state_values.len(),
            state.misses = self.misses,
            state.hits = self.hits,
            "FuzzDictionary stats",
        );
    }

    #[cfg(test)]
    /// Test-only helper to seed the dictionary with literal values.
    pub(crate) fn seed_literals(&mut self, map: LiteralMaps) {
        self.string_literals = map.strings;
        self.byte_literals = map.bytes;
        self.sample_values = map.words;
    }
}

// -- AST LITERALS COLLECTOR ---------------------------------------------------

#[derive(Debug, Default)]
pub struct LiteralMaps {
    pub words: HashMap<DynSolType, B256IndexSet>,
    pub strings: IndexSet<String>,
    pub bytes: IndexSet<Bytes>,
}

#[derive(Debug, Default)]
struct LiteralsCollector {
    max_values: usize,
    total_values: usize,
    output: LiteralMaps,
}

impl LiteralsCollector {
    fn new(max_values: usize) -> Self {
        Self { max_values, ..Default::default() }
    }

    fn process(
        compiler: &Arc<Compiler>,
        paths_config: Option<&ProjectPathsConfig>,
        max_values: usize,
    ) -> LiteralMaps {
        compiler.enter(|compiler| {
            let mut literals_collector = Self::new(max_values);
            for source in compiler.sources().iter() {
                // Ignore scripts, and libs
                if let Some(paths) = paths_config
                    && let FileName::Real(source_path) = &source.file.name
                    && !(source_path.starts_with(&paths.sources) || paths.is_test(source_path))
                {
                    continue;
                }

                if let Some(ref ast) = source.ast {
                    let _ = literals_collector.visit_source_unit(ast);
                }
            }

            literals_collector.output
        })
    }
}

impl<'ast> ast::Visit<'ast> for LiteralsCollector {
    type BreakValue = ();

    fn visit_expr(&mut self, expr: &'ast ast::Expr<'ast>) -> ControlFlow<()> {
        // Stop early if we've hit the limit
        if self.total_values >= self.max_values {
            return ControlFlow::Break(());
        }

        // Handle unary negation of number literals
        if let ast::ExprKind::Unary(un_op, inner_expr) = &expr.kind
            && un_op.kind == ast::UnOpKind::Neg
            && let ast::ExprKind::Lit(lit, _) = &inner_expr.kind
            && let ast::LitKind::Number(n) = &lit.kind
        {
            // Compute the negative I256 value
            if let Ok(pos_i256) = I256::try_from(*n) {
                let neg_value = -pos_i256;
                let neg_b256 = B256::from(neg_value.into_raw());

                // Store under all intN sizes that can represent this value
                for bits in [16, 32, 64, 128, 256] {
                    if can_fit_int(neg_value, bits)
                        && self
                            .output
                            .words
                            .entry(DynSolType::Int(bits))
                            .or_default()
                            .insert(neg_b256)
                    {
                        self.total_values += 1;
                    }
                }
            }

            // Continue walking the expression
            return self.walk_expr(expr);
        }

        // Handle literals
        if let ast::ExprKind::Lit(lit, _) = &expr.kind {
            let is_new = match &lit.kind {
                ast::LitKind::Number(n) => {
                    let pos_value = U256::from(*n);
                    let pos_b256 = B256::from(pos_value);

                    // Store under all uintN sizes that can represent this value
                    for bits in [8, 16, 32, 64, 128, 256] {
                        if can_fit_uint(pos_value, bits)
                            && self
                                .output
                                .words
                                .entry(DynSolType::Uint(bits))
                                .or_default()
                                .insert(pos_b256)
                        {
                            self.total_values += 1;
                        }
                    }
                    false // already handled inserts individually
                }
                ast::LitKind::Address(addr) => self
                    .output
                    .words
                    .entry(DynSolType::Address)
                    .or_default()
                    .insert(addr.into_word()),
                ast::LitKind::Str(ast::StrKind::Hex, sym, _) => {
                    self.output.bytes.insert(Bytes::copy_from_slice(sym.as_byte_str()))
                }
                ast::LitKind::Str(_, sym, _) => {
                    let s = String::from_utf8_lossy(sym.as_byte_str()).into_owned();
                    // For strings, also store the hashed version
                    let hash = keccak256(s.as_bytes());
                    if self.output.words.entry(DynSolType::FixedBytes(32)).or_default().insert(hash)
                    {
                        self.total_values += 1;
                    }
                    // And the right-padded version if it fits.
                    if s.len() <= 32 {
                        let padded = B256::right_padding_from(s.as_bytes());
                        if self
                            .output
                            .words
                            .entry(DynSolType::FixedBytes(32))
                            .or_default()
                            .insert(padded)
                        {
                            self.total_values += 1;
                        }
                    }
                    self.output.strings.insert(s)
                }
                ast::LitKind::Bool(..) | ast::LitKind::Rational(..) | ast::LitKind::Err(..) => {
                    false // ignore
                }
            };

            if is_new {
                self.total_values += 1;
            }
        }

        self.walk_expr(expr)
    }
}

/// Checks if a signed integer value can fit in intN type.
fn can_fit_int(value: I256, bits: usize) -> bool {
    // Calculate the maximum positive value for intN: 2^(N-1) - 1
    let max_val = I256::try_from((U256::from(1) << (bits - 1)) - U256::from(1))
        .expect("max value should fit in I256");
    // Calculate the minimum negative value for intN: -2^(N-1)
    let min_val = -max_val - I256::ONE;

    value >= min_val && value <= max_val
}

/// Checks if an unsigned integer value can fit in uintN type.
fn can_fit_uint(value: U256, bits: usize) -> bool {
    if bits == 256 {
        return true;
    }
    // Calculate the maximum value for uintN: 2^N - 1
    let max_val = (U256::from(1) << bits) - U256::from(1);
    value <= max_val
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use solar::interface::{Session, source_map};

    const SOURCE: &str = r#"
    contract Magic {
        // plain literals
        address constant DAI = 0x6B175474E89094C44Da98b954EedeAC495271d0F;
        uint64 constant MAGIC_NUMBER = 1122334455;
        int32 constant MAGIC_INT = -777;
        bytes32 constant MAGIC_WORD = "abcd1234";
        bytes constant MAGIC_BYTES = hex"deadbeef";
        string constant MAGIC_STRING = "xyzzy";

        // constant exprs with folding
        uint256 constant NEG_FOLDING = uint(-2);
        uint256 constant BIN_FOLDING = 2 * 2 ether;
        bytes32 constant IMPLEMENTATION_SLOT = bytes32(uint256(keccak256('eip1967.proxy.implementation')) - 1);
    }"#;

    #[test]
    fn test_literals_collector_coverage() {
        let map = process_source_literals(SOURCE);

        // Expected values from the SOURCE contract
        let addr = address!("0x6B175474E89094C44Da98b954EedeAC495271d0F").into_word();
        let num = B256::from(U256::from(1122334455u64));
        let int = B256::from(I256::try_from(-777i32).unwrap().into_raw());
        let word = B256::right_padding_from(b"abcd1234");
        let dyn_bytes = Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]);

        assert_word(&map, DynSolType::Address, addr, "Expected DAI in address set");
        assert_word(&map, DynSolType::Uint(64), num, "Expected MAGIC_NUMBER in uint64 set");
        assert_word(&map, DynSolType::Int(32), int, "Expected MAGIC_INT in int32 set");
        assert_word(&map, DynSolType::FixedBytes(32), word, "Expected MAGIC_WORD in bytes32 set");
        assert!(map.strings.contains("xyzzy"), "Expected MAGIC_STRING to be collected");
        assert!(
            map.strings.contains("eip1967.proxy.implementation"),
            "Expected IMPLEMENTATION_SLOT in string set"
        );
        assert!(map.bytes.contains(&dyn_bytes), "Expected MAGIC_BYTES in bytes set");
    }

    #[test]
    fn test_literals_collector_size() {
        let literals = process_source_literals(SOURCE);

        // Helper to get count for a type, returns 0 if not present
        let count = |ty: DynSolType| literals.words.get(&ty).map_or(0, |set| set.len());

        assert_eq!(count(DynSolType::Address), 1, "Address literal count mismatch");
        assert_eq!(literals.strings.len(), 3, "String literals count mismatch");
        assert_eq!(literals.bytes.len(), 1, "Byte literals count mismatch");

        // Unsigned integers - MAGIC_NUMBER (1122334455) appears in multiple sizes
        assert_eq!(count(DynSolType::Uint(8)), 2, "Uint(8) count mismatch");
        assert_eq!(count(DynSolType::Uint(16)), 3, "Uint(16) count mismatch");
        assert_eq!(count(DynSolType::Uint(32)), 4, "Uint(32) count mismatch");
        assert_eq!(count(DynSolType::Uint(64)), 5, "Uint(64) count mismatch");
        assert_eq!(count(DynSolType::Uint(128)), 5, "Uint(128) count mismatch");
        assert_eq!(count(DynSolType::Uint(256)), 5, "Uint(256) count mismatch");

        // Signed integers - MAGIC_INT (-777) appears in multiple sizes
        assert_eq!(count(DynSolType::Int(16)), 2, "Int(16) count mismatch");
        assert_eq!(count(DynSolType::Int(32)), 2, "Int(32) count mismatch");
        assert_eq!(count(DynSolType::Int(64)), 2, "Int(64) count mismatch");
        assert_eq!(count(DynSolType::Int(128)), 2, "Int(128) count mismatch");
        assert_eq!(count(DynSolType::Int(256)), 2, "Int(256) count mismatch");

        // FixedBytes(32) includes:
        // - MAGIC_WORD
        // - String literals (hashed and right-padded versions)
        assert_eq!(count(DynSolType::FixedBytes(32)), 6, "FixedBytes(32) count mismatch");

        // Total count check
        assert_eq!(
            literals.words.values().map(|set| set.len()).sum::<usize>(),
            41,
            "Total word values count mismatch"
        );
    }

    // -- TEST HELPERS ---------------------------------------------------------

    fn process_source_literals(source: &str) -> LiteralMaps {
        let mut compiler = Compiler::new(Session::builder().with_stderr_emitter().build());
        compiler
            .enter_mut(|c| -> std::io::Result<()> {
                let mut pcx = c.parse();
                pcx.set_resolve_imports(false);

                pcx.add_file(
                    c.sess().source_map().new_source_file(source_map::FileName::Stdin, source)?,
                );
                pcx.parse();
                let _ = c.lower_asts();
                Ok(())
            })
            .expect("Failed to compile test source");

        LiteralsCollector::process(&Arc::new(compiler), None, usize::MAX)
    }

    fn assert_word(literals: &LiteralMaps, ty: DynSolType, value: B256, msg: &str) {
        assert!(literals.words.get(&ty).is_some_and(|set| set.contains(&value)), "{}", msg);
    }
}
