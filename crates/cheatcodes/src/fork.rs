//! Implementations of [`Forking`](spec::Group::Forking) cheatcodes.

use crate::{
    Cheatcode, CheatsCtxt, DatabaseExt, Result, Vm::*, json::parse_json_as,
    toml::toml_to_json_value,
};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_sol_types::SolValue;
use eyre::OptionExt;
use foundry_config::{Config, fork_config::ForkConfigPermission};
use foundry_evm_core::ContextExt;
use std::{
    fs,
    path::{Path, PathBuf},
};

// -- CHECK FORK VARIABLES -----------------------------------------------------

// Check if fork variables exist
impl Cheatcode for checkForkVarCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let chain = get_active_fork_chain_id(ccx)?;
        check_var_exists(chain, &self.key, ccx.state)
    }
}

impl Cheatcode for checkForkChainVarCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        check_var_exists(self.chain.to::<u64>(), &self.key, state)
    }
}

/// Helper to check if a variable exists in the TOML config.
fn check_var_exists(chain: u64, key: &str, state: &crate::Cheatcodes) -> Result {
    let name = get_chain_name(chain)?;
    let forks = state.config.forks.read().map_err(|_| fmt_err!("Failed to acquire read lock"))?;
    let exists = forks.get(name).and_then(|config| config.vars.get(key)).is_some();
    Ok(exists.abi_encode())
}

// -- READ FORK VARIABLES ------------------------------------------------------

impl Cheatcode for readForkChainIdsCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let forks =
            state.config.forks.read().map_err(|_| fmt_err!("Failed to acquire read lock"))?;
        Ok(forks
            .keys()
            .map(|name| alloy_chains::Chain::from_named(name.parse().unwrap()).id())
            .collect::<Vec<_>>()
            .abi_encode())
    }
}

impl Cheatcode for readForkChainsCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let forks =
            state.config.forks.read().map_err(|_| fmt_err!("Failed to acquire read lock"))?;
        Ok(forks.keys().collect::<Vec<_>>().abi_encode())
    }
}

impl Cheatcode for readForkChainCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        Ok(get_active_fork_chain_name(ccx)?.abi_encode())
    }
}

impl Cheatcode for readForkChainIdCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        Ok(get_active_fork_chain_id(ccx)?.abi_encode())
    }
}

fn resolve_rpc_url(name: &'static str, state: &mut crate::Cheatcodes) -> Result {
    // Get the chain ID from the chain_configs
    let forks = state.config.forks.read().map_err(|_| fmt_err!("Failed to acquire read lock"))?;
    if let Some(config) = forks.get(name) {
        let rpc = match config.rpc_endpoint {
            Some(ref url) => url.clone().resolve(),
            None => state.config.rpc_endpoint(name)?,
        };

        return Ok(rpc.url()?.abi_encode());
    }

    bail!("[fork.{name}] subsection not found in [fork] of 'foundry.toml'")
}

impl Cheatcode for readForkChainRpcUrlCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { id } = self;
        let name = get_chain_name(id.to::<u64>())?;
        resolve_rpc_url(name, state)
    }
}

impl Cheatcode for readForkRpcUrlCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let name = get_active_fork_chain_name(ccx)?;
        resolve_rpc_url(name, ccx.state)
    }
}

/// Gets the alloy chain name for a given chain id.
fn get_chain_name(id: u64) -> Result<&'static str> {
    let chain = alloy_chains::Chain::from_id(id)
        .named()
        .ok_or_eyre("unknown name for active forked chain")?;

    Ok(chain.as_str())
}

// Helper macros to generate cheatcode implementations
macro_rules! impl_get_value_cheatcode {
    ($struct:ident, $sol_type:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let Self { key } = self;
                let chain = get_active_fork_chain_id(ccx)?;
                get_and_encode_toml_value(chain, key, $sol_type, false, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                let Self { chain, key } = self;
                get_and_encode_toml_value(chain.to::<u64>(), key, $sol_type, false, state)
            }
        }
    };
}

