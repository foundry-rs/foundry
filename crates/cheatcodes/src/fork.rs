//! Implementations of [`Forking`](spec::Group::Forking) cheatcodes.

use crate::{
    Cheatcode, CheatsCtxt, DatabaseExt, Result, Vm::*, json::parse_json_as,
    toml::toml_to_json_value,
};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_sol_types::SolValue;
use eyre::OptionExt;
use foundry_evm_core::ContextExt;

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
                let chain = get_active_fork_chain_id(ccx)?;
                get_and_encode_toml_value(chain, &self.key, $sol_type, false, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                get_and_encode_toml_value(
                    self.chain.to::<u64>(),
                    &self.key,
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
                let chain = get_active_fork_chain_id(ccx)?;
                get_and_encode_toml_value(chain, &self.key, $sol_type, true, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                get_and_encode_toml_value(self.chain.to::<u64>(), &self.key, $sol_type, true, state)
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
    |v: alloy_primitives::Address| toml::Value::String(format!("0x{v}")),
    stateful
);
impl_write_value_cheatcode!(
    writeForkChainVar_6Call,
    &DynSolType::Address,
    |v: alloy_primitives::Address| toml::Value::String(format!("0x{v}"))
);
impl_write_array_cheatcode!(
    writeForkVar_7Call,
    &DynSolType::Address,
    |v: alloy_primitives::Address| toml::Value::String(format!("0x{v}")),
    stateful
);
impl_write_array_cheatcode!(
    writeForkChainVar_7Call,
    &DynSolType::Address,
    |v: alloy_primitives::Address| toml::Value::String(format!("0x{v}"))
);

// Bytes32
impl_write_value_cheatcode!(
    writeForkVar_8Call,
    &DynSolType::FixedBytes(32),
    |v: alloy_primitives::FixedBytes<32>| toml::Value::String(format!("0x{v}")),
    stateful
);
impl_write_value_cheatcode!(
    writeForkChainVar_8Call,
    &DynSolType::FixedBytes(32),
    |v: alloy_primitives::FixedBytes<32>| toml::Value::String(format!("0x{v}"))
);
impl_write_array_cheatcode!(
    writeForkVar_9Call,
    &DynSolType::FixedBytes(32),
    |v: alloy_primitives::FixedBytes<32>| toml::Value::String(format!("0x{v}")),
    stateful
);
impl_write_array_cheatcode!(
    writeForkChainVar_9Call,
    &DynSolType::FixedBytes(32),
    |v: alloy_primitives::FixedBytes<32>| toml::Value::String(format!("0x{v}"))
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
    |v: alloy_primitives::Bytes| toml::Value::String(format!("0x{v}")),
    stateful
);
impl_write_value_cheatcode!(
    writeForkChainVar_12Call,
    &DynSolType::Bytes,
    |v: alloy_primitives::Bytes| toml::Value::String(format!("0x{v}"))
);
impl_write_array_cheatcode!(
    writeForkVar_13Call,
    &DynSolType::Bytes,
    |v: alloy_primitives::Bytes| toml::Value::String(format!("0x{v}")),
    stateful
);
impl_write_array_cheatcode!(
    writeForkChainVar_13Call,
    &DynSolType::Bytes,
    |v: alloy_primitives::Bytes| toml::Value::String(format!("0x{v}"))
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
    context: &str,
    fork_name: &str,
) -> Result<DynSolValue> {
    // Convert TOML value to JSON value and use existing JSON parsing logic
    parse_json_as(&toml_to_json_value(elem.to_owned()), element_ty)
        .map_err(|e| fmt_err!("Failed to parse '{context}' in [fork.{fork_name}]: {e}"))
}

/// Generic helper to write value(s) to the in-memory config.
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
    let name = get_chain_name(chain)?;

    // Acquire write lock
    let mut forks =
        state.config.forks.write().map_err(|_| fmt_err!("Failed to acquire write lock"))?;

    // Check if fork config exists, create if not
    let config = forks.chain_configs.entry(name.to_string()).or_default();

    // Check if key already exists (for overwrote flag)
    let overwrote = config.vars.contains_key(key);

    // Insert the value
    config.vars.insert(key.to_string(), value);

    // Return (success=true, overwrote)
    Ok((true, overwrote).abi_encode())
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
