//! Implementations of [`Utilities`](spec::Group::Utilities) cheatcodes.

use crate::{Cheatcode, Cheatcodes, CheatcodesExecutor, CheatsCtxt, Result, Vm::*};
use alloy_dyn_abi::{DynSolType, DynSolValue, Resolver, TypedData, eip712_parser::EncodeType};
use alloy_ens::namehash;
use alloy_primitives::{B64, Bytes, U256, aliases::B32, keccak256, map::HashMap};
use alloy_sol_types::SolValue;
use foundry_common::{TYPE_BINDING_PREFIX, fs};
use foundry_config::fs_permissions::FsAccessKind;
use foundry_evm_core::constants::DEFAULT_CREATE2_DEPLOYER;
use proptest::prelude::Strategy;
use rand::{Rng, RngCore, seq::SliceRandom};
use revm::context::JournalTr;
use std::path::PathBuf;

/// Contains locations of traces ignored via cheatcodes.
///
/// The way we identify location in traces is by (node_idx, item_idx) tuple where node_idx is an
/// index of a call trace node, and item_idx is a value between 0 and `node.ordering.len()` where i
/// represents point after ith item, and 0 represents the beginning of the node trace.
#[derive(Debug, Default, Clone)]
pub struct IgnoredTraces {
    /// Mapping from (start_node_idx, start_item_idx) to (end_node_idx, end_item_idx) representing
    /// ranges of trace nodes to ignore.
    pub ignored: HashMap<(usize, usize), (usize, usize)>,
    /// Keeps track of (start_node_idx, start_item_idx) of the last `vm.pauseTracing` call.
    pub last_pause_call: Option<(usize, usize)>,
}

impl Cheatcode for labelCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { account, newLabel } = self;
        state.labels.insert(*account, newLabel.clone());
        Ok(Default::default())
    }
}

impl Cheatcode for getLabelCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { account } = self;
        Ok(match state.labels.get(account) {
            Some(label) => label.abi_encode(),
            None => format!("unlabeled:{account}").abi_encode(),
        })
    }
}

impl Cheatcode for computeCreateAddressCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { nonce, deployer } = self;
        ensure!(*nonce <= U256::from(u64::MAX), "nonce must be less than 2^64 - 1");
        Ok(deployer.create(nonce.to()).abi_encode())
    }
}

impl Cheatcode for computeCreate2Address_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { salt, initCodeHash, deployer } = self;
        Ok(deployer.create2(salt, initCodeHash).abi_encode())
    }
}

impl Cheatcode for computeCreate2Address_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { salt, initCodeHash } = self;
        Ok(DEFAULT_CREATE2_DEPLOYER.create2(salt, initCodeHash).abi_encode())
    }
}

impl Cheatcode for ensNamehashCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        Ok(namehash(name).abi_encode())
    }
}

impl Cheatcode for randomUint_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        random_uint(state, None, None)
    }
}

impl Cheatcode for randomUint_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { min, max } = *self;
        random_uint(state, None, Some((min, max)))
    }
}

impl Cheatcode for randomUint_2Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { bits } = *self;
        random_uint(state, Some(bits), None)
    }
}

impl Cheatcode for randomAddressCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        Ok(DynSolValue::type_strategy(&DynSolType::Address)
            .new_tree(state.test_runner())
            .unwrap()
            .current()
            .abi_encode())
    }
}

impl Cheatcode for randomInt_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        random_int(state, None)
    }
}

impl Cheatcode for randomInt_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { bits } = *self;
        random_int(state, Some(bits))
    }
}

impl Cheatcode for randomBoolCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let rand_bool: bool = state.rng().random();
        Ok(rand_bool.abi_encode())
    }
}

impl Cheatcode for randomBytesCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { len } = *self;
        ensure!(
            len <= U256::from(usize::MAX),
            format!("bytes length cannot exceed {}", usize::MAX)
        );
        let mut bytes = vec![0u8; len.to::<usize>()];
        state.rng().fill_bytes(&mut bytes);
        Ok(bytes.abi_encode())
    }
}

impl Cheatcode for randomBytes4Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let rand_u32 = state.rng().next_u32();
        Ok(B32::from(rand_u32).abi_encode())
    }
}

impl Cheatcode for randomBytes8Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let rand_u64 = state.rng().next_u64();
        Ok(B64::from(rand_u64).abi_encode())
    }
}

impl Cheatcode for pauseTracingCall {
    fn apply_full(
        &self,
        ccx: &mut crate::CheatsCtxt,
        executor: &mut dyn CheatcodesExecutor,
    ) -> Result {
        let Some(tracer) = executor.tracing_inspector() else {
            // No tracer -> nothing to pause
            return Ok(Default::default());
        };

        // If paused earlier, ignore the call
        if ccx.state.ignored_traces.last_pause_call.is_some() {
            return Ok(Default::default());
        }

        let cur_node = &tracer.traces().nodes().last().expect("no trace nodes");
        ccx.state.ignored_traces.last_pause_call = Some((cur_node.idx, cur_node.ordering.len()));

        Ok(Default::default())
    }
}