macro_rules! impl_get_array_cheatcode {
    ($struct:ident, $sol_type:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let Self { key } = self;
                let chain = get_active_fork_chain_id(ccx)?;
                get_and_encode_toml_value(chain, key, $sol_type, true, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                let Self { chain, key } = self;
                get_and_encode_toml_value(chain.to::<u64>(), key, $sol_type, true, state)
            }
        }
    };
}

// Bool
impl_get_value_cheatcode!(readForkChainBoolCall, &DynSolType::Bool);
impl_get_value_cheatcode!(readForkBoolCall, &DynSolType::Bool, stateful);
impl_get_array_cheatcode!(readForkChainBoolArrayCall, &DynSolType::Bool);
impl_get_array_cheatcode!(readForkBoolArrayCall, &DynSolType::Bool, stateful);

// Int
impl_get_value_cheatcode!(readForkChainIntCall, &DynSolType::Int(256));
impl_get_value_cheatcode!(readForkIntCall, &DynSolType::Int(256), stateful);
impl_get_array_cheatcode!(readForkChainIntArrayCall, &DynSolType::Int(256));
impl_get_array_cheatcode!(readForkIntArrayCall, &DynSolType::Int(256), stateful);

// Uint
impl_get_value_cheatcode!(readForkChainUintCall, &DynSolType::Uint(256));
impl_get_value_cheatcode!(readForkUintCall, &DynSolType::Uint(256), stateful);
impl_get_array_cheatcode!(readForkChainUintArrayCall, &DynSolType::Uint(256));
impl_get_array_cheatcode!(readForkUintArrayCall, &DynSolType::Uint(256), stateful);

// Address
impl_get_value_cheatcode!(readForkChainAddressCall, &DynSolType::Address);
impl_get_value_cheatcode!(readForkAddressCall, &DynSolType::Address, stateful);
impl_get_array_cheatcode!(readForkChainAddressArrayCall, &DynSolType::Address);
impl_get_array_cheatcode!(readForkAddressArrayCall, &DynSolType::Address, stateful);

// Bytes32
impl_get_value_cheatcode!(readForkChainBytes32Call, &DynSolType::FixedBytes(32));
impl_get_value_cheatcode!(readForkBytes32Call, &DynSolType::FixedBytes(32), stateful);
impl_get_array_cheatcode!(readForkChainBytes32ArrayCall, &DynSolType::FixedBytes(32));
impl_get_array_cheatcode!(readForkBytes32ArrayCall, &DynSolType::FixedBytes(32), stateful);

// Bytes
impl_get_value_cheatcode!(readForkChainBytesCall, &DynSolType::Bytes);
impl_get_value_cheatcode!(readForkBytesCall, &DynSolType::Bytes, stateful);
impl_get_array_cheatcode!(readForkChainBytesArrayCall, &DynSolType::Bytes);
impl_get_array_cheatcode!(readForkBytesArrayCall, &DynSolType::Bytes, stateful);

// String
impl_get_value_cheatcode!(readForkChainStringCall, &DynSolType::String);
impl_get_value_cheatcode!(readForkStringCall, &DynSolType::String, stateful);
impl_get_array_cheatcode!(readForkChainStringArrayCall, &DynSolType::String);
impl_get_array_cheatcode!(readForkStringArrayCall, &DynSolType::String, stateful);

// -- WRITE FORK VARIABLES -----------------------------------------------------

// Helper macros to generate write cheatcode implementations
macro_rules! impl_write_value_cheatcode {
    ($struct:ident, $sol_type:expr, $toml_converter:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let Self { key, value } = self;
                let chain = get_active_fork_chain_id(ccx)?;
                let toml_value = $toml_converter((*value).clone());
                write_toml_value(chain, key, toml_value, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr, $toml_converter:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                let Self { chain, key, value } = self;
                let toml_value = $toml_converter((*value).clone());
                write_toml_value((*chain).to::<u64>(), key, toml_value, state)
            }
        }
    };
}

