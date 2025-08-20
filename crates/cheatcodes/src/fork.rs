//! Implementations of [`Forking`](spec::Group::Forking) cheatcodes.

use crate::{
    Cheatcode, CheatsCtxt, DatabaseExt, Result, Vm::*, json::parse_json_as,
    toml::toml_to_json_value,
};
use alloy_chains::Chain;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_sol_types::SolValue;
use foundry_config::Config;
use foundry_evm_core::ContextExt;
use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};

// -- CHECK FORK VARIABLES -----------------------------------------------------

// Check if fork variables exist
impl Cheatcode for checkForkVarCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let chain = get_active_fork_chain(ccx)?;
        check_var_exists(chain, &self.key, ccx.state)
    }
}

impl Cheatcode for checkForkChainVarCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        check_var_exists(Chain::from_id(self.chain.to::<u64>()), &self.key, state)
    }
}

/// Helper to check if a variable exists in the TOML config.
fn check_var_exists(chain: Chain, key: &str, state: &crate::Cheatcodes) -> Result {
    let forks = state.config.forks.read().map_err(|_| fmt_err!("Failed to acquire read lock"))?;
    let exists = forks.get(&chain).and_then(|config| config.vars.get(key)).is_some();
    Ok(exists.abi_encode())
}

// -- READ FORK VARIABLES ------------------------------------------------------

impl Cheatcode for readForkChainIdsCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self {} = self;
        let forks =
            state.config.forks.read().map_err(|_| fmt_err!("Failed to acquire read lock"))?;
        Ok(forks.keys().map(|chain| chain.id()).collect::<Vec<_>>().abi_encode())
    }
}

impl Cheatcode for readForkChainsCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self {} = self;
        let forks =
            state.config.forks.read().map_err(|_| fmt_err!("Failed to acquire read lock"))?;
        Ok(forks
            .keys()
            .map(|chain| get_chain_name(chain).to_string())
            .collect::<Vec<_>>()
            .abi_encode())
    }
}

impl Cheatcode for readForkChainCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        Ok(get_chain_name(&get_active_fork_chain(ccx)?).to_string().abi_encode())
    }
}

impl Cheatcode for readForkChainIdCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        Ok(get_active_fork_chain_id(ccx)?.abi_encode())
    }
}

fn resolve_rpc_url(chain: Chain, state: &mut crate::Cheatcodes) -> Result {
    let forks = state.config.forks.read().map_err(|_| fmt_err!("Failed to acquire read lock"))?;
    if let Some(config) = forks.get(&chain) {
        let rpc = match config.rpc_endpoint {
            Some(ref url) => url.clone().resolve(),
            None => state.config.rpc_endpoint(&get_chain_name(&chain))?,
        };

        return Ok(rpc.url()?.abi_encode());
    }

    bail!(
        "'rpc_endpoint' not found in '[fork.<chain_id: {chain}>]' subsection of 'foundry.toml'",
        chain = chain.id()
    )
}

impl Cheatcode for readForkChainRpcUrlCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { id } = self;
        resolve_rpc_url(Chain::from_id(id.to::<u64>()), state)
    }
}

impl Cheatcode for readForkRpcUrlCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        resolve_rpc_url(get_active_fork_chain(ccx)?, ccx.state)
    }
}

/// Gets the alloy chain name for a given chain id.
fn get_chain_name(chain: &Chain) -> Cow<'static, str> {
    chain.named().map_or(Cow::Owned(chain.id().to_string()), |name| Cow::Borrowed(name.as_str()))
}

// Helper macros to generate cheatcode implementations
macro_rules! impl_get_value_cheatcode {
    ($struct:ident, $sol_type:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let Self { key } = self;
                let chain = get_active_fork_chain(ccx)?;
                get_and_encode_toml_value(chain, key, $sol_type, false, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                let Self { chain, key } = self;
                get_and_encode_toml_value(
                    Chain::from_id(chain.to::<u64>()),
                    key,
                    $sol_type,
                    false,
                    state,
                )
            }
        }
    };
}

macro_rules! impl_get_array_cheatcode {
    ($struct:ident, $sol_type:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let Self { key } = self;
                let chain = get_active_fork_chain(ccx)?;
                get_and_encode_toml_value(chain, key, $sol_type, true, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                let Self { chain, key } = self;
                get_and_encode_toml_value(
                    Chain::from_id(chain.to::<u64>()),
                    key,
                    $sol_type,
                    true,
                    state,
                )
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
                let chain = get_active_fork_chain(ccx)?;
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
                write_toml_value(Chain::from_id((*chain).to::<u64>()), key, toml_value, state)
            }
        }
    };
}

