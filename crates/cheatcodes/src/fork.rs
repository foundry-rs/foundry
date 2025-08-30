//! Implementations of [`Forking`](spec::Group::Forking) cheatcodes.
use crate::{
    Cheatcode, CheatsCtxt, DatabaseExt, Result, Vm::*, json::parse_json_as,
    toml::toml_to_json_value,
};
use alloy_chains::Chain;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_sol_types::SolValue;
use foundry_evm_core::ContextExt;
use std::borrow::Cow;

impl Cheatcode for readForkChainIdsCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self {} = self;
        Ok(state.config.forks.keys().map(|chain| chain.id()).collect::<Vec<_>>().abi_encode())
    }
}

impl Cheatcode for readForkChainsCall {
    fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
        let Self {} = self;
        Ok(state
            .config
            .forks
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
    if let Some(config) = state.config.forks.get(&chain) {
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

/// Gets the chain id of the active fork. Panics if no fork is selected.
fn get_active_fork_chain_id(ccx: &mut CheatsCtxt) -> Result<u64> {
    let (db, _, env) = ccx.as_db_env_and_journal();
    if !db.is_forked_mode() {
        bail!("a fork must be selected");
    }
    Ok(env.cfg.chain_id)
}

/// Gets the alloy chain for the active fork. Panics if no fork is selected.
fn get_active_fork_chain(ccx: &mut CheatsCtxt) -> Result<Chain> {
    get_active_fork_chain_id(ccx).map(Chain::from_id)
}

// Helper macros to generate cheatcode implementations
macro_rules! impl_get_value_cheatcode {
    ($struct:ident, $sol_type:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let Self { key } = self;
                let chain = get_active_fork_chain(ccx)?;
                get_value(chain, key, $sol_type, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                let Self { chain, key } = self;
                get_value(Chain::from_id(chain.to::<u64>()), key, $sol_type, state)
            }
        }
    };
}

macro_rules! impl_get_array_cheatcode {
    ($struct:ident, $sol_type:expr,stateful) => {
        impl Cheatcode for $struct {
            fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
                let Self { key } = self;
                get_array(get_active_fork_chain(ccx)?, key, $sol_type, ccx.state)
            }
        }
    };
    ($struct:ident, $sol_type:expr) => {
        impl Cheatcode for $struct {
            fn apply(&self, state: &mut crate::Cheatcodes) -> Result {
                let Self { chain, key } = self;
                get_array(Chain::from(chain.to::<u64>()), key, $sol_type, state)
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

/// Generic helper to get any value from the TOML config.
fn get_toml_value<'a>(
    chain: Chain,
    key: &'a str,
    state: &'a crate::Cheatcodes,
) -> Result<&'a toml::Value> {
    let config = state.config.forks.get(&chain).ok_or_else(|| {
        fmt_err!(
            "'[fork.<chain_id: {chain}>]' subsection not found in 'foundry.toml'",
            chain = chain.id()
        )
    })?;
    let value = config.vars.get(key).ok_or_else(|| {
        fmt_err!("variable '{key}' not found in '[fork.<chain_id: {chain}>]'", chain = chain.id())
    })?;

    Ok(value)
}

/// Generic helper to get any single value from the TOML config.
fn get_value(chain: Chain, key: &str, ty: &DynSolType, state: &crate::Cheatcodes) -> Result {
    let value = get_toml_value(chain, key, state)?;
    let sol_value = parse_toml_element(value, ty, key, chain)?;
    Ok(sol_value.abi_encode())
}

/// Generic helper to get an array of values from the TOML config.
fn get_array(
    chain: Chain,
    key: &str,
    element_ty: &DynSolType,
    state: &crate::Cheatcodes,
) -> Result {
    let arr = get_toml_value(chain, key, state)?.as_array().ok_or_else(|| {
        fmt_err!(
            "variable '{key}' in '[fork.<chain_id: {chain}>]' must be an array",
            chain = chain.id()
        )
    })?;
    let result: Result<Vec<_>> = arr
        .iter()
        .enumerate()
        .map(|(i, elem)| {
            let context = format!("{key}[{i}]");
            parse_toml_element(elem, element_ty, &context, chain)
        })
        .collect();

    Ok(DynSolValue::Array(result?).abi_encode())
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