macro_rules! impl_write_array_cheatcode {
    ($struct:ident, $sol_type:expr, $toml_converter:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let Self { key, value } = self;
                let chain = get_active_fork_chain_id(ccx)?;
                let toml_array = value.iter().map(|v| $toml_converter((*v).to_owned())).collect();
                write_toml_value(chain, key, toml::Value::Array(toml_array), ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr, $toml_converter:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                let Self { chain, key, value } = self;
                let toml_array = value.iter().map(|v| $toml_converter((*v).to_owned())).collect();
                write_toml_value((*chain).to::<u64>(), key, toml::Value::Array(toml_array), state)
            }
        }
    };
}

// Bool
impl_write_value_cheatcode!(
    writeForkVar_0Call,
    &DynSolType::Bool,
    |v: bool| toml::Value::Boolean(v),
    stateful
);
impl_write_value_cheatcode!(writeForkChainVar_0Call, &DynSolType::Bool, |v: bool| {
    toml::Value::Boolean(v)
});
impl_write_array_cheatcode!(
    writeForkVar_1Call,
    &DynSolType::Bool,
    |v: bool| toml::Value::Boolean(v),
    stateful
);
impl_write_array_cheatcode!(writeForkChainVar_1Call, &DynSolType::Bool, |v: bool| {
    toml::Value::Boolean(v)
});

// Int
impl_write_value_cheatcode!(
    writeForkVar_2Call,
    &DynSolType::Int(256),
    |v: alloy_primitives::I256| v
        .try_into()
        .map(toml::Value::Integer)
        .unwrap_or_else(|_| toml::Value::String(v.to_string())),
    stateful
);
impl_write_value_cheatcode!(
    writeForkChainVar_2Call,
    &DynSolType::Int(256),
    |v: alloy_primitives::I256| v
        .try_into()
        .map(toml::Value::Integer)
        .unwrap_or_else(|_| toml::Value::String(v.to_string()))
);
impl_write_array_cheatcode!(
    writeForkVar_3Call,
    &DynSolType::Int(256),
    |v: alloy_primitives::I256| v
        .try_into()
        .map(toml::Value::Integer)
        .unwrap_or_else(|_| toml::Value::String(v.to_string())),
    stateful
);
impl_write_array_cheatcode!(
    writeForkChainVar_3Call,
    &DynSolType::Int(256),
    |v: alloy_primitives::I256| v
        .try_into()
        .map(toml::Value::Integer)
        .unwrap_or_else(|_| toml::Value::String(v.to_string()))
);

// Uint
impl_write_value_cheatcode!(
    writeForkVar_4Call,
    &DynSolType::Uint(256),
    |v: alloy_primitives::U256| v
        .try_into()
        .map(toml::Value::Integer)
        .unwrap_or_else(|_| toml::Value::String(v.to_string())),
    stateful
);
impl_write_value_cheatcode!(
    writeForkChainVar_4Call,
    &DynSolType::Uint(256),
    |v: alloy_primitives::U256| v
        .try_into()
        .map(toml::Value::Integer)
        .unwrap_or_else(|_| toml::Value::String(v.to_string()))
);
impl_write_array_cheatcode!(
    writeForkVar_5Call,
    &DynSolType::Uint(256),
    |v: alloy_primitives::U256| v
        .try_into()
        .map(toml::Value::Integer)
        .unwrap_or_else(|_| toml::Value::String(v.to_string())),
    stateful
);
impl_write_array_cheatcode!(
    writeForkChainVar_5Call,
    &DynSolType::Uint(256),
    |v: alloy_primitives::U256| v
        .try_into()
        .map(toml::Value::Integer)
        .unwrap_or_else(|_| toml::Value::String(v.to_string()))
);