impl Cheatcode for resumeTracingCall {
    fn apply_full(
        &self,
        ccx: &mut crate::CheatsCtxt,
        executor: &mut dyn CheatcodesExecutor,
    ) -> Result {
        let Some(tracer) = executor.tracing_inspector() else {
            // No tracer -> nothing to unpause
            return Ok(Default::default());
        };

        let Some(start) = ccx.state.ignored_traces.last_pause_call.take() else {
            // Nothing to unpause
            return Ok(Default::default());
        };

        let node = &tracer.traces().nodes().last().expect("no trace nodes");
        ccx.state.ignored_traces.ignored.insert(start, (node.idx, node.ordering.len()));

        Ok(Default::default())
    }
}

impl Cheatcode for interceptInitcodeCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        if !state.intercept_next_create_call {
            state.intercept_next_create_call = true;
        } else {
            bail!("vm.interceptInitcode() has already been called")
        }
        Ok(Default::default())
    }
}

impl Cheatcode for setArbitraryStorage_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { target } = self;
        ccx.state.arbitrary_storage().mark_arbitrary(target, false);

        Ok(Default::default())
    }
}

impl Cheatcode for setArbitraryStorage_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { target, overwrite } = self;
        ccx.state.arbitrary_storage().mark_arbitrary(target, *overwrite);

        Ok(Default::default())
    }
}

impl Cheatcode for copyStorageCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { from, to } = self;

        ensure!(
            !ccx.state.has_arbitrary_storage(to),
            "target address cannot have arbitrary storage"
        );

        if let Ok(from_account) = ccx.ecx.journaled_state.load_account(*from) {
            let from_storage = from_account.storage.clone();
            if let Ok(mut to_account) = ccx.ecx.journaled_state.load_account(*to) {
                to_account.storage = from_storage;
                if let Some(arbitrary_storage) = &mut ccx.state.arbitrary_storage {
                    arbitrary_storage.mark_copy(from, to);
                }
            }
        }

        Ok(Default::default())
    }
}

impl Cheatcode for sortCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { array } = self;

        let mut sorted_values = array.clone();
        sorted_values.sort();

        Ok(sorted_values.abi_encode())
    }
}

impl Cheatcode for shuffleCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { array } = self;

        let mut shuffled_values = array.clone();
        let rng = state.rng();
        shuffled_values.shuffle(rng);

        Ok(shuffled_values.abi_encode())
    }
}

impl Cheatcode for setSeedCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { seed } = self;
        ccx.state.set_seed(*seed);
        Ok(Default::default())
    }
}

/// Helper to generate a random `uint` value (with given bits or bounded if specified)
/// from type strategy.
fn random_uint(state: &mut Cheatcodes, bits: Option<U256>, bounds: Option<(U256, U256)>) -> Result {
    if let Some(bits) = bits {
        // Generate random with specified bits.
        ensure!(bits <= U256::from(256), "number of bits cannot exceed 256");
        return Ok(DynSolValue::type_strategy(&DynSolType::Uint(bits.to::<usize>()))
            .new_tree(state.test_runner())
            .unwrap()
            .current()
            .abi_encode());
    }

    if let Some((min, max)) = bounds {
        ensure!(min <= max, "min must be less than or equal to max");
        // Generate random between range min..=max
        let exclusive_modulo = max - min;
        let mut random_number: U256 = state.rng().random();
        if exclusive_modulo != U256::MAX {
            let inclusive_modulo = exclusive_modulo + U256::from(1);
            random_number %= inclusive_modulo;
        }
        random_number += min;
        return Ok(random_number.abi_encode());
    }

    // Generate random `uint256` value.
    Ok(DynSolValue::type_strategy(&DynSolType::Uint(256))
        .new_tree(state.test_runner())
        .unwrap()
        .current()
        .abi_encode())
}

/// Helper to generate a random `int` value (with given bits if specified) from type strategy.
fn random_int(state: &mut Cheatcodes, bits: Option<U256>) -> Result {
    let no_bits = bits.unwrap_or(U256::from(256));
    ensure!(no_bits <= U256::from(256), "number of bits cannot exceed 256");
    Ok(DynSolValue::type_strategy(&DynSolType::Int(no_bits.to::<usize>()))
        .new_tree(state.test_runner())
        .unwrap()
        .current()
        .abi_encode())
}

impl Cheatcode for eip712HashType_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { typeNameOrDefinition } = self;

        let type_def = get_canonical_type_def(typeNameOrDefinition, state, None)?;

        Ok(keccak256(type_def.as_bytes()).to_vec())
    }
}

impl Cheatcode for eip712HashType_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { bindingsPath, typeName } = self;

        let path = state.config.ensure_path_allowed(bindingsPath, FsAccessKind::Read)?;
        let type_def = get_type_def_from_bindings(typeName, path, &state.config.root)?;

        Ok(keccak256(type_def.as_bytes()).to_vec())
    }
}

