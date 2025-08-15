//! Implementations of [`Forking`](spec::Group::Forking) cheatcodes.

use crate::{Cheatcode, CheatsCtxt, DatabaseExt, Error, Result, Vm::*, string};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_sol_types::SolValue;
use eyre::OptionExt;
use foundry_evm_core::ContextExt;

impl Cheatcode for forkChainIdsCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self {} = self;
        Ok(state
            .config
            .forks
            .keys()
            .map(|name| alloy_chains::Chain::from_named(name.parse().unwrap()).id())
            .collect::<Vec<_>>()
            .abi_encode())
    }
}

impl Cheatcode for forkChainsCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self {} = self;
        Ok(state.config.forks.keys().collect::<Vec<_>>().abi_encode())
    }
}

impl Cheatcode for forkChainCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        Ok(get_active_fork_chain_name(ccx)?.abi_encode())
    }
}

impl Cheatcode for forkChainIdCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        Ok(get_active_fork_chain_id(ccx)?.abi_encode())
    }
}

fn resolve_rpc_url(name: &'static str, state: &mut crate::Cheatcodes) -> Result {
    // Get the chain ID from the chain_configs
    if let Some(config) = state.config.forks.get(name) {
        let rpc = match config.rpc_endpoint {
            Some(ref url) => url.clone().resolve(),
            None => state.config.rpc_endpoint(name)?,
        };

        return Ok(rpc.url()?.abi_encode());
    }

    bail!("[fork.{name}] subsection not found in [fork] of 'foundry.toml'")
}

impl Cheatcode for forkChainRpcUrlCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { id } = self;
        let name = get_chain_name(id.to::<u64>())?;
        resolve_rpc_url(name, state)
    }
}

impl Cheatcode for forkRpcUrlCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let name = get_active_fork_chain_name(ccx)?;
        resolve_rpc_url(name, ccx.state)
    }
}

/// Converts the error message of a failed parsing attempt to a more user-friendly message that
/// doesn't leak the value.
fn map_env_err<'a>(key: &'a str, value: &'a str) -> impl FnOnce(Error) -> Error + 'a {
    move |e| {
        // failed parsing <value> as type `uint256`: parser error:
        // <value>
        //   ^
        //   expected at least one digit
        let mut e = e.to_string();
        e = e.replacen(&format!("\"{value}\""), &format!("${key}"), 1);
        e = e.replacen(&format!("\n{value}\n"), &format!("\n${key}\n"), 1);
        Error::from(e)
    }
}

/// Gets the alloy chain name for a given chain id.
fn get_chain_name(id: u64) -> Result<&'static str> {
    let chain = alloy_chains::Chain::from_id(id)
        .named()
        .ok_or_eyre("unknown name for active forked chain")?;

    Ok(chain.as_str())
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

// Helper macros to generate cheatcode implementations
macro_rules! impl_get_value_cheatcode {
    ($struct:ident, $sol_type:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let chain = get_active_fork_chain_id(ccx)?;
                get_value(chain, &self.key, $sol_type, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                get_value(self.chain.to::<u64>(), &self.key, $sol_type, state)
            }
        }
    };
}

macro_rules! impl_get_array_cheatcode {
    ($struct:ident, $sol_type:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let chain = get_active_fork_chain_id(ccx)?;
                get_array(chain, &self.key, $sol_type, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                get_array(self.chain.to::<u64>(), &self.key, $sol_type, state)
            }
        }
    };
}

// Bool
impl_get_value_cheatcode!(forkChainBoolCall, &DynSolType::Bool);
impl_get_value_cheatcode!(forkBoolCall, &DynSolType::Bool, stateful);
impl_get_array_cheatcode!(forkChainBoolArrayCall, &DynSolType::Bool);
impl_get_array_cheatcode!(forkBoolArrayCall, &DynSolType::Bool, stateful);

// Int
impl_get_value_cheatcode!(forkChainIntCall, &DynSolType::Int(256));
impl_get_value_cheatcode!(forkIntCall, &DynSolType::Int(256), stateful);
impl_get_array_cheatcode!(forkChainIntArrayCall, &DynSolType::Int(256));
impl_get_array_cheatcode!(forkIntArrayCall, &DynSolType::Int(256), stateful);

// Uint
impl_get_value_cheatcode!(forkChainUintCall, &DynSolType::Uint(256));
impl_get_value_cheatcode!(forkUintCall, &DynSolType::Uint(256), stateful);
impl_get_array_cheatcode!(forkChainUintArrayCall, &DynSolType::Uint(256));
impl_get_array_cheatcode!(forkUintArrayCall, &DynSolType::Uint(256), stateful);

// Address
impl_get_value_cheatcode!(forkChainAddressCall, &DynSolType::Address);
impl_get_value_cheatcode!(forkAddressCall, &DynSolType::Address, stateful);
impl_get_array_cheatcode!(forkChainAddressArrayCall, &DynSolType::Address);
impl_get_array_cheatcode!(forkAddressArrayCall, &DynSolType::Address, stateful);