// Address
impl_write_value_cheatcode!(
    writeForkVar_6Call,
    &DynSolType::Address,
    |v: alloy_primitives::Address| toml::Value::String(v.to_string()),
    stateful
);
impl_write_value_cheatcode!(
    writeForkChainVar_6Call,
    &DynSolType::Address,
    |v: alloy_primitives::Address| toml::Value::String(v.to_string())
);
impl_write_array_cheatcode!(
    writeForkVar_7Call,
    &DynSolType::Address,
    |v: alloy_primitives::Address| toml::Value::String(v.to_string()),
    stateful
);
impl_write_array_cheatcode!(
    writeForkChainVar_7Call,
    &DynSolType::Address,
    |v: alloy_primitives::Address| toml::Value::String(v.to_string())
);

// Bytes32
impl_write_value_cheatcode!(
    writeForkVar_8Call,
    &DynSolType::FixedBytes(32),
    |v: alloy_primitives::FixedBytes<32>| toml::Value::String(v.to_string()),
    stateful
);
impl_write_value_cheatcode!(
    writeForkChainVar_8Call,
    &DynSolType::FixedBytes(32),
    |v: alloy_primitives::FixedBytes<32>| toml::Value::String(v.to_string())
);
impl_write_array_cheatcode!(
    writeForkVar_9Call,
    &DynSolType::FixedBytes(32),
    |v: alloy_primitives::FixedBytes<32>| toml::Value::String(v.to_string()),
    stateful
);
impl_write_array_cheatcode!(
    writeForkChainVar_9Call,
    &DynSolType::FixedBytes(32),
    |v: alloy_primitives::FixedBytes<32>| toml::Value::String(v.to_string())
);

// String
impl_write_value_cheatcode!(
    writeForkVar_10Call,
    &DynSolType::String,
    |v: String| toml::Value::String(v),
    stateful
);
impl_write_value_cheatcode!(writeForkChainVar_10Call, &DynSolType::String, |v: String| {
    toml::Value::String(v)
});
impl_write_array_cheatcode!(
    writeForkVar_11Call,
    &DynSolType::String,
    |v: String| toml::Value::String(v),
    stateful
);
impl_write_array_cheatcode!(writeForkChainVar_11Call, &DynSolType::String, |v: String| {
    toml::Value::String(v)
});

// Bytes
impl_write_value_cheatcode!(
    writeForkVar_12Call,
    &DynSolType::Bytes,
    |v: alloy_primitives::Bytes| toml::Value::String(v.to_string()),
    stateful
);
impl_write_value_cheatcode!(
    writeForkChainVar_12Call,
    &DynSolType::Bytes,
    |v: alloy_primitives::Bytes| toml::Value::String(v.to_string())
);
impl_write_array_cheatcode!(
    writeForkVar_13Call,
    &DynSolType::Bytes,
    |v: alloy_primitives::Bytes| toml::Value::String(v.to_string()),
    stateful
);
impl_write_array_cheatcode!(
    writeForkChainVar_13Call,
    &DynSolType::Bytes,
    |v: alloy_primitives::Bytes| toml::Value::String(v.to_string())
);

// -- HELPER METHODS -----------------------------------------------------