impl Cheatcode for eip712HashStruct_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { typeNameOrDefinition, abiEncodedData } = self;

        let type_def = get_canonical_type_def(typeNameOrDefinition, state, None)?;
        let primary = &type_def[..type_def.find('(').unwrap_or(type_def.len())];

        get_struct_hash(primary, &type_def, abiEncodedData)
    }
}

impl Cheatcode for eip712HashStruct_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { bindingsPath, typeName, abiEncodedData } = self;

        let path = state.config.ensure_path_allowed(bindingsPath, FsAccessKind::Read)?;
        let type_def = get_type_def_from_bindings(typeName, path, &state.config.root)?;

        get_struct_hash(typeName, &type_def, abiEncodedData)
    }
}

impl Cheatcode for eip712HashTypedDataCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { jsonData } = self;
        let typed_data: TypedData = serde_json::from_str(jsonData)?;
        let digest = typed_data.eip712_signing_hash()?;

        Ok(digest.to_vec())
    }
}

/// Returns EIP-712 canonical type definition from the provided string type representation or type
/// name. If type name provided, then it looks up bindings from file generated by `forge bind-json`.
fn get_canonical_type_def(
    name_or_def: &String,
    state: &mut Cheatcodes,
    path: Option<PathBuf>,
) -> Result<String> {
    let type_def = if name_or_def.contains('(') {
        // If the input contains '(', it must be the type definition.
        EncodeType::parse(name_or_def).and_then(|parsed| parsed.canonicalize())?
    } else {
        // Otherwise, it must be the type name.
        let path = path.as_ref().unwrap_or(&state.config.bind_json_path);
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        get_type_def_from_bindings(name_or_def, path, &state.config.root)?
    };

    Ok(type_def)
}

/// Returns the EIP-712 type definition from the bindings in the provided path.
/// Assumes that read validation for the path has already been checked.
fn get_type_def_from_bindings(name: &String, path: PathBuf, root: &PathBuf) -> Result<String> {
    let content = fs::read_to_string(&path)?;

    let type_defs: HashMap<&str, &str> = content
        .lines()
        .filter_map(|line| {
            let relevant = line.trim().strip_prefix(TYPE_BINDING_PREFIX)?;
            let (name, def) = relevant.split_once('=')?;
            Some((name.trim(), def.trim().strip_prefix('"')?.strip_suffix("\";")?))
        })
        .collect();

    match type_defs.get(name.as_str()) {
        Some(value) => Ok(value.to_string()),
        None => {
            let bindings =
                type_defs.keys().map(|k| format!(" - {k}")).collect::<Vec<String>>().join("\n");

            bail!(
                "'{}' not found in '{}'.{}",
                name,
                path.strip_prefix(root).unwrap_or(&path).to_string_lossy(),
                if bindings.is_empty() {
                    String::new()
                } else {
                    format!("\nAvailable bindings:\n{bindings}\n")
                }
            );
        }
    }
}

/// Returns the EIP-712 struct hash for provided name, definition and ABI encoded data.
fn get_struct_hash(primary: &str, type_def: &String, abi_encoded_data: &Bytes) -> Result {
    let mut resolver = Resolver::default();

    // Populate the resolver by ingesting the canonical type definition, and then get the
    // corresponding `DynSolType` of the primary type.
    resolver
        .ingest_string(type_def)
        .map_err(|e| fmt_err!("Resolver failed to ingest type definition: {e}"))?;

    let resolved_sol_type = resolver
        .resolve(primary)
        .map_err(|e| fmt_err!("Failed to resolve EIP-712 primary type '{primary}': {e}"))?;

    // ABI-decode the bytes into `DynSolValue::CustomStruct`.
    let sol_value = resolved_sol_type.abi_decode(abi_encoded_data.as_ref()).map_err(|e| {
        fmt_err!("Failed to ABI decode using resolved_sol_type directly for '{primary}': {e}.")
    })?;

    // Use the resolver to properly encode the data.
    let encoded_data: Vec<u8> = resolver
        .encode_data(&sol_value)
        .map_err(|e| fmt_err!("Failed to EIP-712 encode data for struct '{primary}': {e}"))?
        .ok_or_else(|| fmt_err!("EIP-712 data encoding returned 'None' for struct '{primary}'"))?;

    // Compute the type hash of the primary type.
    let type_hash = resolver
        .type_hash(primary)
        .map_err(|e| fmt_err!("Failed to compute typeHash for EIP712 type '{primary}': {e}"))?;

    // Compute the struct hash of the concatenated type hash and encoded data.
    let mut bytes_to_hash = Vec::with_capacity(32 + encoded_data.len());
    bytes_to_hash.extend_from_slice(type_hash.as_slice());
    bytes_to_hash.extend_from_slice(&encoded_data);

    Ok(keccak256(&bytes_to_hash).to_vec())
}