macro_rules! impl_write_array_cheatcode {
    ($struct:ident, $sol_type:expr, $toml_converter:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let Self { key, value } = self;
                let chain = get_active_fork_chain(ccx)?;
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
                write_toml_value(
                    Chain::from_id((*chain).to::<u64>()),
                    key,
                    toml::Value::Array(toml_array),
                    state,
                )
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
    chain: Chain,
    key: &str,
    ty: &DynSolType,
    is_array: bool,
    state: &crate::Cheatcodes,
) -> Result {
    let forks = state.config.forks.read().map_err(|_| fmt_err!("Failed to acquire read lock"))?;
    let config = forks.get(&chain).ok_or_else(|| {
        fmt_err!(
            "'[fork.<chain_id: {chain}>]' subsection not found in 'foundry.toml'",
            chain = chain.id()
        )
    })?;
    let value = config.vars.get(key).ok_or_else(|| {
        fmt_err!("variable '{key}' not found in '[fork.<chain_id: {chain}>]'", chain = chain.id())
    })?;

    if is_array {
        let arr = value.as_array().ok_or_else(|| {
            fmt_err!(
                "variable '{key}' in '[fork.<chain_id: {id}>]' must be an array",
                id = chain.id()
            )
        })?;

        let result: Result<Vec<_>> = arr
            .iter()
            .enumerate()
            .map(|(i, elem)| {
                let context = format!("{key}[{i}]");
                parse_toml_element(elem, ty, &context, chain)
            })
            .collect();

        Ok(DynSolValue::Array(result?).abi_encode())
    } else {
        let sol_value = parse_toml_element(value, ty, key, chain)?;
        Ok(sol_value.abi_encode())
    }
}

/// Parses a single TOML value into a specific Solidity type.
fn parse_toml_element(
    elem: &toml::Value,
    element_ty: &DynSolType,
    key: &str,
    chain: Chain,
) -> Result<DynSolValue> {
    // Convert TOML value to JSON value and use existing JSON parsing logic
    parse_json_as(&toml_to_json_value(elem.to_owned()), element_ty).map_err(|e| {
        fmt_err!("failed to parse '{key}' in '[fork.<chain_id: {chain}>]': {e}", chain = chain.id())
    })
}

/// A writer struct that, for a given fork chain, determines the correct toml file to modify.
struct ForkConfigWriter<'a> {
    root: &'a Path,
    chain: Cow<'static, str>,
    key: &'a str,
    value: &'a toml::Value,
}

impl<'a> ForkConfigWriter<'a> {
    fn new(
        root: &'a Path,
        chain_name: Cow<'static, str>,
        key: &'a str,
        value: &'a toml::Value,
    ) -> Self {
        Self { root, chain: chain_name, key, value }
    }

    /// Persists the variable to the appropriate config file.
    ///
    /// The logic is as follows:
    /// 1. Check if `[forks.<chain_name>]` exists in `foundry.toml`. If so, write there.
    /// 2. If not, check if `foundry.toml` `extends` a base file and if that file contains the
    ///    section. If so, write to the base file.
    /// 3. If the section exists in neither, write to the local `foundry.toml`.
    ///
    /// Returns `Result<bool>` where the boolean indicates if a value was overwritten.
    /// Returns an error if `foundry.toml` cannot be found or written to.
    fn persist(&self) -> Result<bool> {
        let local_path = self.root.join(Config::FILE_NAME);
        if !local_path.exists() {
            bail!("failed to find a 'foundry.toml' file to write to in root: {:?}", self.root);
        }

        let (target_path, mut doc) = self.resolve_target_doc(&local_path)?;

        let vars_table = self.get_or_create_vars_table(&mut doc)?;

        let previous_value =
            vars_table.insert(self.key, to_toml_edit_value(self.value.clone()).into());
        let overwritten = previous_value.is_some();

        fs::write(&target_path, doc.to_string())?;

        Ok(overwritten)
    }

    /// Resolves the correct TOML document and its path.
    fn resolve_target_doc(
        &self,
        local_path: &Path,
    ) -> eyre::Result<(PathBuf, toml_edit::DocumentMut)> {
        let local_content = fs::read_to_string(local_path)?;
        let local_doc: toml_edit::DocumentMut = local_content.parse()?;

        // 1. Local file has precedence.
        if get_toml_section(&local_doc.as_item(), &self.chain).is_some() {
            return Ok((local_path.to_path_buf(), local_doc));
        }

        // 2. Check the base file specified by `extends`.
        if let Some(base_path_str) = local_doc
            .get("profile")
            .and_then(|p| p.as_table())
            .and_then(|p| p.iter().find_map(|(_, p)| p.get("extends")?.as_str()))
        {
            if let Some(parent) = local_path.parent() {
                let base_path = foundry_compilers::utils::canonicalize(parent.join(base_path_str))?;
                if base_path.exists() {
                    let base_content = fs::read_to_string(&base_path)?;
                    let base_doc: toml_edit::DocumentMut = base_content.parse()?;
                    if get_toml_section(&base_doc.as_item(), &self.chain).is_some() {
                        return Ok((base_path, base_doc));
                    }
                }
            }
        }

        // 3. Default to local file.
        Ok((local_path.to_path_buf(), local_doc))
    }

