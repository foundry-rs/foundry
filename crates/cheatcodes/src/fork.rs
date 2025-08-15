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

fn cast_string(key: &str, val: &str, ty: &DynSolType) -> Result {
    string::parse(val, ty).map_err(map_env_err(key, val))
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

impl Cheatcode for forkChainBoolCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_bool(chain.to::<u64>(), key, state)
    }
}

impl Cheatcode for forkBoolCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        get_bool(get_active_fork_chain_id(ccx)?, key, ccx.state)
    }
}

impl Cheatcode for forkBoolArrayCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        get_array(get_active_fork_chain_id(ccx)?, key, &DynSolType::Bool, ccx.state)
    }
}

impl Cheatcode for forkChainBoolArrayCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_array(chain.to::<u64>(), key, &DynSolType::Bool, state)
    }
}

impl Cheatcode for forkChainIntCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_int256(chain.to::<u64>(), key, state)
    }
}

impl Cheatcode for forkIntCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        get_int256(get_active_fork_chain_id(ccx)?, key, ccx.state)
    }
}

impl Cheatcode for forkIntArrayCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        get_array(get_active_fork_chain_id(ccx)?, key, &DynSolType::Int(256), ccx.state)
    }
}

impl Cheatcode for forkChainIntArrayCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_array(chain.to::<u64>(), key, &DynSolType::Int(256), state)
    }
}

impl Cheatcode for forkChainUintCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_uint256(chain.to::<u64>(), key, state)
    }
}

impl Cheatcode for forkUintCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        get_uint256(get_active_fork_chain_id(ccx)?, key, ccx.state)
    }
}

impl Cheatcode for forkUintArrayCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        get_array(get_active_fork_chain_id(ccx)?, key, &DynSolType::Uint(256), ccx.state)
    }
}

impl Cheatcode for forkChainUintArrayCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_array(chain.to::<u64>(), key, &DynSolType::Uint(256), state)
    }
}

impl Cheatcode for forkChainAddressCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_type_from_str_input(chain.to::<u64>(), key, &DynSolType::Address, state)
    }
}

impl Cheatcode for forkAddressCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        let chain = get_active_fork_chain_id(ccx)?;
        get_type_from_str_input(chain, key, &DynSolType::Address, ccx.state)
    }
}

impl Cheatcode for forkAddressArrayCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        get_array(get_active_fork_chain_id(ccx)?, key, &DynSolType::Address, ccx.state)
    }
}

impl Cheatcode for forkChainAddressArrayCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_array(chain.to::<u64>(), key, &DynSolType::Address, state)
    }
}

impl Cheatcode for forkChainBytes32Call {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_type_from_str_input(chain.to::<u64>(), key, &DynSolType::FixedBytes(32), state)
    }
}

impl Cheatcode for forkBytes32Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        let chain = get_active_fork_chain_id(ccx)?;
        get_type_from_str_input(chain, key, &DynSolType::FixedBytes(32), ccx.state)
    }
}

impl Cheatcode for forkBytes32ArrayCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        get_array(get_active_fork_chain_id(ccx)?, key, &DynSolType::FixedBytes(32), ccx.state)
    }
}

impl Cheatcode for forkChainBytes32ArrayCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_array(chain.to::<u64>(), key, &DynSolType::FixedBytes(32), state)
    }
}

impl Cheatcode for forkChainBytesCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_type_from_str_input(chain.to::<u64>(), key, &DynSolType::Bytes, state)
    }
}

impl Cheatcode for forkBytesCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        let chain = get_active_fork_chain_id(ccx)?;
        get_type_from_str_input(chain, key, &DynSolType::Bytes, ccx.state)
    }
}

impl Cheatcode for forkBytesArrayCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        get_array(get_active_fork_chain_id(ccx)?, key, &DynSolType::Bytes, ccx.state)
    }
}

impl Cheatcode for forkChainBytesArrayCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_array(chain.to::<u64>(), key, &DynSolType::Bytes, state)
    }
}

impl Cheatcode for forkChainStringCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_type_from_str_input(chain.to::<u64>(), key, &DynSolType::String, state)
    }
}

impl Cheatcode for forkStringCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        let chain = get_active_fork_chain_id(ccx)?;
        get_type_from_str_input(chain, key, &DynSolType::String, ccx.state)
    }
}

impl Cheatcode for forkStringArrayCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { key } = self;
        get_array(get_active_fork_chain_id(ccx)?, key, &DynSolType::String, ccx.state)
    }
}

impl Cheatcode for forkChainStringArrayCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self { chain, key } = self;
        get_array(chain.to::<u64>(), key, &DynSolType::String, state)
    }
}

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