// Bytes32
impl_get_value_cheatcode!(forkChainBytes32Call, &DynSolType::FixedBytes(32));
impl_get_value_cheatcode!(forkBytes32Call, &DynSolType::FixedBytes(32), stateful);
impl_get_array_cheatcode!(forkChainBytes32ArrayCall, &DynSolType::FixedBytes(32));
impl_get_array_cheatcode!(forkBytes32ArrayCall, &DynSolType::FixedBytes(32), stateful);

// Bytes
impl_get_value_cheatcode!(forkChainBytesCall, &DynSolType::Bytes);
impl_get_value_cheatcode!(forkBytesCall, &DynSolType::Bytes, stateful);
impl_get_array_cheatcode!(forkChainBytesArrayCall, &DynSolType::Bytes);
impl_get_array_cheatcode!(forkBytesArrayCall, &DynSolType::Bytes, stateful);

// String
impl_get_value_cheatcode!(forkChainStringCall, &DynSolType::String);
impl_get_value_cheatcode!(forkStringCall, &DynSolType::String, stateful);
impl_get_array_cheatcode!(forkChainStringArrayCall, &DynSolType::String);
impl_get_array_cheatcode!(forkStringArrayCall, &DynSolType::String, stateful);

fn get_toml_value<'a>(
    name: &'a str,
    key: &'a str,
    state: &'a crate::Cheatcodes,
) -> Result<&'a toml::Value> {
    let config = state.config.forks.get(name).ok_or_else(|| {
        fmt_err!("[fork.{name}] subsection not found in [fork] of 'foundry.toml'")
    })?;
    let value = config
        .vars
        .get(key)
        .ok_or_else(|| fmt_err!("Variable '{key}' not found in [fork.{name}] configuration"))?;

    Ok(value)
}

/// Generic helper to get any single value from the TOML config.
/// This replaces get_bool, get_int256, get_uint256, and get_type_from_str_input.
fn get_value(chain: u64, key: &str, ty: &DynSolType, state: &crate::Cheatcodes) -> Result {
    let name = get_chain_name(chain)?;
    let value = get_toml_value(name, key, state)?;
    let sol_value = parse_toml_element(value, ty, key, name)?;
    Ok(sol_value.abi_encode())
}

/// Generic helper to get an array of values from the TOML config.
fn get_array(chain: u64, key: &str, element_ty: &DynSolType, state: &crate::Cheatcodes) -> Result {
    let name = get_chain_name(chain)?;
    let value = get_toml_value(name, key, state)?;

    let arr = value
        .as_array()
        .ok_or_else(|| fmt_err!("Variable '{key}' in [fork.{name}] must be an array"))?;

    let result: Result<Vec<_>> = arr
        .iter()
        .enumerate()
        .map(|(i, elem)| {
            let context = format!("{key}[{i}]");
            parse_toml_element(elem, element_ty, &context, name)
        })
        .collect();

    Ok(DynSolValue::Array(result?).abi_encode())
}

/// Parses a single TOML value into a specific Solidity type.
fn parse_toml_element<'a>(
    elem: &'a toml::Value,
    element_ty: &DynSolType,
    context: &str,
    fork_name: &str,
) -> Result<DynSolValue> {
    match element_ty {
        DynSolType::Bool => {
            if let Some(b) = elem.as_bool() {
                Ok(DynSolValue::Bool(b))
            } else if let Some(v) = elem.as_integer() {
                Ok(DynSolValue::Bool(v != 0))
            } else if let Some(s) = elem.as_str() {
                string::parse_value(s, element_ty).map_err(map_env_err(context, s))
            } else {
                bail!(
                    "Element '{context}' in [fork.{fork_name}] must be a boolean, integer, or a string"
                )
            }
        }
        DynSolType::Int(256) => {
            if let Some(int_value) = elem.as_integer() {
                Ok(DynSolValue::Int(alloy_primitives::I256::try_from(int_value).unwrap(), 256))
            } else if let Some(s) = elem.as_str() {
                string::parse_value(s, element_ty).map_err(map_env_err(context, s))
            } else {
                bail!("Element '{context}' in [fork.{fork_name}] must be an integer or a string")
            }
        }
        DynSolType::Uint(256) => {
            if let Some(int_value) = elem.as_integer() {
                if int_value < 0 {
                    bail!(
                        "Element '{context}' in [fork.{fork_name}] is a negative integer but expected an unsigned integer"
                    );
                }
                Ok(DynSolValue::Uint(alloy_primitives::U256::from(int_value as u64), 256))
            } else if let Some(s) = elem.as_str() {
                string::parse_value(s, element_ty).map_err(map_env_err(context, s))
            } else {
                bail!("Element '{context}' in [fork.{fork_name}] must be an integer or a string")
            }
        }
        DynSolType::Address
        | DynSolType::FixedBytes(32)
        | DynSolType::String
        | DynSolType::Bytes => {
            if let Some(s) = elem.as_str() {
                string::parse_value(s, element_ty).map_err(map_env_err(context, s))
            } else {
                bail!("Element '{context}' in [fork.{fork_name}] must be a string");
            }
        }
        _ => bail!("Unsupported array element type for fork configuration: {element_ty:?}"),
    }
}