    /// Navigates to `[forks.<chain>.vars]`, creating tables if needed.
    fn get_or_create_vars_table<'doc>(
        &self,
        doc: &'doc mut toml_edit::DocumentMut,
    ) -> Result<&'doc mut toml_edit::Table> {
        let forks_table = doc["forks"]
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| eyre::eyre!("'forks' must be a table."))?;
        let chain_table = forks_table
            .entry(&self.chain)
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| fmt_err!("'[forks.{}]' must be a table.", self.chain))?;
        chain_table
            .entry("vars")
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| fmt_err!("'[forks.{}.vars]' must be a table.", self.chain))
    }
}

/// Converts a `toml::Value` to a `toml_edit::Value`.
///
/// This is necessary because the in-memory representation uses `toml::Value` for
/// convenience, but the file persistence requires `toml_edit::Value` to avoid
/// clobbering formatting.
fn to_toml_edit_value(value: toml::Value) -> toml_edit::Value {
    match value {
        toml::Value::String(s) => toml_edit::Value::from(s),
        toml::Value::Integer(i) => toml_edit::Value::from(i),
        toml::Value::Float(f) => toml_edit::Value::from(f),
        toml::Value::Boolean(b) => toml_edit::Value::from(b),
        toml::Value::Datetime(d) => toml_edit::Value::from(d),
        toml::Value::Array(arr) => {
            let values = arr.into_iter().map(to_toml_edit_value).collect::<Vec<_>>();
            toml_edit::Value::Array(toml_edit::Array::from_iter(values))
        }
        toml::Value::Table(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (k, v) in map {
                table.insert(k, to_toml_edit_value(v));
            }
            toml_edit::Value::InlineTable(table)
        }
    }
}

/// Generic helper to write value(s) to the in-memory config and disk.
///
/// # Arguments
/// * `chain`: The chain to write the configuration for
/// * `key`: The variable key to write in the fork configuration
/// * `value`: The TOML value to write (already converted from solidity types)
/// * `state`: The cheatcodes state containing the fork configurations
///
/// # Returns
/// Returns ABI-encoded tuple of (success: bool, overwrote: bool)
fn write_toml_value(
    chain: Chain,
    key: &str,
    value: toml::Value,
    state: &mut crate::Cheatcodes,
) -> Result {
    if matches!(key, "access" | "rpc_endpoint") {
        bail!("'{key}' cannot be modified with cheatcodes");
    }

    if !state.config.forks.read().unwrap().access.can_write() {
        return Ok((false, false).abi_encode());
    }

    // Attempt to update the config file.
    let writer = ForkConfigWriter::new(&state.config.root, get_chain_name(&chain), key, &value);
    let overwritten = match writer.persist() {
        Ok(overwritten) => overwritten,
        Err(e) => {
            warn!("warning: failed to write '{key}' to disk: {e}");
            return Ok((false, false).abi_encode());
        }
    };

    // Update the in-memory state.
    let mut forks = state.config.forks.write().unwrap();
    let fork_chain_config = forks.chain_configs.entry(chain).or_default();
    fork_chain_config.vars.insert(key.to_string(), value);

    Ok((true, overwritten).abi_encode())
}

/// Helper to safely traverse a TOML document to find a specific fork chain's section.
fn get_toml_section<'a>(doc: &'a toml_edit::Item, chain_name: &str) -> Option<&'a toml_edit::Item> {
    doc.get("forks")?.get(chain_name)
}

/// Gets the chain id of the active fork. Bails if no fork is selected.
fn get_active_fork_chain_id(ccx: &mut CheatsCtxt) -> Result<u64> {
    let (db, _, env) = ccx.as_db_env_and_journal();
    if !db.is_forked_mode() {
        bail!("a fork must be selected");
    }
    Ok(env.cfg.chain_id)
}

/// Gets the alloy chain for the active fork. Bails if no fork is selected.
fn get_active_fork_chain(ccx: &mut CheatsCtxt) -> Result<Chain> {
    get_active_fork_chain_id(ccx).map(Chain::from_id)
}