fn get_bool(chain: u64, key: &str, state: &crate::Cheatcodes) -> Result {
    let name = get_chain_name(chain)?;
    let value = get_toml_value(name, key, state)?;

    if let Some(b) = value.as_bool() {
        Ok(b.abi_encode())
    } else if let Some(v) = value.as_integer() {
        Ok((v == 0).abi_encode())
    } else if let Some(s) = value.as_str() {
        cast_string(key, s, &DynSolType::Bool)
    } else {
        bail!("Variable '{key}' in [fork.{name}] must be a boolean or a string");
    }
}

fn get_int256(chain: u64, key: &str, state: &crate::Cheatcodes) -> Result {
    let name = get_chain_name(chain)?;
    let value = get_toml_value(name, key, state)?;
    if let Some(int_value) = value.as_integer() {
        Ok(int_value.abi_encode())
    } else if let Some(s) = value.as_str() {
        cast_string(key, s, &DynSolType::Int(256))
    } else {
        bail!("Variable '{key}' in [fork.{name}] must be an integer or a string");
    }
}

fn get_uint256(chain: u64, key: &str, state: &crate::Cheatcodes) -> Result {
    let name = get_chain_name(chain)?;
    let value = get_toml_value(name, key, state)?;

    if let Some(int_value) = value.as_integer() {
        if int_value >= 0 {
            Ok((int_value as u64).abi_encode())
        } else {
            bail!("Variable '{key}' in [fork.{name}] is a negative integer");
        }
    } else if let Some(s) = value.as_str() {
        cast_string(key, s, &DynSolType::Uint(256))
    } else {
        bail!("Variable '{key}' in [fork.{name}] must be an integer or a string");
    }
}

fn get_type_from_str_input(
    chain: u64,
    key: &str,
    ty: &DynSolType,
    state: &crate::Cheatcodes,
) -> Result {
    let name = get_chain_name(chain)?;
    let value = get_toml_value(name, key, state)?;

    if let Some(val) = value.as_str() {
        cast_string(key, val, ty)
    } else {
        bail!("Variable '{key}' in [fork.{name}] must be a string");
    }
}

fn get_array(chain: u64, key: &str, element_ty: &DynSolType, state: &crate::Cheatcodes) -> Result {
    let name = get_chain_name(chain)?;
    let value = get_toml_value(name, key, state)?;

    if let Some(arr) = value.as_array() {
        let mut result = Vec::new();
        for (i, elem) in arr.iter().enumerate() {
            let parsed = match element_ty {
                DynSolType::Bool => {
                    if let Some(b) = elem.as_bool() {
                        DynSolValue::Bool(b)
                    } else if let Some(v) = elem.as_integer() {
                        DynSolValue::Bool(v != 0)
                    } else if let Some(s) = elem.as_str() {
                        string::parse_value(s, element_ty)
                            .map_err(map_env_err(&format!("{key}[{i}]"), s))?
                    } else {
                        bail!(
                            "Element {i} of '{key}' in [fork.{name}] must be a boolean or a string"
                        );
                    }
                }
                DynSolType::Int(256) => {
                    if let Some(int_value) = elem.as_integer() {
                        DynSolValue::Int(alloy_primitives::I256::try_from(int_value).unwrap(), 256)
                    } else if let Some(s) = elem.as_str() {
                        string::parse_value(s, element_ty)
                            .map_err(map_env_err(&format!("{key}[{i}]"), s))?
                    } else {
                        bail!(
                            "Element {i} of '{key}' in [fork.{name}] must be an integer or a string"
                        );
                    }
                }
                DynSolType::Uint(256) => {
                    if let Some(int_value) = elem.as_integer() {
                        if int_value >= 0 {
                            DynSolValue::Uint(alloy_primitives::U256::from(int_value as u64), 256)
                        } else {
                            bail!("Element {i} of '{key}' in [fork.{name}] is a negative integer");
                        }
                    } else if let Some(s) = elem.as_str() {
                        string::parse_value(s, element_ty)
                            .map_err(map_env_err(&format!("{key}[{i}]"), s))?
                    } else {
                        bail!(
                            "Element {i} of '{key}' in [fork.{name}] must be an integer or a string"
                        );
                    }
                }
                DynSolType::Address
                | DynSolType::FixedBytes(32)
                | DynSolType::String
                | DynSolType::Bytes => {
                    if let Some(s) = elem.as_str() {
                        string::parse_value(s, element_ty)
                            .map_err(map_env_err(&format!("{key}[{i}]"), s))?
                    } else {
                        bail!("Element {i} of '{key}' in [fork.{name}] must be a string");
                    }
                }
                _ => bail!("Unsupported array element type for fork configuration"),
            };
            result.push(parsed);
        }

        Ok(DynSolValue::Array(result).abi_encode())
    } else {
        bail!("Variable '{key}' in [fork.{name}] must be an array");
    }
}