/// Generic helper to get a value from TOML config and encode it as a Solidity type.
///
/// # Arguments
/// * `chain`: The chain ID to look up the configuration for
/// * `key`: The variable key to look up in the fork configuration
/// * `ty`: The Solidity type to parse the TOML value into (for array operations, this is the
///   element type)
/// * `is_array`:
///     - If `true`, expects a `toml::Value::Array` and returns an encoded array of the input type.
///     - If `false`, expects a single value and returns it encoded as the specified type.
/// * `state`: The cheatcodes state containing the fork configurations
///
/// # Returns
/// Returns the ABI-encoded value(s) from the TOML configuration, parsed according to the specified
/// Solidity type.
fn get_and_encode_toml_value(
    chain: u64,
    key: &str,
    ty: &DynSolType,
    is_array: bool,
    state: &crate::Cheatcodes,
) -> Result {
    let name = get_chain_name(chain)?;
    let forks = state.config.forks.read().map_err(|_| fmt_err!("Failed to acquire read lock"))?;
    let config = forks.get(name).ok_or_else(|| {
        fmt_err!("[fork.{name}] subsection not found in [fork] of 'foundry.toml'")
    })?;
    let value = config
        .vars
        .get(key)
        .ok_or_else(|| fmt_err!("Variable '{key}' not found in [fork.{name}] configuration"))?;

    if is_array {
        let arr = value
            .as_array()
            .ok_or_else(|| fmt_err!("Variable '{key}' in [fork.{name}] must be an array"))?;

        let result: Result<Vec<_>> = arr
            .iter()
            .enumerate()
            .map(|(i, elem)| {
                let context = format!("{key}[{i}]");
                parse_toml_element(elem, ty, &context, name)
            })
            .collect();

        Ok(DynSolValue::Array(result?).abi_encode())
    } else {
        let sol_value = parse_toml_element(value, ty, key, name)?;
        Ok(sol_value.abi_encode())
    }
}

/// Parses a single TOML value into a specific Solidity type.
fn parse_toml_element(
    elem: &toml::Value,
    element_ty: &DynSolType,
    key: &str,
    fork_name: &str,
) -> Result<DynSolValue> {
    // Convert TOML value to JSON value and use existing JSON parsing logic
    parse_json_as(&toml_to_json_value(elem.to_owned()), element_ty)
        .map_err(|e| fmt_err!("Failed to parse '{key}' in [fork.{fork_name}]: {e}"))
}

/// A resolver to determine the correct configuration file to modify for a given fork chain.
struct ForkConfigResolver<'a> {
    root: &'a Path,
    chain: &'a str,
}

impl<'a> ForkConfigResolver<'a> {
    /// Creates a new resolver for a specific fork chain.
    fn new(root: &'a Path, chain_name: &'a str) -> Self {
        Self { root, chain: chain_name }
    }

    /// Determines the correct config file and returns its path and parsed content.
    ///
    /// The logic is as follows:
    /// 1. Check if the section `[forks.<chain_name>]` exists in `foundry.toml`. If so, it is the
    ///    target.
    /// 2. If not, check if `foundry.toml` `extends` a base file and if that base file contains the
    ///    section. If so, the base file is the target.
    /// 3. If the section exists in neither, `foundry.toml` is the default target for creation.
    ///
    /// Returns `Ok(None)` if the local `foundry.toml` doesn't exist.
    fn resolve_and_load(&self) -> eyre::Result<Option<(PathBuf, toml_edit::DocumentMut)>> {
        let local_path = self.root.join(Config::FILE_NAME);
        if !local_path.exists() {
            return Ok(None);
        }

        let local_content = fs::read_to_string(&local_path)?;
        let mut local_doc: toml_edit::DocumentMut = local_content.parse()?;

        // 1. Local file has precedence. If the section exists here, this is our target.
        if get_toml_section(&local_doc.into_item(), self.chain).is_some() {
            return Ok(Some((local_path, local_doc)));
        }

        // 2. If not local, check the base file specified by `extends`.
        let extends_path_str = local_doc
            .get("profile")
            .and_then(|p| p.as_table())
            .and_then(|profiles| profiles.values().find_map(|p| p.get("extends")?.as_str()));

        if let Some(extends_path_str) = extends_path_str {
            if let Some(parent) = local_path.parent() {
                let base_path =
                    foundry_compilers::utils::canonicalize(parent.join(extends_path_str))?;
                if base_path.exists() {
                    let base_content = fs::read_to_string(&base_path)?;
                    let base_doc: toml_edit::DocumentMut = base_content.parse()?;
                    // If the section exists in the base, that's our target.
                    if get_toml_section(&base_doc.into_item(), self.chain).is_some() {
                        return Ok(Some((base_path, base_doc)));
                    }
                }
            }
        }

        // 3. Default to local file if the section is not found in either place.
        Ok(Some((local_path, local_doc)))
    }
}

/// Generic helper to write value(s) to the in-memory config and disk.
///
/// # Arguments
/// * `chain`: The chain ID to write the configuration for
/// * `key`: The variable key to write in the fork configuration
/// * `value`: The TOML value to write (already converted)
/// * `state`: The cheatcodes state containing the fork configurations
///
/// # Returns
/// Returns ABI-encoded tuple of (success: bool, overwrote: bool)
fn write_toml_value(
    chain: u64,
    key: &str,
    value: toml::Value,
    state: &mut crate::Cheatcodes,
) -> Result {
    if matches!(key, "access" | "rpc_endpoint") {
        bail!("'{key}' cannot be modified with cheatcodes");
    }

    let overwrote = false;
    let name = get_chain_name(chain)?;

    // Update in-memory config first to calculate `overwrote` for the return value.
    {
        state
            .config
            .forks
            .write()
            .map_err(|_| fmt_err!("Failed to acquire write lock"))?
            .chain_configs
            .entry(name.to_string())
            .and_modify(|v| {
                overwrote = true;
                v = value;
            })
            .or_insert(value);
    }

    // The `overwrote` flag is NOT passed down, if the disk update is unsuccessful.
    let (success, overworte) = match persist_fork_config_to_file(chain, key, &value, state) {
        Err(e) => {
            eprintln!("Warning: Failed to persist fork config to disk: {e}");
            (false, false)
        }
        Ok(()) => (true, overwrote),
    };

    Ok((success, overwrote).abi_encode())
}

/// Orchestrates persisting a fork variable by finding the correct config file.
fn persist_fork_config_to_file(
    chain: u64,
    key: &str,
    value: &toml::Value,
    state: &crate::Cheatcodes,
) -> Result<()> {
    if !state
        .config
        .forks
        .read()
        .map_err(|_| eyre::eyre!("Failed to acquire read lock for fork configs"))?
        .access
        .can_write()
    {
        return Ok(());
    }

    let chain_name = get_chain_name(chain)?;

    // Use the resolver to determine the correct file to write to.
    let resolver = ForkConfigResolver::new(&state.config.root, chain_name);
    if let Some((target_path, mut doc)) = resolver.resolve_and_load()? {
        // Modify the document.
        let forks_table = doc
            .entry("forks")
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| eyre::eyre!("Invalid TOML: root 'forks' entry must be a table"))?;

        let chain_table =
            forks_table.entry(chain_name).or_insert(toml_edit::table()).as_table_mut().ok_or_else(
                || eyre::eyre!("Invalid TOML: '[forks.{chain_name}]' must be a table"),
            )?;

        let vars_table =
            chain_table.entry("vars").or_insert(toml_edit::table()).as_table_mut().ok_or_else(
                || eyre::eyre!("Invalid TOML: '[forks.{chain_name}.vars]' must be a table"),
            )?;

        // Insert the value and write back to the file.
        vars_table[key] = toml_edit::value(value);
        fs::write(&target_path, doc.to_string())?;
    }

    Ok(())
}

/// Helper to safely traverse a TOML document to find a specific fork chain's section.
fn get_toml_section<'a>(doc: &'a toml_edit::Item, chain_name: &str) -> Option<&'a toml_edit::Item> {
    doc.get("forks")?.get(chain_name)
}

/// Gets the chain id of the active fork. Panics if no fork is selected.
fn get_active_fork_chain_id(ccx: &mut CheatsCtxt) -> Result<u64> {
    let (db, _, env) = ccx.as_db_env_and_journal();
    if !db.is_forked_mode() {
        bail!("a fork must be selected");
    }
    Ok(env.cfg.chain_id)
}

/// Gets the alloy chain name for the active fork. Panics if no fork is selected.
fn get_active_fork_chain_name(ccx: &mut CheatsCtxt) -> Result<&'static str> {
    get_chain_name(get_active_fork_chain_id(ccx)?)
}
